//! Provider-agnostic AI + the audit-log chokepoint (spec §3.3, §9).
//!
//! NOT on the v1 critical path: the app runs fully with no provider configured
//! (`AiRegistry::is_configured` is then false and AI commands return a clear
//! error, while heuristic loops keep working). The single rule that makes the
//! privacy promise enforceable — every outbound call writes `ai_audit_log`
//! *before* sending — lives in [`audit::complete`], the only way to reach a
//! provider.

pub mod audit;
pub mod openai;
pub mod provider;

use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use openai::OpenAiProvider;
use provider::{AiProvider, AiRequest, AiStream};

/// Persisted (non-secret) configuration of the active provider. The API key is
/// NEVER stored here — it lives in the OS keychain (spec §7). Serialized into the
/// `app_settings` table so the choice survives restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    /// Provider kind: 'openai' | 'openrouter' | 'deepseek' | 'azure' | 'ollama' |
    /// 'lmstudio' | 'custom'.
    pub kind: String,
    pub base_url: String,
    pub model: String,
    /// True for on-device endpoints — drives `was_local` in the audit log.
    pub local: bool,
}

impl AiConfig {
    /// Default base URL + locality for a known provider kind. `custom` has no
    /// default and requires an explicit base_url from the caller.
    pub fn defaults_for(kind: &str) -> Option<(&'static str, bool)> {
        match kind {
            "openai" => Some(("https://api.openai.com/v1", false)),
            "openrouter" => Some(("https://openrouter.ai/api/v1", false)),
            "deepseek" => Some(("https://api.deepseek.com/v1", false)),
            "ollama" => Some(("http://localhost:11434/v1", true)),
            "lmstudio" => Some(("http://localhost:1234/v1", true)),
            _ => None,
        }
    }

    /// Does this kind authenticate with an API key? Local servers don't.
    pub fn needs_key(&self) -> bool {
        !self.local
    }
}

/// Build a concrete provider from config + an optional key. All current kinds are
/// OpenAI-compatible, so they share one implementation (spec §9).
pub fn build_provider(config: &AiConfig, api_key: Option<String>) -> Arc<dyn AiProvider> {
    Arc::new(OpenAiProvider::new(
        config.kind.clone(),
        config.base_url.clone(),
        config.model.clone(),
        api_key,
        config.local,
    ))
}

/// Holds the currently-selected provider, if any. Default: none configured.
/// Providers are held behind `Arc` so the active one can be cloned out from under
/// the lock — we never hold the (non-Send) guard across an await.
#[derive(Default)]
pub struct AiRegistry {
    active: Mutex<Option<Arc<dyn AiProvider>>>,
}

impl AiRegistry {
    pub fn is_configured(&self) -> bool {
        self.active.lock().expect("ai mutex poisoned").is_some()
    }

    pub fn set(&self, provider: Arc<dyn AiProvider>) {
        *self.active.lock().expect("ai mutex poisoned") = Some(provider);
    }

    pub fn clear(&self) {
        *self.active.lock().expect("ai mutex poisoned") = None;
    }

    /// Run an audited completion against the active provider. Returns a clear
    /// error when none is configured rather than failing opaquely.
    pub async fn complete(
        &self,
        db: &Mutex<Connection>,
        now: i64,
        req: AiRequest,
    ) -> Result<AiStream> {
        let provider = {
            let guard = self.active.lock().expect("ai mutex poisoned");
            guard.clone()
        }
        .ok_or_else(|| AppError::Ai("no AI provider configured".into()))?;

        audit::complete(db, provider.as_ref(), now, req).await
    }
}
