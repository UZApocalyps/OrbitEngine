//! Text generation: the [`TextModel`] trait and its OpenAI-compatible client.

use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::http::Request;
use crate::settings::TextSettings;

/// Why a provider call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    /// Network / HTTP failure (message for the user).
    Http(String),
    /// The answer did not have the expected shape.
    BadResponse(String),
    Cancelled,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Http(m) => write!(f, "request failed: {m}"),
            ProviderError::BadResponse(m) => write!(f, "unexpected answer: {m}"),
            ProviderError::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for ProviderError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(c: impl Into<String>) -> Self {
        Self { role: Role::System, content: c.into() }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self { role: Role::User, content: c.into() }
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: c.into() }
    }
}

/// Something that completes a conversation. Implemented by the HTTP client
/// and by test mocks.
pub trait TextModel: Send + Sync {
    fn complete(&self, messages: &[Message]) -> Result<String, ProviderError>;
}

pub(crate) fn join_url(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// Sends a request and reads a JSON answer, turning an HTTP error status
/// into a readable message.
pub(crate) fn send_json(req: &Request<'_>) -> Result<Json, ProviderError> {
    let resp = req.send().map_err(ProviderError::Http)?;
    let text = resp.text();
    if !(200..300).contains(&resp.status) {
        let short: String = text.chars().take(400).collect();
        return Err(ProviderError::Http(format!("HTTP {}: {short}", resp.status)));
    }
    serde_json::from_str(&text).map_err(|e| ProviderError::BadResponse(format!("{e}")))
}

/// OpenAI-compatible `POST {base}/chat/completions` (non-streaming).
pub struct OpenAiChat {
    settings: TextSettings,
}

impl OpenAiChat {
    pub fn new(settings: TextSettings) -> Self {
        Self { settings }
    }

    /// `GET {base}/models`: the model ids the server offers (to fill a
    /// picker, and to test the connection).
    pub fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let url = join_url(&self.settings.base_url, "models");
        let json = send_json(&Request::get(&url, Duration::from_secs(20)).bearer(&self.settings.api_key))?;
        Ok(json["data"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect())
            .unwrap_or_default())
    }
}

impl TextModel for OpenAiChat {
    fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        let messages: Vec<Json> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                json!({"role": role, "content": m.content})
            })
            .collect();
        let body = json!({
            "model": self.settings.model,
            "messages": messages,
            "temperature": self.settings.temperature,
            "max_tokens": self.settings.max_tokens,
            "stream": false,
        });
        let url = join_url(&self.settings.base_url, "chat/completions");
        let timeout = Duration::from_secs(self.settings.timeout_s.max(5));
        let json = send_json(&Request::post_json(&url, &body, timeout).bearer(&self.settings.api_key))?;
        let content = &json["choices"][0]["message"]["content"];
        match content.as_str() {
            Some(text) => Ok(strip_think(text)),
            None => Err(ProviderError::BadResponse(
                "no choices[0].message.content in the answer".to_owned(),
            )),
        }
    }
}

/// Removes `<think>…</think>` blocks that reasoning models put before
/// their answer.
pub fn strip_think(text: &str) -> String {
    let mut out = text.to_owned();
    while let Some(start) = out.find("<think>") {
        match out[start..].find("</think>") {
            Some(end) => out.replace_range(start..start + end + "</think>".len(), ""),
            None => {
                out.truncate(start);
                break;
            }
        }
    }
    out.trim().to_owned()
}
