use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rb::pki::{ImplantCredentials, IssuedOperator, PkiAuthority};
use rb::store::Store;
use rcgen::KeyPair;
use rustls::pki_types::{CertificateRevocationListDer, PrivatePkcs8KeyDer};
use rustls::server::{ServerConfig, WebPkiClientVerifier};
use rustls::RootCertStore;

/// The CA plus the server and shared client certificates it has signed.
///
/// The CA key is persisted to disk so operator certificates stay valid across restarts. The server
/// and client certificates are regenerated each start (they chain to the same CA, so clients that
/// trust the CA keep working).
pub struct TestPki {
    // Operator channel
    pub roots: Arc<RootCertStore>,
    pub ca_cert: rcgen::Certificate,
    pub ca_key: KeyPair,
    pub client_cert: rcgen::CertifiedKey,
    pub server_cert: rcgen::CertifiedKey,
    // Implant channel
    pub implant_roots: Arc<RootCertStore>,
    pub implant_ca_cert: rcgen::Certificate,
    pub implant_ca_key: KeyPair,
    pub implant_cert: rcgen::CertifiedKey,
    pub listener_server_cert: rcgen::CertifiedKey,
}

impl TestPki {
    /// Load the CA key from `ca_key_path` if present, otherwise generate one and persist it (and
    /// its certificate) to `ca_cert_path` / `ca_key_path`.
    pub fn load_or_create(
        ca_cert_path: &str,
        ca_key_path: &str,
        implant_ca_cert_path: &str,
        implant_ca_key_path: &str,
    ) -> Self {
        let (ca_cert, ca_key) = match Self::load_ca(ca_key_path, "RustBucket CA") {
            Some(pair) => pair,
            None => {
                let pair = Self::generate_ca("RustBucket CA");
                Self::persist_ca(ca_cert_path, ca_key_path, &pair);
                pair
            }
        };
        let (implant_ca_cert, implant_ca_key) =
            match Self::load_ca(implant_ca_key_path, "RustBucket Implant CA") {
                Some(pair) => pair,
                None => {
                    let pair = Self::generate_ca("RustBucket Implant CA");
                    Self::persist_ca(implant_ca_cert_path, implant_ca_key_path, &pair);
                    pair
                }
            };
        Self::from_cas(ca_cert, ca_key, implant_ca_cert, implant_ca_key)
    }

    fn ca_params_template(common_name: &str) -> rcgen::CertificateParams {
        let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::OrganizationName, "RustBucket C2");
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, common_name);
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        ca_params
    }

    fn load_ca(ca_key_path: &str, common_name: &str) -> Option<(rcgen::Certificate, KeyPair)> {
        let key_pem = std::fs::read_to_string(ca_key_path).ok()?;
        let ca_key = KeyPair::from_pem(&key_pem).ok()?;
        let ca_cert = Self::ca_params_template(common_name).self_signed(&ca_key).ok()?;
        Some((ca_cert, ca_key))
    }

    fn generate_ca(common_name: &str) -> (rcgen::Certificate, KeyPair) {
        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let ca_cert = Self::ca_params_template(common_name).self_signed(&ca_key).unwrap();
        (ca_cert, ca_key)
    }

    fn persist_ca(ca_cert_path: &str, ca_key_path: &str, pair: &(rcgen::Certificate, KeyPair)) {
        let (ca_cert, ca_key) = pair;
        ensure_parent(ca_cert_path);
        ensure_parent(ca_key_path);
        std::fs::write(ca_cert_path, ca_cert.pem()).unwrap();
        std::fs::write(ca_key_path, ca_key.serialize_pem()).unwrap();
    }

    fn issue_ee(
        ca_cert: &rcgen::Certificate,
        ca_key: &KeyPair,
        common_name: &str,
        sans: Vec<String>,
        eku: rcgen::ExtendedKeyUsagePurpose,
        serial: Option<rcgen::SerialNumber>,
    ) -> rcgen::CertifiedKey {
        let mut params = rcgen::CertificateParams::new(sans).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, common_name);
        params.is_ca = rcgen::IsCa::NoCa;
        params.extended_key_usages = vec![eku];
        params.serial_number = serial;
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = params.signed_by(&key, ca_cert, ca_key).unwrap();
        rcgen::CertifiedKey {
            cert,
            key_pair: key,
        }
    }

    fn from_cas(
        ca_cert: rcgen::Certificate,
        ca_key: KeyPair,
        implant_ca_cert: rcgen::Certificate,
        implant_ca_key: KeyPair,
    ) -> Self {
        let server_cert = Self::issue_ee(
            &ca_cert,
            &ca_key,
            "localhost",
            vec!["localhost".to_string(), "127.0.0.1".to_string()],
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            None,
        );
        let client_cert = Self::issue_ee(
            &ca_cert,
            &ca_key,
            "RustBucket Client",
            Vec::new(),
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
            Some(rcgen::SerialNumber::from(vec![0xC0, 0xFF, 0xEE])),
        );
        let listener_server_cert = Self::issue_ee(
            &implant_ca_cert,
            &implant_ca_key,
            "localhost",
            vec!["localhost".to_string(), "127.0.0.1".to_string()],
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            None,
        );
        let implant_cert = Self::issue_ee(
            &implant_ca_cert,
            &implant_ca_key,
            "RustBucket Implant",
            Vec::new(),
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
            Some(rcgen::SerialNumber::from(
                rand::random::<u64>().to_be_bytes().to_vec(),
            )),
        );

        let mut roots = RootCertStore::empty();
        roots.add(ca_cert.der().clone()).unwrap();
        let mut implant_roots = RootCertStore::empty();
        implant_roots.add(implant_ca_cert.der().clone()).unwrap();

        Self {
            roots: Arc::new(roots),
            ca_cert,
            ca_key,
            client_cert,
            server_cert,
            implant_roots: Arc::new(implant_roots),
            implant_ca_cert,
            implant_ca_key,
            implant_cert,
            listener_server_cert,
        }
    }

    /// Generate a server configuration for the client using the test PKI.
    ///
    /// Importantly this creates a new client certificate verifier per-connection so that the server
    /// can read in the latest CRL content from disk.
    pub fn server_config(&self, crl_path: &str) -> Arc<ServerConfig> {
        // Read the latest CRL from disk
        let mut crl_file = File::open(crl_path).unwrap();
        let mut crl = Vec::default();
        crl_file.read_to_end(&mut crl).unwrap();

        // Construct a fresh verifier using the test PKI roots, and the updated CRL.
        let verifier = WebPkiClientVerifier::builder(self.roots.clone())
            .with_crls([CertificateRevocationListDer::from(crl)])
            .build()
            .unwrap();

        let mut server_config = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![self.server_cert.cert.der().clone()],
                PrivatePkcs8KeyDer::from(self.server_cert.key_pair.serialize_der()).into(),
            )
            .unwrap();

        // Allow using SSLKEYLOGFILE.
        server_config.key_log = Arc::new(rustls::KeyLogFile::new());

        Arc::new(server_config)
    }

    /// Server config for the implant-facing listener. Client certs must chain to the implant CA.
    pub fn listener_server_config(&self) -> Arc<ServerConfig> {
        let verifier = WebPkiClientVerifier::builder(self.implant_roots.clone())
            .build()
            .unwrap();

        let mut server_config = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![self.listener_server_cert.cert.der().clone()],
                PrivatePkcs8KeyDer::from(self.listener_server_cert.key_pair.serialize_der()).into(),
            )
            .unwrap();

        server_config.key_log = Arc::new(rustls::KeyLogFile::new());
        Arc::new(server_config)
    }

    /// Write the implant CA certificate and the shared implant client cert/key to disk.
    pub fn write_implant_artifacts(&self, ca_path: &str, cert_path: &str, key_path: &str) {
        let write_pem = |path: &str, pem: &str| {
            ensure_parent(path);
            let mut file = File::create(path).unwrap();
            file.write_all(pem.as_bytes()).unwrap();
        };

        write_pem(ca_path, &self.implant_ca_cert.pem());
        write_pem(cert_path, &self.implant_cert.cert.pem());
        write_pem(key_path, &self.implant_cert.key_pair.serialize_pem());
    }

    /// Issue a certificate revocation list (CRL) for the revoked `serials` provided (may be empty).
    /// The CRL will be signed by the test PKI CA and returned in DER serialized form.
    pub fn crl(
        &self,
        serials: Vec<rcgen::SerialNumber>,
        next_update_seconds: u64,
    ) -> CertificateRevocationListDer<'static> {
        // webpki rejects a CRL that is outside its validity window, so use the current time and
        // give it a comfortable window (at least an hour) rather than just the refresh interval.
        let now = time::OffsetDateTime::now_utc();
        let valid_for = (next_update_seconds as i64).max(3600);
        let next_update = now + time::Duration::seconds(valid_for);

        // For each serial, create a revoked certificate entry.
        let revoked_certs = serials
            .into_iter()
            .map(|serial| rcgen::RevokedCertParams {
                serial_number: serial,
                revocation_time: now,
                reason_code: Some(rcgen::RevocationReason::KeyCompromise),
                invalidity_date: None,
            })
            .collect();

        // Create a new CRL signed by the CA cert.
        let crl_params = rcgen::CertificateRevocationListParams {
            this_update: now,
            next_update,
            crl_number: rcgen::SerialNumber::from(1234u64),
            issuing_distribution_point: None,
            revoked_certs,
            key_identifier_method: rcgen::KeyIdMethod::Sha256,
        };
        crl_params
            .signed_by(&self.ca_cert, &self.ca_key)
            .unwrap()
            .into()
    }

    /// Write the shared client certificate/key and an initial empty CRL to disk.
    pub fn write_client_artifacts(
        &self,
        client_cert_path: &str,
        client_key_path: &str,
        crl_path: &str,
        crl_update_seconds: u64,
    ) {
        let write_pem = |path: &str, pem: &str| {
            ensure_parent(path);
            let mut file = File::create(path).unwrap();
            file.write_all(pem.as_bytes()).unwrap();
        };

        write_pem(client_cert_path, &self.client_cert.cert.pem());
        write_pem(client_key_path, &self.client_cert.key_pair.serialize_pem());

        // Write out an initial DER CRL that has no revoked certificates.
        ensure_parent(crl_path);
        let mut crl_der = File::create(crl_path).unwrap();
        crl_der
            .write_all(&self.crl(Vec::default(), crl_update_seconds))
            .unwrap();
    }
}

impl PkiAuthority for TestPki {
    fn issue_operator(&self, name: &str) -> Result<IssuedOperator, String> {
        let mut params =
            rcgen::CertificateParams::new(Vec::new()).map_err(|e| e.to_string())?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        params.is_ca = rcgen::IsCa::NoCa;
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];

        let serial_bytes = rand::random::<u64>().to_be_bytes().to_vec();
        params.serial_number = Some(rcgen::SerialNumber::from(serial_bytes.clone()));

        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
        let cert = params
            .signed_by(&key, &self.ca_cert, &self.ca_key)
            .map_err(|e| e.to_string())?;

        Ok(IssuedOperator {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            serial_hex: to_hex(&serial_bytes),
        })
    }

    fn implant_credentials(&self) -> ImplantCredentials {
        ImplantCredentials {
            ca_cert_pem: self.implant_ca_cert.pem(),
            client_cert_pem: self.implant_cert.cert.pem(),
            client_key_pem: self.implant_cert.key_pair.serialize_pem(),
        }
    }
}

fn ensure_parent(path: &str) {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).unwrap();
        }
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// CRL updater that runs in a separate thread. This periodically refreshes the CRL file on disk
/// from the revoked operator serials in the store.
pub struct CrlUpdater {
    pub sleep_duration: Duration,
    pub crl_path: PathBuf,
    pub pki: Arc<TestPki>,
    pub store: Arc<dyn Store>,
}

impl CrlUpdater {
    pub fn new(
        sleep_duration: Duration,
        crl_path: String,
        pki: Arc<TestPki>,
        store: Arc<dyn Store>,
    ) -> Self {
        CrlUpdater {
            sleep_duration,
            crl_path: PathBuf::from(crl_path),
            pki,
            store,
        }
    }

    pub fn run(self) {
        loop {
            std::thread::sleep(self.sleep_duration);

            let revoked: Vec<rcgen::SerialNumber> = self
                .store
                .revoked_operator_serials()
                .iter()
                .filter_map(|serial| from_hex(serial))
                .map(rcgen::SerialNumber::from)
                .collect();

            // Write the new CRL content to a temp file, this avoids a race condition where the server
            // reads the configured CRL path while we're in the process of writing it.
            let mut tmp_path = self.crl_path.clone();
            tmp_path.set_extension("tmp");
            ensure_parent(&tmp_path.to_string_lossy());
            let mut crl_der = File::create(&tmp_path).unwrap();
            crl_der
                .write_all(&self.pki.crl(revoked, self.sleep_duration.as_secs()))
                .unwrap();

            // Once the new CRL content is available, atomically rename.
            std::fs::rename(&tmp_path, &self.crl_path).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, UnixTime};
    use rustls::server::danger::ClientCertVerifier;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn revoked_operator_is_rejected() {
        let dir = std::env::temp_dir().join(format!("rb_pki_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ca_cert = dir.join("ca-cert.pem");
        let ca_key = dir.join("ca-key.pem");
        let implant_ca_cert = dir.join("implant-ca-cert.pem");
        let implant_ca_key = dir.join("implant-ca-key.pem");
        let pki = TestPki::load_or_create(
            ca_cert.to_str().unwrap(),
            ca_key.to_str().unwrap(),
            implant_ca_cert.to_str().unwrap(),
            implant_ca_key.to_str().unwrap(),
        );

        let issued = pki.issue_operator("tester").unwrap();
        let cert_der: CertificateDer<'static> =
            rustls_pemfile::certs(&mut issued.cert_pem.as_bytes())
                .next()
                .unwrap()
                .unwrap();

        let now = UnixTime::since_unix_epoch(
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap(),
        );

        // A CRL that does not list the cert must accept it.
        let empty_crl = pki.crl(vec![], 3600);
        let ok_verifier = WebPkiClientVerifier::builder(pki.roots.clone())
            .with_crls([empty_crl])
            .build()
            .unwrap();
        assert!(
            ok_verifier.verify_client_cert(&cert_der, &[], now).is_ok(),
            "non-revoked cert was rejected"
        );

        // A CRL listing the serial must reject it.
        let serial = from_hex(&issued.serial_hex).unwrap();
        let revoked_crl = pki.crl(vec![rcgen::SerialNumber::from(serial)], 3600);
        let revoked_verifier = WebPkiClientVerifier::builder(pki.roots.clone())
            .with_crls([revoked_crl])
            .build()
            .unwrap();
        let result = revoked_verifier.verify_client_cert(&cert_der, &[], now);
        assert!(result.is_err(), "revoked cert was accepted: {:?}", result);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn implant_channel_rejects_operator_cert() {
        let dir = std::env::temp_dir().join(format!("rb_pki2_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pki = TestPki::load_or_create(
            dir.join("ca-cert.pem").to_str().unwrap(),
            dir.join("ca-key.pem").to_str().unwrap(),
            dir.join("implant-ca-cert.pem").to_str().unwrap(),
            dir.join("implant-ca-key.pem").to_str().unwrap(),
        );

        let now = UnixTime::since_unix_epoch(
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap(),
        );
        let verifier = WebPkiClientVerifier::builder(pki.implant_roots.clone())
            .build()
            .unwrap();

        let operator = pki.issue_operator("tester").unwrap();
        let operator_der: CertificateDer<'static> =
            rustls_pemfile::certs(&mut operator.cert_pem.as_bytes())
                .next()
                .unwrap()
                .unwrap();
        assert!(
            verifier.verify_client_cert(&operator_der, &[], now).is_err(),
            "operator cert was accepted on the implant channel"
        );

        let implant_der = pki.implant_cert.cert.der().clone();
        assert!(
            verifier.verify_client_cert(&implant_der, &[], now).is_ok(),
            "implant cert was rejected on the implant channel"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn implant_credentials_and_listener_config_are_usable() {
        let dir = std::env::temp_dir().join(format!("rb_pki3_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pki = TestPki::load_or_create(
            dir.join("ca-cert.pem").to_str().unwrap(),
            dir.join("ca-key.pem").to_str().unwrap(),
            dir.join("implant-ca-cert.pem").to_str().unwrap(),
            dir.join("implant-ca-key.pem").to_str().unwrap(),
        );

        let creds = pki.implant_credentials();
        assert!(creds.ca_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(creds.client_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(creds.client_key_pem.contains("BEGIN PRIVATE KEY"));

        let _config = pki.listener_server_config();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
