//! OpenAI-compatible chat provider. One implementation covers OpenAI, Azure
//! OpenAI, OpenRouter, DeepSeek, and local servers (Ollama, LM Studio) — they all
//! speak the `/chat/completions` streaming protocol. The only differences are the
//! `base_url`, model name, and whether an API key / local flag applies.
//!
//! Reachable ONLY through [`super::audit::complete`]; this type never writes to
//! the audit log itself.

use futures::channel::mpsc;
use futures::StreamExt;
use serde_json::json;

use crate::ai::provider::{AiProvider, AiRequest, AiStream};
use crate::error::{AppError, Result};

/// A configured OpenAI-compatible endpoint.
pub struct OpenAiProvider {
    client: reqwest::Client,
    /// Audit-log identifier: 'openai' | 'openrouter' | 'ollama' | …
    name: String,
    /// API root, e.g. `https://api.openai.com/v1` or `http://localhost:11434/v1`.
    base_url: String,
    model: String,
    /// None for keyless local servers (Ollama/LM Studio).
    api_key: Option<String>,
    /// True when the endpoint runs on-device — nothing leaves the machine.
    local: bool,
}

impl OpenAiProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
        local: bool,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            name: name.into(),
            base_url: base_url.into(),
            model: model.into(),
            api_key,
            local,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait::async_trait]
impl AiProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn is_local(&self) -> bool {
        self.local
    }

    async fn complete(&self, req: AiRequest) -> Result<AiStream> {
        let model = req.model.clone().unwrap_or_else(|| self.model.clone());

        let mut messages = Vec::new();
        if let Some(system) = &req.system {
            messages.push(json!({ "role": "system", "content": system }));
        }
        messages.push(json!({ "role": "user", "content": req.prompt }));
        let body = json!({ "model": model, "stream": true, "messages": messages });

        let mut request = self.client.post(self.endpoint()).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let resp = request
            .send()
            .await
            .map_err(|e| AppError::Ai(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(AppError::Ai(format!("provider returned {status}: {detail}")));
        }

        // Read the SSE body off-thread, pushing decoded tokens through a channel so
        // the caller gets a clean `Stream<Item = Result<String>>`.
        let (tx, rx) = mpsc::unbounded::<Result<String>>();
        tokio::spawn(async move {
            let mut bytes = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = bytes.next().await {
                match chunk {
                    Ok(part) => {
                        buf.push_str(&String::from_utf8_lossy(&part));
                        // Process whole lines; keep any partial tail in `buf`.
                        while let Some(nl) = buf.find('\n') {
                            let line: String = buf.drain(..=nl).collect();
                            match parse_sse_line(line.trim()) {
                                SseLine::Token(t) if !t.is_empty() => {
                                    if tx.unbounded_send(Ok(t)).is_err() {
                                        return; // receiver dropped
                                    }
                                }
                                SseLine::Done => return,
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(Err(AppError::Ai(format!("stream error: {e}"))));
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(rx))
    }
}

/// One decoded SSE line.
#[derive(Debug, PartialEq, Eq)]
enum SseLine {
    /// A content delta.
    Token(String),
    /// The terminating `data: [DONE]`.
    Done,
    /// Comment, blank, or a non-content event we ignore.
    Ignore,
}

/// Decode a single Server-Sent-Events line from a `/chat/completions` stream.
/// Pulls `choices[0].delta.content` out of each `data:` payload.
fn parse_sse_line(line: &str) -> SseLine {
    let Some(data) = line.strip_prefix("data:") else {
        return SseLine::Ignore;
    };
    let data = data.trim();
    if data == "[DONE]" {
        return SseLine::Done;
    }
    match serde_json::from_str::<serde_json::Value>(data) {
        Ok(v) => v["choices"][0]["delta"]["content"]
            .as_str()
            .map(|s| SseLine::Token(s.to_string()))
            .unwrap_or(SseLine::Ignore),
        Err(_) => SseLine::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_delta() {
        let line = r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#;
        assert_eq!(parse_sse_line(line), SseLine::Token("Hello".into()));
    }

    #[test]
    fn recognizes_done_sentinel() {
        assert_eq!(parse_sse_line("data: [DONE]"), SseLine::Done);
    }

    #[test]
    fn ignores_comments_and_roles() {
        assert_eq!(parse_sse_line(": keep-alive"), SseLine::Ignore);
        assert_eq!(parse_sse_line(""), SseLine::Ignore);
        // A role-only delta (start of stream) carries no content.
        let role = r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#;
        assert_eq!(parse_sse_line(role), SseLine::Ignore);
    }

    #[test]
    fn endpoint_normalizes_trailing_slash() {
        let p = OpenAiProvider::new("openai", "https://api.openai.com/v1/", "gpt-x", None, false);
        assert_eq!(p.endpoint(), "https://api.openai.com/v1/chat/completions");
    }
}
