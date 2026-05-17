use anyhow::{Context as _, Result};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Runtime context shared by generated CLI command handlers.
#[derive(Clone)]
pub struct Context {
    pub client: reqwest::Client,
    pub base_url: String,
    pub auth: Option<(String, String)>,
    pub output: OutputMode,
    pub debug_enabled: bool,
}

#[derive(Clone)]
pub enum OutputMode {
    Human,
    Json(Arc<Mutex<CapturedOutput>>),
    Capture(Arc<Mutex<CapturedOutput>>),
}

#[derive(Default)]
pub struct CapturedOutput {
    pub value: Option<serde_json::Value>,
    pub list: Option<Vec<serde_json::Value>>,
}

impl Context {
    pub fn new() -> Result<Self> {
        Self::new_with_output(OutputMode::Human)
    }

    pub fn new_json() -> Result<Self> {
        Self::new_with_output(OutputMode::Json(Arc::new(Mutex::new(CapturedOutput::default()))))
    }

    pub fn new_capture() -> Result<(Self, Arc<Mutex<CapturedOutput>>)> {
        let captured = Arc::new(Mutex::new(CapturedOutput::default()));
        let context = Self::new_with_output(OutputMode::Capture(captured.clone()))?;
        Ok((context, captured))
    }

    pub fn new_mcp() -> Result<Self> {
        Self::new_with_output_and_auth(OutputMode::Human, Self::auth_header().ok().flatten())
    }

    fn new_with_output(output: OutputMode) -> Result<Self> {
        let auth = Self::auth_header()?;
        Self::new_with_output_and_auth(output, auth)
    }

    fn new_with_output_and_auth(output: OutputMode, auth: Option<(String, String)>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some((name, value)) = &auth {
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
                reqwest::header::HeaderValue::from_str(value)?,
            );
        }
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .default_headers(headers);
        if let Some(timeout) = Self::timeout_from_env()? {
            builder = builder.timeout(timeout);
        }
        let client = builder.build()?;
        Ok(Self {
            client,
            base_url: "https://example.test/api".to_string(),
            auth,
            output,
            debug_enabled: Self::debug_enabled_from_env(),
        })
    }

    fn timeout_from_env() -> Result<Option<Duration>> {
        let raw = match std::env::var("NATIVE_CORE_API_TIMEOUT_SECS") {
            Ok(raw) => raw,
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let seconds = raw
            .parse::<f64>()
            .with_context(|| format!("NATIVE_CORE_API_TIMEOUT_SECS must be a positive number of seconds"))?;
        if !seconds.is_finite() || seconds <= 0.0 {
            anyhow::bail!("NATIVE_CORE_API_TIMEOUT_SECS must be a positive number of seconds");
        }
        let timeout = Duration::try_from_secs_f64(seconds)
            .with_context(|| format!("NATIVE_CORE_API_TIMEOUT_SECS must be a positive number of seconds"))?;
        Ok(Some(timeout))
    }

    fn debug_enabled_from_env() -> bool {
        std::env::var("NATIVE_CORE_API_DEBUG").as_deref() == Ok("1")
    }
}