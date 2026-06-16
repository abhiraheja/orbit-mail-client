//! Email sync engine. Runs in the background and emits `sync:*` events.
//!
//! The sync loop is written against the [`MailSource`] trait so it can be driven
//! by a real IMAP server in production and by in-memory fixtures in tests
//! (spec §12 — "make sync logic testable without a live server"). Fetched mail is
//! normalized and persisted by [`ingest`], which is what loop detection reads.

pub mod imap;
pub mod ingest;
pub mod runner;

use crate::error::Result;

/// A message as fetched from a mail source, before normalization/storage.
/// Threading and ownership are resolved later, during ingest.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// RFC 822 Message-ID (angle brackets stripped). Used for dedup + threading.
    pub message_id: Option<String>,
    /// Message-ID this is a direct reply to, if any.
    pub in_reply_to: Option<String>,
    /// Message-IDs referenced (the thread chain), oldest→newest.
    pub references: Vec<String>,
    pub from_email: String,
    pub from_name: Option<String>,
    /// Direct recipients (To:). Used for the owe_reply "I'm a direct recipient" test.
    pub to_emails: Vec<String>,
    /// Carbon-copy recipients (Cc:).
    pub cc_emails: Vec<String>,
    pub subject: Option<String>,
    pub body_text: Option<String>,
    /// UTC unix seconds the message was sent.
    pub sent_at: i64,
    /// Raw header hints that mark bulk/automated mail (List-Id, Precedence, etc.).
    pub bulk_headers: bool,
}

/// Which folder we're pulling from. Sent mail is `is_from_me` regardless of the
/// From address, so the source must tell ingest where a batch came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folder {
    Inbox,
    Sent,
}

impl Folder {
    pub fn as_str(self) -> &'static str {
        match self {
            Folder::Inbox => "INBOX",
            Folder::Sent => "Sent",
        }
    }
}

/// A batch of fetched messages plus the cursor needed for the next incremental pull.
pub struct FetchBatch {
    pub folder: Folder,
    pub messages: Vec<IncomingMessage>,
    /// Highest UID seen, to persist as the incremental sync cursor.
    pub last_uid: Option<i64>,
    pub uid_validity: Option<i64>,
}

/// Abstraction over a mail backend so the sync loop is testable without a server.
/// The real implementation wraps `async-imap`; tests use an in-memory fixture.
#[async_trait::async_trait]
pub trait MailSource: Send {
    /// Fetch messages newer than `since_uid` from `folder` (None = full pull).
    async fn fetch(&mut self, folder: Folder, since_uid: Option<i64>) -> Result<FetchBatch>;
}
