use crate::{profiles::Profile, Error, Result};
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};
pub type Values = BTreeMap<String, String>;
/// Token is deliberately never Debug/Serialize; callers must supply a dedicated vault identity.
pub struct OpenBao {
    client: Client,
    profile: Profile,
}
impl OpenBao {
    pub fn new(profile: Profile, token: &str) -> Result<Self> {
        profile.validate()?;
        if token.is_empty() {
            return Err(Error::Denied);
        }
        let mut headers = HeaderMap::new();
        let mut value = HeaderValue::from_str(token).map_err(|_| Error::Invalid)?;
        value.set_sensitive(true);
        headers.insert("X-Vault-Token", value);
        let mut builder = Client::builder()
            .default_headers(headers)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(20));
        if let Ok(path) = std::env::var("WK_BAO_CA_FILE") {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| Error::Local)?
                .take(65537)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Local)?;
            if bytes.len() > 65536 {
                return Err(Error::Invalid);
            }
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(&bytes).map_err(|_| Error::Invalid)?,
            );
        }
        let client = builder.build().map_err(|_| Error::Vault)?;
        Ok(Self { client, profile })
    }
    fn url(&self) -> String {
        format!(
            "{}/v1/{}/data/{}",
            self.profile.address.trim_end_matches('/'),
            self.profile.mount,
            self.profile.path
        )
    }
    fn response(response: reqwest::Result<reqwest::blocking::Response>) -> Result<Value> {
        let response = response.map_err(|_| Error::Vault)?;
        if !response.status().is_success() {
            return Err(Error::Denied);
        }
        // Bounded payload and content-free errors: never print upstream responses.
        use std::io::Read;
        let mut bytes = Vec::new();
        response
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Vault)?;
        if bytes.len() > 1_048_576 {
            return Err(Error::Vault);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::Vault)
    }
    pub fn read(&self) -> Result<Values> {
        let body = Self::response(self.client.get(self.url()).send())?;
        let values: Values =
            serde_json::from_value(body.pointer("/data/data").cloned().ok_or(Error::Vault)?)
                .map_err(|_| Error::Vault)?;
        self.check(&values)?;
        Ok(values)
    }
    pub fn write(&self, values: &Values) -> Result<()> {
        self.check(values)?;
        // CAS=0 intentionally refuses overwriting/rotating existing data by import.
        Self::response(
            self.client
                .post(self.url())
                .json(&json!({"options":{"cas":0},"data":values}))
                .send(),
        )?;
        Ok(())
    }
    fn check(&self, values: &Values) -> Result<()> {
        if values
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            != self.profile.keys
            || values.values().any(|v| v.contains('\0') || v.len() > 65536)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
