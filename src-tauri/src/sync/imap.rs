//! Concrete `MailSource` backed by `async-imap` (app-password auth first, spec §8).
//!
//! NOTE: the live fetch is a stub pending a test account. The sync *orchestration*
//! (fetch → ingest → detect → events, in `runner`) is complete and exercised by a
//! mock source in tests. Wiring the real IMAP fetch is the next step once an
//! account is available to verify incremental pulls, reconnects, and rate limits
//! against a real server — exactly the part the spec warns must be proven live.

use crate::error::{AppError, Result};
use crate::sync::{FetchBatch, Folder, MailSource};

/// Connection parameters for a plain-IMAP account.
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub email: String,
    pub password: String,
}

pub struct ImapSource {
    #[allow(dead_code)]
    cfg: ImapConfig,
}

impl ImapSource {
    pub fn new(cfg: ImapConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait::async_trait]
impl MailSource for ImapSource {
    async fn fetch(&mut self, _folder: Folder, _since_uid: Option<i64>) -> Result<FetchBatch> {
        // TODO(live-account): open TLS, login, SELECT folder, UID-FETCH new
        // messages, parse with `mail-parser`, map into IncomingMessage. Persist
        // UIDVALIDITY/last_uid as the incremental cursor.
        Err(AppError::Sync(
            "live IMAP fetch not yet wired — pending a test account".into(),
        ))
    }
}
