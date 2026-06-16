//! Sync orchestration: pull mail from a [`MailSource`], ingest it, run loop
//! detection, and report progress. Written against the trait so it runs against
//! a real IMAP server in production and an in-memory mock in tests.
//!
//! The connection is only locked for the (synchronous) ingest/detect steps, never
//! held across a network await, so the UI never blocks behind a fetch (spec §5).

use std::collections::HashSet;

use rusqlite::Connection;

use crate::db::queries;
use crate::error::Result;
use crate::loops::rules;
use crate::state::Config;
use crate::sync::{ingest::ingest_messages, Folder, MailSource};

/// Result of one sync pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct SyncOutcome {
    pub new_messages: usize,
    pub active_loops: i64,
}

/// Run one incremental sync of INBOX + Sent for an account. `progress` is invoked
/// after each folder with (done, total) message counts so the caller can emit
/// `sync:progress` events.
pub async fn sync_with_source(
    source: &mut dyn MailSource,
    conn: &std::sync::Mutex<Connection>,
    account_id: i64,
    cfg: &Config,
    now: i64,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<SyncOutcome> {
    let owner: HashSet<String> = {
        let c = conn.lock().expect("db mutex poisoned");
        queries::account_emails(&c)?.into_iter().collect()
    };

    let mut outcome = SyncOutcome::default();
    let folders = [Folder::Inbox, Folder::Sent];

    // First pass: learn totals so progress is meaningful. We fetch per folder and
    // ingest immediately (batched commit happens inside ingest's transaction).
    let mut done: u64 = 0;
    let mut batches = Vec::new();
    let mut total: u64 = 0;
    for folder in folders {
        let since = {
            let c = conn.lock().expect("db mutex poisoned");
            last_uid(&c, account_id, folder)?
        };
        let batch = source.fetch(folder, since).await?;
        total += batch.messages.len() as u64;
        batches.push(batch);
    }

    for batch in batches {
        let n = {
            let mut c = conn.lock().expect("db mutex poisoned");
            let n = ingest_messages(&mut c, account_id, &owner, batch.folder, &batch.messages, now)?;
            persist_cursor(&c, account_id, batch.folder, batch.uid_validity, batch.last_uid)?;
            n
        };
        outcome.new_messages += n;
        done += batch.messages.len() as u64;
        progress(done, total);
    }

    // Re-detect loops over the (now updated) mail and stamp last_synced.
    let active = {
        let mut c = conn.lock().expect("db mutex poisoned");
        let active = rules::detect_and_store(&mut c, cfg, now)?;
        queries::set_last_synced(&c, account_id, now)?;
        active
    };
    outcome.active_loops = active;
    Ok(outcome)
}

fn last_uid(conn: &Connection, account_id: i64, folder: Folder) -> Result<Option<i64>> {
    let v: Option<i64> = conn
        .query_row(
            "SELECT last_uid FROM sync_state WHERE account_id = ?1 AND folder = ?2",
            rusqlite::params![account_id, folder.as_str()],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    Ok(v)
}

fn persist_cursor(
    conn: &Connection,
    account_id: i64,
    folder: Folder,
    uid_validity: Option<i64>,
    last_uid: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_state (account_id, folder, uid_validity, last_uid)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, folder) DO UPDATE SET
            uid_validity = COALESCE(excluded.uid_validity, sync_state.uid_validity),
            last_uid = COALESCE(excluded.last_uid, sync_state.last_uid)",
        rusqlite::params![account_id, folder.as_str(), uid_validity, last_uid],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{FetchBatch, IncomingMessage};
    use std::sync::Mutex;

    const NOW: i64 = 1_000_000_000;
    const DAY: i64 = 86_400;

    /// In-memory mail source: hands back canned batches per folder.
    struct MockSource {
        inbox: Vec<IncomingMessage>,
        sent: Vec<IncomingMessage>,
    }

    #[async_trait::async_trait]
    impl MailSource for MockSource {
        async fn fetch(&mut self, folder: Folder, _since: Option<i64>) -> Result<FetchBatch> {
            let messages = match folder {
                Folder::Inbox => self.inbox.clone(),
                Folder::Sent => self.sent.clone(),
            };
            Ok(FetchBatch {
                folder,
                last_uid: Some(messages.len() as i64),
                uid_validity: Some(1),
                messages,
            })
        }
    }

    fn db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        queries::insert_account(&conn, "me@acme.com", None, "imap", "password", None, 0).unwrap();
        Mutex::new(conn)
    }

    fn outbound(id: &str, sent_at: i64) -> IncomingMessage {
        IncomingMessage {
            message_id: Some(id.into()),
            in_reply_to: None,
            references: vec![],
            from_email: "me@acme.com".into(),
            from_name: None,
            to_emails: vec!["john@x.com".into()],
            cc_emails: vec![],
            subject: Some("Pricing".into()),
            body_text: Some("here's the quote".into()),
            sent_at,
            bulk_headers: false,
        }
    }

    #[tokio::test]
    async fn full_slice_sync_detects_loops_and_reports_progress() {
        let conn = db();
        let mut source = MockSource {
            inbox: vec![],
            sent: vec![outbound("<a>", NOW - 5 * DAY)],
        };
        let mut progress_calls = Vec::new();
        let outcome = sync_with_source(
            &mut source,
            &conn,
            1,
            &Config::default(),
            NOW,
            &mut |done, total| progress_calls.push((done, total)),
        )
        .await
        .unwrap();

        assert_eq!(outcome.new_messages, 1);
        assert_eq!(outcome.active_loops, 1, "5-day-old sent mail → one waiting_on loop");
        assert!(!progress_calls.is_empty(), "progress should be reported");
        assert_eq!(progress_calls.last().unwrap().0, 1, "final done count == total");
    }

    #[tokio::test]
    async fn second_sync_dedups() {
        let conn = db();
        let mut source = MockSource {
            inbox: vec![],
            sent: vec![outbound("<a>", NOW - 5 * DAY)],
        };
        sync_with_source(&mut source, &conn, 1, &Config::default(), NOW, &mut |_, _| {})
            .await
            .unwrap();
        let second = sync_with_source(&mut source, &conn, 1, &Config::default(), NOW, &mut |_, _| {})
            .await
            .unwrap();
        assert_eq!(second.new_messages, 0, "same Message-ID is not re-ingested");
    }
}
