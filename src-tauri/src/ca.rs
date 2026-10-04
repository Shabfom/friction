//! Per-machine root certificate authority used to inspect HTTPS traffic.
//!
//! The CA is generated on first launch and stored in ~/.friction (key file
//! mode 0600). It never ships with the app or lives in the repository, so
//! every install has its own key.

use crate::config::{data_dir, write_private};
use chrono::Datelike;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, SerialNumber,
};
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type BoxErr = Box<dyn std::error::Error + Send + Sync>;

pub struct Ca {
    pub cert_path: PathBuf,
    issuer: rcgen::Certificate,
    issuer_key: KeyPair,
    cache: Mutex<HashMap<String, Arc<ServerConfig>>>,
}

fn ymd(dt: chrono::DateTime<chrono::Utc>) -> time::OffsetDateTime {
    rcgen::date_time_ymd(dt.year(), dt.month() as u8, dt.day() as u8)
}

fn random_serial() -> SerialNumber {
    let mut bytes = uuid::Uuid::new_v4().as_bytes().to_vec();
    bytes[0] &= 0x7f;
    SerialNumber::from(bytes)
}

impl Ca {
    pub fn load_or_create() -> Result<Ca, BoxErr> {
        let dir = data_dir();
        let cert_path = dir.join("friction-root-ca.pem");
        let key_path = dir.join("friction-root-ca.key");

        if let (Ok(cert_pem), Ok(key_pem)) =
            (std::fs::read_to_string(&cert_path), std::fs::read_to_string(&key_path))
        {
            let issuer_key = KeyPair::from_pem(&key_pem)?;
            let params = CertificateParams::from_ca_cert_pem(&cert_pem)?;
            let issuer = params.self_signed(&issuer_key)?;
            return Ok(Ca { cert_path, issuer, issuer_key, cache: Mutex::new(HashMap::new()) });
        }

        let mut params = CertificateParams::new(Vec::<String>::new())?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Friction Local Root CA");
        dn.push(DnType::OrganizationName, "Friction (this Mac only)");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.serial_number = Some(random_serial());
        let now = chrono::Utc::now();
        params.not_before = ymd(now - chrono::Duration::days(1));
        params.not_after = ymd(now + chrono::Duration::days(3650));

        let issuer_key = KeyPair::generate()?;
        let issuer = params.self_signed(&issuer_key)?;

        write_private(&key_path, issuer_key.serialize_pem().as_bytes())?;
        std::fs::write(&cert_path, issuer.pem())?;

        Ok(Ca { cert_path, issuer, issuer_key, cache: Mutex::new(HashMap::new()) })
    }

    /// TLS server config presenting a leaf certificate for `host`, signed by the CA.
    pub fn server_config_for(&self, host: &str) -> Result<Arc<ServerConfig>, BoxErr> {
        let host = host.trim_start_matches('[').trim_end_matches(']').to_lowercase();
        if let Some(cfg) = self.cache.lock().unwrap().get(&host) {
            return Ok(cfg.clone());
        }

        let mut params = CertificateParams::new(vec![host.clone()])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, host.clone());
        params.distinguished_name = dn;
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        params.serial_number = Some(random_serial());
        let now = chrono::Utc::now();
        params.not_before = ymd(now - chrono::Duration::days(1));
        params.not_after = ymd(now + chrono::Duration::days(365));

        let leaf_key = KeyPair::generate()?;
        let leaf = params.signed_by(&leaf_key, &self.issuer, &self.issuer_key)?;

        let chain: Vec<CertificateDer<'static>> = vec![leaf.der().clone()];
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));

        let mut cfg = ServerConfig::builder().with_no_client_auth().with_single_cert(chain, key)?;
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let cfg = Arc::new(cfg);
        self.cache.lock().unwrap().insert(host, cfg.clone());
        Ok(cfg)
    }

    /// Whether macOS currently trusts this CA (checked with `security verify-cert`).
    pub fn is_trusted(&self) -> bool {
        std::process::Command::new("/usr/bin/security")
            .arg("verify-cert")
            .arg("-c")
            .arg(&self.cert_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Adds the CA to the user's login keychain as a trusted root.
    /// macOS shows its own password / Touch ID prompt for this.
    pub fn install_trust(&self) -> Result<(), String> {
        let home = std::env::var("HOME").map_err(|e| e.to_string())?;
        let keychain = PathBuf::from(home).join("Library/Keychains/login.keychain-db");
        let out = std::process::Command::new("/usr/bin/security")
            .arg("add-trusted-cert")
            .arg("-r")
            .arg("trustRoot")
            .arg("-k")
            .arg(keychain)
            .arg(&self.cert_path)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}
