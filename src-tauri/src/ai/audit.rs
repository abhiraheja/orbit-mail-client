//! The privacy chokepoint. EVERY outbound AI call goes through [`complete`],
//! which writes the audit row BEFORE contacting the provider. No other code path
//! may call `AiProvider::complete` directly (spec §3.3, §9).
//!
//! Writing-before-sending is the load-bearing guarantee: even if the provider
//! call fails or panics, the log already records that data was about to leave.

use std::sync::Mutex;

use rusqlite::Connection;

use crate::ai::provider::{AiProvider, AiRequest, AiStream};
use crate::db::queries;
use crate::error::Result;

/// Audited completion. Locks the DB only to write the audit row, then releases it
/// before the (async) provider call so a slow request never blocks the UI.
pub async fn complete(
    db: &Mutex<Connection>,
    provider: &dyn AiProvider,
    now: i64,
    req: AiRequest,
) -> Result<AiStream> {
    {
        let conn = db.lock().expect("db mutex poisoned");
        queries::insert_audit(
            &conn,
            now,
            provider.name(),
            Some(provider.model()),
            &req.purpose,
            &req.data_summary,
            provider.is_local(),
        )?;
    } // lock released before any network I/O

    provider.complete(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use futures::stream;
    use futures::StreamExt;

    fn db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        Mutex::new(conn)
    }

    fn audit_count(db: &Mutex<Connection>) -> i64 {
        db.lock()
            .unwrap()
            .query_row("SELECT count(*) FROM ai_audit_log", [], |r| r.get(0))
            .unwrap()
    }

    /// A provider that streams canned tokens, or fails on demand.
    struct FakeProvider {
        local: bool,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl AiProvider for FakeProvider {
        fn name(&self) -> &str { "fake" }
        fn model(&self) -> &str { "fake-1" }
        fn is_local(&self) -> bool { self.local }
        async fn complete(&self, _req: AiRequest) -> Result<AiStream> {
            if self.fail {
                return Err(AppError::Ai("provider exploded".into()));
            }
            let toks = vec![Ok("Hello".to_string()), Ok(" world".to_string())];
            Ok(Box::pin(stream::iter(toks)))
        }
    }

    fn req() -> AiRequest {
        AiRequest {
            purpose: "draft_reply".into(),
            system: None,
            prompt: "write a reply".into(),
            data_summary: "thread 'Pricing' (2 messages) + user instructions".into(),
            model: None,
        }
    }

    #[tokio::test]
    async fn streams_tokens_and_writes_audit() {
        let db = db();
        let p = FakeProvider { local: false, fail: false };
        let mut stream = complete(&db, &p, 100, req()).await.unwrap();
        let mut out = String::new();
        while let Some(tok) = stream.next().await {
            out.push_str(&tok.unwrap());
        }
        assert_eq!(out, "Hello world");
        assert_eq!(audit_count(&db), 1);
    }

    #[tokio::test]
    async fn audit_written_even_when_provider_fails() {
        // The chokepoint's whole point: the log records the attempt BEFORE the
        // provider is reached, so a failure can't hide that data was about to send.
        let db = db();
        let p = FakeProvider { local: false, fail: true };
        let result = complete(&db, &p, 100, req()).await;
        assert!(result.is_err());
        assert_eq!(audit_count(&db), 1, "audit row exists despite provider failure");
    }

    #[tokio::test]
    async fn local_provider_marked_was_local() {
        let db = db();
        let p = FakeProvider { local: true, fail: false };
        let _ = complete(&db, &p, 100, req()).await.unwrap();
        let was_local: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT was_local FROM ai_audit_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(was_local, 1, "on-device provider logged as local");
    }
}
