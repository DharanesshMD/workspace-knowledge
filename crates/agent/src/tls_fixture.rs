//! Test-only HTTPS server whose certificate chains to a throwaway private CA.
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{
    rustls::{
        pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
        ServerConfig,
    },
    server::TlsStream,
    TlsAcceptor,
};

/// Throwaway PKI generated per test process, so no key material lives in the repository.
pub struct Pki {
    pub ca: Vec<u8>,
    pub other_ca: Vec<u8>,
    leaf: Vec<u8>,
    key: Vec<u8>,
}
pub fn pki() -> &'static Pki {
    static PKI: std::sync::OnceLock<Pki> = std::sync::OnceLock::new();
    PKI.get_or_init(|| {
        let authority = |name: &str| {
            let key = rcgen::KeyPair::generate().unwrap();
            let mut params = rcgen::CertificateParams::new(Vec::new()).unwrap();
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, name);
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            params.key_usages = vec![
                rcgen::KeyUsagePurpose::KeyCertSign,
                rcgen::KeyUsagePurpose::CrlSign,
            ];
            let certificate = params.self_signed(&key).unwrap();
            (certificate, params, key)
        };
        let (ca, ca_params, ca_key) = authority("WK Agent Test CA");
        let (other, _, _) = authority("WK Agent Unrelated CA");
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
            .unwrap()
            .signed_by(&leaf_key, &rcgen::Issuer::new(ca_params, ca_key))
            .unwrap();
        Pki {
            ca: ca.pem().into_bytes(),
            other_ca: other.pem().into_bytes(),
            leaf: leaf.pem().into_bytes(),
            key: leaf_key.serialize_pem().into_bytes(),
        }
    })
}

struct Tls {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
}
impl axum::serve::Listener for Tls {
    type Io = TlsStream<TcpStream>;
    type Addr = std::net::SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok((stream, address)) = self.tcp.accept().await else {
                continue;
            };
            // A client that rejects the certificate aborts the handshake; keep serving.
            if let Ok(stream) = self.acceptor.accept(stream).await {
                return (stream, address);
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

/// Serve the router built from the `https://127.0.0.1:port` origin over TLS on that port.
pub async fn serve(
    router: impl FnOnce(&str) -> axum::Router,
) -> (String, tokio::task::JoinHandle<()>) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", tcp.local_addr().unwrap());
    let chain = CertificateDer::pem_slice_iter(&pki().leaf)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(&pki().key).unwrap();
    let config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .unwrap();
    let listener = Tls {
        tcp,
        acceptor: TlsAcceptor::from(Arc::new(config)),
    };
    let router = router(&origin);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (origin, task)
}
