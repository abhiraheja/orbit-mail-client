//! The provider-agnostic AI interface. The rest of the app depends only on this
//! trait, never on a concrete provider (spec §9). Concrete HTTP providers
//! (OpenAI, Claude, Gemini, Ollama, …) implement `complete`; they are reachable
//! ONLY through the audited entry point in [`super::audit`].

use std::pin::Pin;

use futures::Stream;

use crate::error::Result;

/// A single outbound AI request. `data_summary` is the human-readable description
/// of what is being sent — it is what gets written to the audit log, so it must
/// honestly describe the payload (spec §3.3).
#[derive(Debug, Clone)]
pub struct AiRequest {
    /// Why we're calling: 'draft_reply' | 'detect_promise' | …
    pub purpose: String,
    pub system: Option<String>,
    pub prompt: String,
    /// Human-readable summary of the data leaving the machine, for the audit log.
    pub data_summary: String,
    /// Optional model override; otherwise the provider's default.
    pub model: Option<String>,
}

/// A streamed completion: tokens as they arrive, so drafting feels alive
/// (`ai:token` events, spec §5).
pub type AiStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;

#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    /// Provider identifier written to the audit log ('openai', 'ollama', …).
    fn name(&self) -> &str;
    /// Default model identifier for the audit log.
    fn model(&self) -> &str;
    /// True for on-device providers (Ollama/LM Studio): nothing leaves the machine.
    fn is_local(&self) -> bool;
    /// Produce a streamed completion. MUST be reached only via `audit::complete`,
    /// which writes the audit row first.
    async fn complete(&self, req: AiRequest) -> Result<AiStream>;
}
