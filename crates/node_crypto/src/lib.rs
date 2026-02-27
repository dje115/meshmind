//! Node identity, mTLS configuration, and dev CA helpers.
//!
//! Each node has a unique NodeId derived from its certificate fingerprint.
//! Provides helpers to generate dev CA + node certs for local development,
//! and to configure rustls for mTLS.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use sha2::{Digest, Sha256};

/// A node's identity: its ID (derived from cert fingerprint) and key/cert material.
#[derive(Clone)]
pub struct NodeIdentity {
    pub node_id: String,
    pub cert_pem: String,
    pub key_pem: String,
}

/// A dev CA bundle: cert, key, and the rcgen objects needed to sign more certs.
pub struct DevCa {
    pub cert_pem: String,
    pub key_pem: String,
    key_pair: KeyPair,
    params: CertificateParams,
}

impl DevCa {
    /// Generate a new self-signed dev CA.
    pub fn generate() -> Result<Self> {
        let mut params = CertificateParams::default();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "MeshMind Dev CA");
        dn.push(DnType::OrganizationName, "MeshMind");
        params.distinguished_name = dn;

        let key_pair = KeyPair::generate().context("generate CA key pair")?;
        let cert = params
            .clone()
            .self_signed(&key_pair)
            .context("self-sign CA cert")?;

        Ok(Self {
            cert_pem: cert.pem(),
            key_pem: key_pair.serialize_pem(),
            key_pair,
            params,
        })
    }

    /// Generate a node certificate signed by this CA.
    pub fn generate_node_cert(&self, node_name: &str) -> Result<NodeIdentity> {
        let ca_cert = self
            .params
            .clone()
            .self_signed(&self.key_pair)
            .context("rebuild CA cert for signing")?;

        let mut params = CertificateParams::new(vec![
            node_name.to_string(),
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ])
        .context("create node cert params")?;

        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, node_name);
        dn.push(DnType::OrganizationName, "MeshMind");
        params.distinguished_name = dn;

        let node_key = KeyPair::generate().context("generate node key pair")?;
        let node_cert = params
            .signed_by(&node_key, &ca_cert, &self.key_pair)
            .context("sign node cert")?;

        let cert_pem = node_cert.pem();
        let key_pem = node_key.serialize_pem();

        let fingerprint = hex::encode(Sha256::digest(cert_pem.as_bytes()));
        let node_id = format!("node-{}", &fingerprint[..16]);

        Ok(NodeIdentity {
            node_id,
            cert_pem,
            key_pem,
        })
    }
}

/// Derive a node_id from cert PEM by hashing.
pub fn node_id_from_cert(cert_pem: &str) -> String {
    let fingerprint = hex::encode(Sha256::digest(cert_pem.as_bytes()));
    format!("node-{}", &fingerprint[..16])
}

/// Write cert and key to files in a directory.
pub fn write_identity(identity: &NodeIdentity, dir: &Path) -> Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, &identity.cert_pem)?;
    std::fs::write(&key_path, &identity.key_pem)?;
    Ok((cert_path, key_path))
}

fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Build a rustls ServerConfig for mTLS.
pub fn build_server_config(
    cert_pem: &str,
    key_pem: &str,
    ca_cert_pem: &str,
) -> Result<rustls::ServerConfig> {
    ensure_crypto_provider();
    let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parse server certs")?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .context("parse server key")?
        .context("no private key found")?;

    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs = rustls_pemfile::certs(&mut ca_cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parse CA cert")?;
    for cert in ca_certs {
        root_store.add(cert).context("add CA to root store")?;
    }

    let client_verifier = rustls::server::WebPkiClientVerifier::builder(root_store.into())
        .build()
        .context("build client verifier")?;

    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(certs, key)
        .context("build server config")?;

    Ok(config)
}

/// Build a rustls ClientConfig for mTLS.
pub fn build_client_config(
    cert_pem: &str,
    key_pem: &str,
    ca_cert_pem: &str,
) -> Result<rustls::ClientConfig> {
    ensure_crypto_provider();
    let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parse client certs")?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .context("parse client key")?
        .context("no private key found")?;

    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs = rustls_pemfile::certs(&mut ca_cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parse CA cert")?;
    for cert in ca_certs {
        root_store.add(cert).context("add CA to root store")?;
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(certs, key)
        .context("build client config")?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_dev_ca_succeeds() {
        let ca = DevCa::generate().unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn generate_node_cert_succeeds() {
        let ca = DevCa::generate().unwrap();
        let identity = ca.generate_node_cert("test-node").unwrap();
        assert!(identity.node_id.starts_with("node-"));
        assert!(identity.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(identity.key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn two_nodes_different_ids() {
        let ca = DevCa::generate().unwrap();
        let id1 = ca.generate_node_cert("node-alpha").unwrap();
        let id2 = ca.generate_node_cert("node-beta").unwrap();
        assert_ne!(id1.node_id, id2.node_id);
    }

    #[test]
    fn node_id_from_cert_deterministic() {
        let ca = DevCa::generate().unwrap();
        let identity = ca.generate_node_cert("det-node").unwrap();
        let id1 = node_id_from_cert(&identity.cert_pem);
        let id2 = node_id_from_cert(&identity.cert_pem);
        assert_eq!(id1, id2);
        assert_eq!(id1, identity.node_id);
    }

    #[test]
    fn write_and_read_identity() {
        let ca = DevCa::generate().unwrap();
        let identity = ca.generate_node_cert("write-test").unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let (cert_path, key_path) = write_identity(&identity, tmp.path()).unwrap();
        assert!(cert_path.exists());
        assert!(key_path.exists());
        let read_cert = std::fs::read_to_string(&cert_path).unwrap();
        assert_eq!(read_cert, identity.cert_pem);
    }

    #[test]
    fn build_server_config_succeeds() {
        let ca = DevCa::generate().unwrap();
        let identity = ca.generate_node_cert("server-node").unwrap();
        let config = build_server_config(&identity.cert_pem, &identity.key_pem, &ca.cert_pem);
        assert!(config.is_ok());
    }

    #[test]
    fn build_client_config_succeeds() {
        let ca = DevCa::generate().unwrap();
        let identity = ca.generate_node_cert("client-node").unwrap();
        let config = build_client_config(&identity.cert_pem, &identity.key_pem, &ca.cert_pem);
        assert!(config.is_ok());
    }
}
