//! Capability for issuing operator credentials.
//!
//! The server owns the CA and implements [`PkiAuthority`]; the command layer (which lives in this
//! crate) calls it through the trait so `rcgen` never reaches the implant build.

/// A freshly issued operator certificate and key.
pub struct IssuedOperator {
    pub cert_pem: String,
    pub key_pem: String,
    /// Certificate serial number as lowercase hex, used for revocation.
    pub serial_hex: String,
}

/// Issues operator certificates signed by the server CA.
pub trait PkiAuthority: Send + Sync {
    fn issue_operator(&self, name: &str) -> Result<IssuedOperator, String>;
}
