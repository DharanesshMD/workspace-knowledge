//! Operator ingestion of exact signed bytes. Trust policy is local deployment configuration.
use super::*;
use ed25519_dalek::{Signature, VerifyingKey};
use std::collections::BTreeMap;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trust {
    pub keys: BTreeMap<String, TrustedKey>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedKey {
    pub public_key_hex: String,
    pub provider: String,
    pub org: String,
    pub product: String,
    pub hosts: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub key_id: String,
    pub payload: String,
    pub signature_hex: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    event_id: uuid::Uuid,
    org: String,
    product: String,
    card_id: uuid::Uuid,
    card_revision: i64,
    issued_at: i64,
    expires_at: i64,
    evidence: evidence::VerifiedEvidence,
    #[serde(default)]
    revoke: bool,
}
impl Trust {
    fn verify(&self, envelope: &Envelope, now: i64) -> Result<Payload, Error> {
        if envelope.payload.len() > 16384 || envelope.signature_hex.len() != 128 {
            return Err(Error::Invalid);
        }
        let trust = self.keys.get(&envelope.key_id).ok_or(Error::Denied)?;
        let key: [u8; 32] = hex::decode(&trust.public_key_hex)
            .map_err(|_| Error::Invalid)?
            .try_into()
            .map_err(|_| Error::Invalid)?;
        let signature = Signature::from_slice(
            &hex::decode(&envelope.signature_hex).map_err(|_| Error::Invalid)?,
        )
        .map_err(|_| Error::Invalid)?;
        VerifyingKey::from_bytes(&key)
            .map_err(|_| Error::Invalid)?
            .verify_strict(envelope.payload.as_bytes(), &signature)
            .map_err(|_| Error::Denied)?;
        let payload: Payload =
            serde_json::from_str(&envelope.payload).map_err(|_| Error::Invalid)?;
        if payload.org != trust.org
            || payload.product != trust.product
            || payload.evidence.provider != trust.provider
            || payload.issued_at > now
            || payload.expires_at <= now
            || payload
                .expires_at
                .checked_sub(payload.issued_at)
                .is_none_or(|n| !(1..=900).contains(&n))
        {
            return Err(Error::Denied);
        }
        payload.evidence.validate(&trust.hosts)?;
        Ok(payload)
    }
    pub async fn ingest(
        &self,
        k: &Knowledge,
        operator: &Actor,
        envelope: Envelope,
        now: i64,
    ) -> Result<(), Error> {
        let payload = self.verify(&envelope, now)?;
        if operator.org != payload.org {
            return Err(Error::Denied);
        }
        let hosts = &self.keys.get(&envelope.key_id).ok_or(Error::Denied)?.hosts;
        k.attest_event(
            operator,
            &payload.product,
            payload.card_id,
            payload.card_revision,
            payload.evidence,
            hosts,
            payload.event_id,
            payload.revoke,
        )
        .await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    #[test]
    fn pinned_signature_scope_and_expiry_are_mandatory() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let trust:Trust=serde_json::from_value(json!({"keys":{"ci":{"public_key_hex":hex::encode(signing.verifying_key().as_bytes()),"provider":"ci","org":"team","product":"product","hosts":["example.org"]}}})).unwrap();
        let payload=json!({"event_id":uuid::Uuid::new_v4(),"org":"team","product":"product","card_id":uuid::Uuid::new_v4(),"card_revision":1,"issued_at":100,"expires_at":200,"evidence":{"provider":"ci","reference":"https://example.org/build/1","code_revision":"abcdef123","kind":"tests_verified","metadata":{"passed":true,"command":"cargo test","executed_at":"synthetic","environment":"synthetic"}}}).to_string();
        let mut envelope = Envelope {
            key_id: "ci".into(),
            signature_hex: hex::encode(signing.sign(payload.as_bytes()).to_bytes()),
            payload,
        };
        assert!(trust.verify(&envelope, 150).is_ok());
        assert!(trust.verify(&envelope, 200).is_err());
        assert!(trust.verify(&envelope, 99).is_err());
        envelope.payload.push(' ');
        assert!(trust.verify(&envelope, 150).is_err());
        envelope.payload.pop();
        envelope.key_id = "unknown".into();
        assert!(trust.verify(&envelope, 150).is_err());
    }
}
