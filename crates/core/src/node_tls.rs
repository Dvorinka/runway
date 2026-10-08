//! mTLS certificate bundles for remote Docker nodes.
//!
//! Generates a dedicated CA per node plus a server certificate (installed on
//! the remote dockerd) and a client certificate (used by `Docker::connect_with_ssl`).

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    PKCS_ECDSA_P256_SHA256,
};

use crate::error::{Error, Result};

pub struct NodeTlsBundle {
    pub ca_pem: String,
    /// Installed on the remote dockerd (`tlscert`/`tlskey`).
    pub server_cert_pem: String,
    pub server_key_pem: String,
    /// Stored on the node row; runway presents these to dockerd.
    pub client_cert_pem: String,
    pub client_key_pem: String,
}

fn new_key() -> Result<KeyPair> {
    KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|e| Error::Crypto(format!("key generation failed: {e}")))
}

/// Generate a CA + server cert (SAN = `host`) + client cert for one node.
/// `host` should be the DNS name or IP the daemon will be reached at.
pub fn generate_node_tls(host: &str) -> Result<NodeTlsBundle> {
    let host = host.trim();
    if host.is_empty() {
        return Err(Error::BadRequest("node host is required".into()));
    }

    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "runway-node-ca");
    let ca_key = new_key()?;
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| Error::Crypto(format!("ca generation failed: {e}")))?;

    // CertificateParams::new parses IP SANs automatically.
    let mut server_params = CertificateParams::new(vec![host.to_string()])
        .map_err(|e| Error::BadRequest(format!("invalid node host: {e}")))?;
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params
        .distinguished_name
        .push(DnType::CommonName, host);
    let server_key = new_key()?;
    let server_cert = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .map_err(|e| Error::Crypto(format!("server cert failed: {e}")))?;

    let mut client_params = CertificateParams::default();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    client_params
        .distinguished_name
        .push(DnType::CommonName, "runway-node-client");
    let client_key = new_key()?;
    let client_cert = client_params
        .signed_by(&client_key, &ca_cert, &ca_key)
        .map_err(|e| Error::Crypto(format!("client cert failed: {e}")))?;

    Ok(NodeTlsBundle {
        ca_pem: ca_cert.pem(),
        server_cert_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
        client_cert_pem: client_cert.pem(),
        client_key_pem: client_key.serialize_pem(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_is_valid_pem_with_host_san() {
        let b = generate_node_tls("203.0.113.10").unwrap();
        for pem in [
            &b.ca_pem,
            &b.server_cert_pem,
            &b.client_cert_pem,
            &b.server_key_pem,
            &b.client_key_pem,
        ] {
            assert!(pem.starts_with("-----BEGIN"), "not PEM: {pem:.40}");
        }
        // Server cert must carry the host as an IP SAN.
        let pem = x509_parser::pem::parse_x509_pem(b.server_cert_pem.as_bytes())
            .unwrap()
            .1;
        let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).unwrap();
        let san = cert
            .subject_alternative_name()
            .unwrap()
            .expect("server cert must have SANs");
        assert!(matches!(
            &san.value.general_names[0],
            x509_parser::extensions::GeneralName::IPAddress(_)
        ));
    }

    #[test]
    fn rejects_empty_host() {
        assert!(generate_node_tls("  ").is_err());
    }

    /// End-to-end mTLS: spawn `openssl s_server` with the server bundle
    /// and connect through bollard the same way `node_docker_client` does.
    /// Needs `openssl` on PATH; skipped otherwise.
    #[tokio::test]
    async fn bollard_ssl_handshake_with_real_certs() {
        if std::process::Command::new("openssl")
            .arg("version")
            .output()
            .is_err()
        {
            return;
        }
        let b = generate_node_tls("127.0.0.1").unwrap();
        let dir = std::env::temp_dir().join(format!("rw-tls-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, pem) in [
            ("ca.pem", &b.ca_pem),
            ("cert.pem", &b.client_cert_pem),
            ("key.pem", &b.client_key_pem),
            ("srv.pem", &b.server_cert_pem),
            ("srv-key.pem", &b.server_key_pem),
        ] {
            std::fs::write(dir.join(name), pem).unwrap();
        }
        // Ephemeral port: bind then drop so a stale listener can't serve
        // an old CA (silent bind failure would verify the wrong cert).
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut srv = std::process::Command::new("openssl")
            .args([
                "s_server",
                "-accept",
                &port.to_string(),
                "-naccept",
                "1",
                "-cert",
                dir.join("srv.pem").to_str().unwrap(),
                "-key",
                dir.join("srv-key.pem").to_str().unwrap(),
                "-CAfile",
                dir.join("ca.pem").to_str().unwrap(),
                "-Verify",
                "1",
                "-verify_return_error",
                "-www",
            ])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(800));

        let _ = rustls::crypto::ring::default_provider().install_default();
        let docker = bollard::Docker::connect_with_ssl(
            &format!("tcp://127.0.0.1:{port}"),
            &dir.join("key.pem"),
            &dir.join("cert.pem"),
            &dir.join("ca.pem"),
            10,
            bollard::API_DEFAULT_VERSION,
        )
        .unwrap();
        // s_server isn't dockerd, but `-www` answers HTTP — ping() getting
        // any response proves the mTLS handshake incl. client cert verify.
        let r = docker.ping().await;
        let _ = srv.kill();
        let _ = srv.wait();
        assert!(r.is_ok(), "mTLS handshake or request failed: {r:?}");
    }
}
