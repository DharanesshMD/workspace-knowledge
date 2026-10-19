//! Operator-supplied private CA bundles for hosted-server and identity-provider requests.

pub const CA_BUNDLE_MAX: usize = 65536;

/// Read a user-named CA bundle: a regular file of at most 64 KiB holding only certificates.
pub fn read_ca_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "CA file unreadable")?;
    if !file.metadata().map_err(|_| "CA file unreadable")?.is_file() {
        return Err("CA file must be a regular file".into());
    }
    let mut pem = Vec::new();
    file.take(CA_BUNDLE_MAX as u64 + 1)
        .read_to_end(&mut pem)
        .map_err(|_| "CA file unreadable")?;
    parse_ca_bundle(&pem)?;
    Ok(pem)
}

/// The bundle named by an environment variable the operator set, if any. An unset or empty
/// variable means public roots; a set one that cannot be used is an error, never ignored.
pub fn from_env(variable: &str) -> Result<Option<Vec<reqwest::Certificate>>, String> {
    match std::env::var_os(variable).filter(|path| !path.is_empty()) {
        None => Ok(None),
        Some(path) => parse_ca_bundle(&read_ca_file(std::path::Path::new(&path))?).map(Some),
    }
}

/// Parse an operator-supplied PEM bundle. It must hold at least one certificate and nothing
/// else: a key, a malformed block or trailing junk is an error, never silently skipped.
pub fn parse_ca_bundle(pem: &[u8]) -> Result<Vec<reqwest::Certificate>, String> {
    if pem.len() > CA_BUNDLE_MAX {
        return Err("CA bundle too large".into());
    }
    let text = std::str::from_utf8(pem).map_err(|_| "invalid CA bundle")?;
    let blocks = text.matches("-----BEGIN ").count();
    if text.matches("-----BEGIN CERTIFICATE-----").count() != blocks {
        return Err("invalid CA bundle".into());
    }
    let certificates =
        reqwest::Certificate::from_pem_bundle(pem).map_err(|_| "invalid CA bundle")?;
    if certificates.is_empty() {
        return Err("CA bundle contains no certificates".into());
    }
    if certificates.len() != blocks {
        return Err("invalid CA bundle".into());
    }
    // Parsing is lazy: building a client is what rejects malformed certificate data.
    let mut probe = reqwest::Client::builder().tls_built_in_root_certs(false);
    for certificate in &certificates {
        probe = probe.add_root_certificate(certificate.clone());
    }
    probe.build().map_err(|_| "invalid CA bundle")?;
    Ok(certificates)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A throwaway CA certificate and an unrelated private key, generated per test process.
    fn pem() -> &'static (Vec<u8>, Vec<u8>) {
        static PEM: std::sync::OnceLock<(Vec<u8>, Vec<u8>)> = std::sync::OnceLock::new();
        PEM.get_or_init(|| {
            let key = rcgen::KeyPair::generate().unwrap();
            let mut params = rcgen::CertificateParams::new(Vec::new()).unwrap();
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            let certificate = params.self_signed(&key).unwrap();
            (
                certificate.pem().into_bytes(),
                rcgen::KeyPair::generate()
                    .unwrap()
                    .serialize_pem()
                    .into_bytes(),
            )
        })
    }

    fn ca() -> &'static [u8] {
        &pem().0
    }
    fn key() -> &'static [u8] {
        &pem().1
    }

    #[test]
    fn bundle_accepts_only_certificates() {
        assert_eq!(parse_ca_bundle(ca()).unwrap().len(), 1);
        let doubled = [ca(), ca()].concat();
        assert_eq!(parse_ca_bundle(&doubled).unwrap().len(), 2);
        let mixed = [ca(), key()].concat();
        let key_only = key().to_vec();
        let truncated = b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n".to_vec();
        for (name, bad) in [
            ("empty", Vec::new()),
            ("text", b"not a certificate".to_vec()),
            ("corrupt certificate", truncated),
            ("private key", key_only),
            ("certificate plus private key", mixed),
            ("oversized", vec![b'A'; CA_BUNDLE_MAX + 1]),
        ] {
            assert!(parse_ca_bundle(&bad).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn a_ca_file_must_be_a_small_regular_certificate_file() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("ca.pem");
        std::fs::write(&good, ca()).unwrap();
        assert_eq!(read_ca_file(&good).unwrap(), ca());
        assert!(read_ca_file(&dir.path().join("missing.pem")).is_err());
        assert!(
            read_ca_file(dir.path()).is_err(),
            "a directory is not a bundle"
        );
        for (name, bytes) in [
            ("empty.pem", vec![]),
            ("big.pem", vec![b'A'; 70_000]),
            ("junk.pem", b"nope".to_vec()),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            assert!(read_ca_file(&path).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn an_environment_bundle_is_used_only_when_the_operator_set_it() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("ca.pem");
        std::fs::write(&good, ca()).unwrap();
        let var = "WK_TEST_TLS_FROM_ENV_ONLY";
        std::env::remove_var(var);
        assert!(from_env(var).unwrap().is_none());
        std::env::set_var(var, "");
        assert!(from_env(var).unwrap().is_none());
        std::env::set_var(var, &good);
        assert_eq!(from_env(var).unwrap().unwrap().len(), 1);
        std::env::set_var(var, dir.path().join("missing.pem"));
        assert!(
            from_env(var).is_err(),
            "an unusable bundle must not be ignored"
        );
        std::env::remove_var(var);
    }

    #[test]
    fn bundle_size_limit_is_checked_before_parsing() {
        let padded = [ca(), &vec![b'\n'; CA_BUNDLE_MAX][..]].concat();
        assert_eq!(parse_ca_bundle(&padded).unwrap_err(), "CA bundle too large");
    }
}
