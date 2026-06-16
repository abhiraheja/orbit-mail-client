//! Provider-agnostic AI + the audit-log chokepoint (spec §3.3, §9).
//!
//! NOT on the v1 critical path: the app runs fully with no provider configured
//! (`AiRegistry::is_configured` is then false and AI commands return a clear
//! error, while heuristic loops keep working). The single rule that makes the
//! privacy promise enforceable — every outbound call writes `ai_audit_log`
//! *before* sending — lives in [`audit::complete`], the only way to reach a
//! provider.

pub mod audit;
pub mod provider;

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, Result};
use provider::{AiProvider, AiRequest, AiStream};

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
