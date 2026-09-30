//! Encrypted local outbox. Callers supply a key from secure OS storage.
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    XChaCha20Poly1305, XNonce,
};
use rusqlite::{params, Connection};
use zeroize::Zeroize;
pub struct LocalStore {
    db: Connection,
    cipher: XChaCha20Poly1305,
    quota: usize,
}
fn db_error(_: rusqlite::Error) -> String {
    "local storage operation failed".into()
}
impl LocalStore {
    pub fn open(path: &std::path::Path, mut key: [u8; 32], quota: usize) -> Result<Self, String> {
        let cipher = XChaCha20Poly1305::new((&key).into());
        key.zeroize();
        let db = Connection::open(path).map_err(db_error)?;
        db.busy_timeout(std::time::Duration::from_millis(100))
            .map_err(db_error)?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON; CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY, nonce BLOB NOT NULL, payload BLOB NOT NULL, expires INTEGER NOT NULL);").map_err(db_error)?;
        Ok(Self { db, cipher, quota })
    }
    pub fn enqueue(&mut self, id: &str, data: &[u8], now: i64, ttl: i64) -> Result<bool, String> {
        if id.is_empty() || id.len() > 256 || ttl <= 0 || ttl > 86400 || data.len() > 1024 * 1024 {
            return Err("invalid event or retention".into());
        }
        let expires = now.checked_add(ttl).ok_or("invalid expiry")?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute("DELETE FROM events WHERE expires <= ?", [now])
            .map_err(db_error)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE id=?)",
                [id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if exists {
            tx.commit().map_err(db_error)?;
            return Ok(false);
        }
        let used: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(length(payload)+length(nonce)+length(id)),0) FROM events",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let required = data.len().saturating_add(40).saturating_add(id.len());
        if (used as usize).saturating_add(required) > self.quota {
            return Err("capture quota exceeded".into());
        }
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let aad = format!("v1:{id}:{expires}");
        let encrypted = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: data,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "capture encryption failed")?;
        tx.execute(
            "INSERT INTO events(id,nonce,payload,expires) VALUES(?,?,?,?)",
            params![id, nonce.as_slice(), encrypted, expires],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(true)
    }
    pub fn pending(&mut self, now: i64) -> Result<Vec<(String, Vec<u8>)>, String> {
        self.db
            .execute("DELETE FROM events WHERE expires <= ?", [now])
            .map_err(db_error)?;
        let mut stmt = self
            .db
            .prepare("SELECT id,nonce,payload,expires FROM events ORDER BY rowid LIMIT 100")
            .map_err(db_error)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(db_error)?;
        let mut result = Vec::new();
        for row in rows {
            let (id, nonce, payload, expires) = row.map_err(db_error)?;
            if nonce.len() != 24 {
                return Err("invalid encrypted event".into());
            }
            let aad = format!("v1:{id}:{expires}");
            let plain = self
                .cipher
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &payload,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| "capture authentication failed")?;
            result.push((id, plain));
        }
        Ok(result)
    }
    pub fn ack(&mut self, id: &str) -> Result<(), String> {
        self.db
            .execute("DELETE FROM events WHERE id=?", [id])
            .map_err(db_error)?;
        Ok(())
    }
}
