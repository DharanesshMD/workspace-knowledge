//! The signed-in developer's refresh token, kept only in the OS credential store.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// What `wk-mcp login` leaves behind for one server. The provider endpoints are rediscovered
/// from `issuer`; nothing here is an access token.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stored {
    pub issuer: String,
    pub client_id: String,
    pub resource: String,
    pub refresh_token: String,
}
impl Drop for Stored {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.refresh_token);
    }
}
pub trait Store: Send + Sync {
    fn load(&self) -> Result<Option<Stored>, String>;
    fn save(&self, stored: &Stored) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}
/// One credential-store entry per server origin, named like the collector's entries.
pub struct Keyring(keyring::Entry);
impl Keyring {
    pub fn for_server(origin: &str) -> Result<Self, String> {
        if !cfg!(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )) {
            return Err("native credential store unsupported".into());
        }
        let account = format!("mcp:{:x}", Sha256::digest(origin.as_bytes()));
        keyring::Entry::new("workspace-knowledge", &account)
            .map(Self)
            .map_err(|_| "native credential store unavailable".into())
    }
}
impl Store for Keyring {
    fn load(&self) -> Result<Option<Stored>, String> {
        match self.0.get_secret() {
            Ok(secret) => serde_json::from_slice(&Zeroizing::new(secret))
                .map(Some)
                .map_err(|_| "stored sign-in is unreadable; run wk-mcp login again".into()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("native credential unavailable".into()),
        }
    }
    fn save(&self, stored: &Stored) -> Result<(), String> {
        let bytes = Zeroizing::new(
            serde_json::to_vec(stored).map_err(|_| "native credential storage failed")?,
        );
        self.0
            .set_secret(&bytes)
            .map_err(|_| "native credential storage failed".into())
    }
    fn delete(&self) -> Result<(), String> {
        match self.0.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("native credential removal failed".into()),
        }
    }
}
#[cfg(test)]
pub use memory::Memory;
#[cfg(test)]
mod memory {
    use super::*;
    use std::sync::Mutex;
    /// In-memory stand-in used by tests; it round-trips through the same JSON as the real store.
    #[derive(Default)]
    pub struct Memory(pub Mutex<Option<Vec<u8>>>);
    impl Store for Memory {
        fn load(&self) -> Result<Option<Stored>, String> {
            self.0
                .lock()
                .unwrap()
                .as_deref()
                .map(|b| {
                    serde_json::from_slice(b).map_err(|_| "stored sign-in is unreadable".into())
                })
                .transpose()
        }
        fn save(&self, stored: &Stored) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(serde_json::to_vec(stored).unwrap());
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accounts_are_per_server_and_do_not_contain_the_origin() {
        let name = |origin: &str| format!("mcp:{:x}", Sha256::digest(origin.as_bytes()));
        assert_ne!(name("https://a.example"), name("https://b.example"));
        assert!(!name("https://a.example").contains("a.example"));
        assert!(Keyring::for_server("https://a.example").is_ok());
    }
    #[test]
    fn a_stored_record_rejects_unknown_fields_and_round_trips() {
        let memory = Memory::default();
        assert!(memory.load().unwrap().is_none());
        let stored = Stored {
            issuer: "https://idp.example".into(),
            client_id: "wk-mcp".into(),
            resource: "https://wk.example".into(),
            refresh_token: "r".into(),
        };
        memory.save(&stored).unwrap();
        assert_eq!(memory.load().unwrap().unwrap().client_id, "wk-mcp");
        *memory.0.lock().unwrap() = Some(
            br#"{"issuer":"i","client_id":"c","resource":"r","refresh_token":"t","extra":1}"#
                .to_vec(),
        );
        assert!(memory.load().is_err());
        memory.delete().unwrap();
        assert!(memory.load().unwrap().is_none());
    }
}
