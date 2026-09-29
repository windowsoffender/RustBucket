use std::fs::File;
use std::io::{Read, Write};
use std::ops::Add;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rcgen::KeyPair;
use rustls::pki_types::{CertificateRevocationListDer, PrivatePkcs8KeyDer};
use rustls::server::{ServerConfig, WebPkiClientVerifier};
use rustls::RootCertStore;

/// A test PKI with a CA certificate, server certificate, and client certificate.
pub struct TestPki {
    pub roots: Arc<RootCertStore>,
    pub ca_cert: rcgen::CertifiedKey,
    pub client_cert: rcgen::CertifiedKey,
    pub server_cert: rcgen::CertifiedKey,
}

impl TestPki {
    /// Create a new test PKI using `rcgen`.
    pub fn new() -> Self {
        // Create an issuer CA cert.
        let alg = &rcgen::PKCS_ECDSA_P256_SHA256;
        let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::OrganizationName, "RustBucket C2");
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "RustBucket CA");
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let ca_key = KeyPair::generate_for(alg).unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();

        // Create a server end entity cert issued by the CA.
        let mut server_ee_params =
            rcgen::CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .unwrap();
        server_ee_params.is_ca = rcgen::IsCa::NoCa;
        server_ee_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let ee_key = KeyPair::generate_for(alg).unwrap();
        let server_cert = server_ee_params
            .signed_by(&ee_key, &ca_cert, &ca_key)
            .unwrap();

        // Create a client end entity cert issued by the CA.
        let mut client_ee_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        client_ee_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "RustBucket Client");
        client_ee_params.is_ca = rcgen::IsCa::NoCa;
        client_ee_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        client_ee_params.serial_number = Some(rcgen::SerialNumber::from(vec![0xC0, 0xFF, 0xEE]));
        let client_key = KeyPair::generate_for(alg).unwrap();
        let client_cert = client_ee_params
            .signed_by(&client_key, &ca_cert, &ca_key)
            .unwrap();

        // Create a root cert store that includes the CA certificate.
        let mut roots = RootCertStore::empty();
        roots.add(ca_cert.der().clone()).unwrap();
        Self {
            roots: Arc::new(roots),
            ca_cert: rcgen::CertifiedKey {
                cert: ca_cert,
                key_pair: ca_key,
            },
            client_cert: rcgen::CertifiedKey {
                cert: client_cert,
                key_pair: client_key,
            },
            server_cert: rcgen::CertifiedKey {
                cert: server_cert,
                key_pair: ee_key,
            },
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

        // Build a server config using the fresh verifier. If necessary, this could be customized
        // based on the ClientHello (e.g. selecting a different certificate, or customizing
        // supported algorithms/protocol versions).
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

    /// Issue a certificate revocation list (CRL) for the revoked `serials` provided (may be empty).
    /// The CRL will be signed by the test PKI CA and returned in DER serialized form.
    pub fn crl(
        &self,
        serials: Vec<rcgen::SerialNumber>,
        next_update_seconds: u64,
    ) -> CertificateRevocationListDer {
        // In a real use-case you would want to set this to the current date/time.
        let now = rcgen::date_time_ymd(2023, 1, 1);

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
            next_update: now.add(Duration::from_secs(next_update_seconds)),
            crl_number: rcgen::SerialNumber::from(1234),
            issuing_distribution_point: None,
            revoked_certs,
            key_identifier_method: rcgen::KeyIdMethod::Sha256,
        };
        crl_params
            .signed_by(&self.ca_cert.cert, &self.ca_cert.key_pair)
            .unwrap()
            .into()
    }

    /// Write the certificates and keys to disk
    pub fn write_to_disk(
        &self,
        ca_path: &str,
        client_cert_path: &str,
        client_key_path: &str,
        crl_path: &str,
        crl_update_seconds: u64,
    ) {
        // Helper function to write PEM files
        let write_pem = |path: &str, pem: &str| {
            if let Some(parent) = std::path::Path::new(path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).unwrap();
                }
            }
            let mut file = File::create(path).unwrap();
            file.write_all(pem.as_bytes()).unwrap();
        };

        // Write out the parts of the test PKI a client will need to connect:
        // * The CA certificate for validating the server certificate.
        // * The client certificate and key for its presented mTLS identity.
        write_pem(ca_path, &self.ca_cert.cert.pem());
        write_pem(client_cert_path, &self.client_cert.cert.pem());
        write_pem(client_key_path, &self.client_cert.key_pair.serialize_pem());

        // Write out an initial DER CRL that has no revoked certificates.
        if let Some(parent) = std::path::Path::new(crl_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).unwrap();
            }
        }
        let mut crl_der = File::create(crl_path).unwrap();
        crl_der
            .write_all(&self.crl(Vec::default(), crl_update_seconds))
            .unwrap();
    }
}

/// CRL updater that runs in a separate thread. This periodically refreshes the CRL file on disk.
///
/// In a real use case, the CRL would be updated by fetching fresh CRL data from an authoritative
/// distribution point. Here we keep issuing an empty CRL (no revoked certs) so the client identity
/// stays valid.
pub struct CrlUpdater {
    pub sleep_duration: Duration,
    pub crl_path: PathBuf,
    pub pki: Arc<TestPki>,
}

impl CrlUpdater {
    pub fn new(sleep_duration: Duration, crl_path: String, pki: Arc<TestPki>) -> Self {
        CrlUpdater {
            sleep_duration,
            crl_path: PathBuf::from(crl_path),
            pki,
        }
    }

    pub fn run(self) {
        loop {
            std::thread::sleep(self.sleep_duration);

            // Write the new CRL content to a temp file, this avoids a race condition where the server
            // reads the configured CRL path while we're in the process of writing it.
            let mut tmp_path = self.crl_path.clone();
            tmp_path.set_extension("tmp");
            let mut crl_der = File::create(&tmp_path).unwrap();
            crl_der
                .write_all(&self.pki.crl(Vec::default(), self.sleep_duration.as_secs()))
                .unwrap();

            // Once the new CRL content is available, atomically rename.
            std::fs::rename(&tmp_path, &self.crl_path).unwrap();
        }
    }
}
