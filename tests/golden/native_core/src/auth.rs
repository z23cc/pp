use anyhow::Result;
use base64::Engine;
use crate::context::Context;

impl Context {
    pub fn auth_header() -> Result<Option<(String, String)>> {

        let token = std::env::var("NATIVE_CORE_API_TOKEN")?;
        Ok(Some(("authorization".to_string(), format!("Bearer {token}"))))

    }
}