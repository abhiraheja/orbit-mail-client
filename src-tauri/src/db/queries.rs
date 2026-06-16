//! Typed query functions. The rest of the app calls these; it never writes SQL.
//!
//! Ingest and loop detection contain the *logic*; every SQL string lives here so
//! the database remains the single module that touches SQLite (spec §6).

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::models::{Account, AuditEntry, Contact, LoopView};

// --- Schema / health --------------------------------------------------------

/// Current schema version (number of applied migrations).
pub fn schema_version(conn: &Connection) -> Result<i64> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(v)
}

// --- Accounts ---------------------------------------------------------------

pub fn insert_account(
    conn: &Connection,
    email: &str,
    display_name: Option<&str>,
    provider: &str,
    auth_kind: &str,
    cred_ref: Option<&str>,
    created_at: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO accounts (email, display_name, provider, auth_kind, cred_ref, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![email, display_name, provider, auth_kind, cred_ref, created_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, display_name, provider, auth_kind, last_synced, created_at
         FROM accounts ORDER BY created_at",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Account {
            id: r.get(0)?,
            email: r.get(1)?,
            display_name: r.get(2)?,
            provider: r.get(3)?,
            auth_kind: r.get(4)?,
            last_synced: r.get(5)?,
            created_at: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn account_emails(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT email FROM accounts")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn set_last_synced(conn: &Connection, account_id: i64, ts: i64) -> Result<()> {
    conn.execute(
        "UPDATE accounts SET last_synced = ?1 WHERE id = ?2",
        params![ts, account_id],
    )?;
    Ok(())
}

pub fn remove_account(conn: &Connection, account_id: i64) -> Result<()> {
    // Order matters: drop dependent rows before the account.
    conn.execute("DELETE FROM loops WHERE thread_id IN (SELECT id FROM threads WHERE account_id = ?1)", params![account_id])?;
    conn.execute("DELETE FROM messages WHERE account_id = ?1", params![account_id])?;
    conn.execute("DELETE FROM threads WHERE account_id = ?1", params![account_id])?;
    conn.execute("DELETE FROM sync_state WHERE account_id = ?1", params![account_id])?;
    conn.execute("DELETE FROM accounts WHERE id = ?1", params![account_id])?;
    Ok(())
}

// --- Contacts ---------------------------------------------------------------

/// Insert or update a contact by normalized email; returns its id. Keeps the
/// most recent `last_seen` and fills a display name if we didn't have one.
pub fn upsert_contact(
    conn: &Connection,
    email: &str,
    display_name: Option<&str>,
    seen_at: i64,
    created_at: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO contacts (email, display_name, last_seen, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(email) DO UPDATE SET
            last_seen = MAX(COALESCE(last_seen, 0), excluded.last_seen),
            display_name = COALESCE(contacts.display_name, excluded.display_name)",
        params![email, display_name, seen_at, created_at],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM contacts WHERE email = ?1",
        params![email],
        |r| r.get(0),
    )?;
    Ok(id)
}

pub fn find_contact_by_email(conn: &Connection, email: &str) -> Result<Option<i64>> {
    let id = conn
        .query_row("SELECT id FROM contacts WHERE email = ?1", params![email], |r| r.get(0))
        .optional()?;
    Ok(id)
}

pub fn list_contacts(conn: &Connection) -> Result<Vec<Contact>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, display_name, last_seen FROM contacts ORDER BY last_seen DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Contact {
            id: r.get(0)?,
            email: r.get(1)?,
            display_name: r.get(2)?,
            last_seen: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// --- Threads ----------------------------------------------------------------

/// Find the thread that contains a message with the given RFC Message-ID.
pub fn thread_of_message_id(
    conn: &Connection,
    account_id: i64,
    message_id: &str,
) -> Result<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT thread_id FROM messages WHERE account_id = ?1 AND message_id = ?2",
            params![account_id, message_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id)
}

/// Find an existing thread by normalized subject within an account.
pub fn thread_by_norm_subject(
    conn: &Connection,
    account_id: i64,
    norm_subject: &str,
) -> Result<Option<i64>> {
    if norm_subject.is_empty() {
        return Ok(None);
    }
    let id = conn
        .query_row(
            "SELECT id FROM threads WHERE account_id = ?1 AND norm_subject = ?2 LIMIT 1",
            params![account_id, norm_subject],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id)
}

pub fn create_thread(
    conn: &Connection,
    account_id: i64,
    subject: Option<&str>,
    norm_subject: &str,
    last_message: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO threads (account_id, subject, norm_subject, last_message)
         VALUES (?1, ?2, ?3, ?4)",
        params![account_id, subject, norm_subject, last_message],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn bump_thread_last_message(conn: &Connection, thread_id: i64, sent_at: i64) -> Result<()> {
    conn.execute(
        "UPDATE threads SET last_message = MAX(COALESCE(last_message, 0), ?2) WHERE id = ?1",
        params![thread_id, sent_at],
    )?;
    Ok(())
}

// --- Messages ---------------------------------------------------------------

pub fn message_exists(conn: &Connection, account_id: i64, message_id: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM messages WHERE account_id = ?1 AND message_id = ?2",
        params![account_id, message_id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

#[allow(clippy::too_many_arguments)]
pub fn insert_message(
    conn: &Connection,
    account_id: i64,
    thread_id: i64,
    message_id: Option<&str>,
    from_email: &str,
    to_emails_json: &str,
    subject: Option<&str>,
    snippet: Option<&str>,
    body_text: Option<&str>,
    sent_at: i64,
    is_from_me: bool,
    is_automated: bool,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO messages
            (account_id, thread_id, message_id, from_email, to_emails, subject,
             snippet, body_text, sent_at, is_from_me, is_automated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            account_id, thread_id, message_id, from_email, to_emails_json, subject,
            snippet, body_text, sent_at, is_from_me as i64, is_automated as i64
        ],
    )?;
    let id = conn.last_insert_rowid();
    // Keep the FTS index in step (contentless-external table).
    conn.execute(
        "INSERT INTO messages_fts (rowid, subject, body_text) VALUES (?1, ?2, ?3)",
        params![id, subject, body_text],
    )?;
    Ok(id)
}

/// Subject + all messages in a thread, oldest first, for the context view.
pub fn thread_view(conn: &Connection, thread_id: i64) -> Result<Option<crate::models::ThreadView>> {
    let subject: Option<Option<String>> = conn
        .query_row(
            "SELECT subject FROM threads WHERE id = ?1",
            params![thread_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(subject) = subject else { return Ok(None) };

    let mut stmt = conn.prepare(
        "SELECT id, account_id, thread_id, message_id, from_email, to_emails, subject,
                snippet, body_text, sent_at, is_from_me, is_automated
         FROM messages WHERE thread_id = ?1 ORDER BY sent_at ASC, id ASC",
    )?;
    let messages = stmt
        .query_map(params![thread_id], |r| {
            let to_json: Option<String> = r.get(5)?;
            Ok(crate::models::Message {
                id: r.get(0)?,
                account_id: r.get(1)?,
                thread_id: r.get(2)?,
                message_id: r.get(3)?,
                from_email: r.get(4)?,
                to_emails: to_json
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
                subject: r.get(6)?,
                snippet: r.get(7)?,
                body_text: r.get(8)?,
                sent_at: r.get(9)?,
                is_from_me: r.get::<_, i64>(10)? != 0,
                is_automated: r.get::<_, i64>(11)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Some(crate::models::ThreadView {
        id: thread_id,
        subject,
        messages,
    }))
}

/// Auth details for a sync: (email, provider, auth_kind, cred_ref).
pub fn account_auth(conn: &Connection, account_id: i64) -> Result<Option<(String, String, String, String)>> {
    let row = conn
        .query_row(
            "SELECT email, provider, auth_kind, COALESCE(cred_ref, '')
             FROM accounts WHERE id = ?1",
            params![account_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
        )
        .optional()?;
    Ok(row)
}

/// The keychain reference stored for an account (to load creds / clean up).
pub fn account_cred_ref(conn: &Connection, account_id: i64) -> Result<Option<(String, String)>> {
    let row = conn
        .query_row(
            "SELECT email, COALESCE(cred_ref, '') FROM accounts WHERE id = ?1",
            params![account_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(row)
}

// --- App settings (non-secret key/value) ------------------------------------

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let v = conn
        .query_row("SELECT value FROM app_settings WHERE key = ?1", params![key], |r| r.get(0))
        .optional()?;
    Ok(v)
}

pub fn delete_setting(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
    Ok(())
}

// --- Search -----------------------------------------------------------------

/// One message/thread hit from the FTS index. Caller turns it into a SearchResult.
pub struct MessageHit {
    pub thread_id: i64,
    pub subject: String,
    pub snippet: String,
    pub from_email: String,
}

/// Full-text search over message subject/body, collapsed to one hit per thread
/// and ordered by FTS relevance. `fts_query` is an FTS5 MATCH expression built by
/// the search module — never raw user input.
pub fn search_messages(conn: &Connection, fts_query: &str, limit: i64) -> Result<Vec<MessageHit>> {
    let mut stmt = conn.prepare(
        "SELECT m.thread_id,
                COALESCE(m.subject, '(no subject)'),
                COALESCE(m.snippet, ''),
                m.from_email
         FROM messages_fts
         JOIN messages m ON m.id = messages_fts.rowid
         WHERE messages_fts MATCH ?1 AND m.thread_id IS NOT NULL
         GROUP BY m.thread_id
         ORDER BY MIN(messages_fts.rank)
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![fts_query, limit], |r| {
        Ok(MessageHit {
            thread_id: r.get(0)?,
            subject: r.get(1)?,
            snippet: r.get(2)?,
            from_email: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// One contact hit. `like` is a `%…%` pattern built by the search module.
pub struct ContactHit {
    pub email: String,
    pub display: String,
}

pub fn search_contacts(conn: &Connection, like: &str, limit: i64) -> Result<Vec<ContactHit>> {
    let mut stmt = conn.prepare(
        "SELECT email, COALESCE(display_name, email)
         FROM contacts
         WHERE email LIKE ?1 ESCAPE '\\' OR display_name LIKE ?1 ESCAPE '\\'
         ORDER BY last_seen DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![like, limit], |r| {
        Ok(ContactHit { email: r.get(0)?, display: r.get(1)? })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// --- AI audit log (the privacy chokepoint, spec §3.3) -----------------------

/// Record one outbound AI call. Written BEFORE the provider is contacted so the
/// log can never under-report what left the machine.
pub fn insert_audit(
    conn: &Connection,
    timestamp: i64,
    provider: &str,
    model: Option<&str>,
    purpose: &str,
    data_summary: &str,
    was_local: bool,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO ai_audit_log
            (timestamp, provider, model, purpose, data_summary, was_local)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![timestamp, provider, model, purpose, data_summary, was_local as i64],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_audit(conn: &Connection) -> Result<Vec<AuditEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp, provider, model, purpose, data_summary, was_local
         FROM ai_audit_log ORDER BY timestamp DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(AuditEntry {
            id: r.get(0)?,
            timestamp: r.get(1)?,
            provider: r.get(2)?,
            model: r.get(3)?,
            purpose: r.get(4)?,
            data_summary: r.get(5)?,
            was_local: r.get::<_, i64>(6)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// --- Loop detection inputs --------------------------------------------------

/// The newest message in a thread — the "tip" the kind-1/2 rules hinge on.
#[derive(Debug, Clone)]
pub struct ThreadTip {
    pub thread_id: i64,
    pub message_id: i64,
    pub from_email: String,
    pub to_emails: String, // JSON array
    pub subject: Option<String>,
    pub body_text: Option<String>,
    pub sent_at: i64,
    pub is_from_me: bool,
    pub is_automated: bool,
}

/// One row per thread: the newest message by `sent_at` (id breaks ties).
pub fn thread_tips(conn: &Connection) -> Result<Vec<ThreadTip>> {
    let mut stmt = conn.prepare(
        "SELECT m.thread_id, m.id, m.from_email, m.to_emails, m.subject, m.body_text,
                m.sent_at, m.is_from_me, m.is_automated
         FROM messages m
         JOIN (
            SELECT thread_id, MAX(sent_at) AS mx FROM messages GROUP BY thread_id
         ) t ON t.thread_id = m.thread_id AND t.mx = m.sent_at
         GROUP BY m.thread_id
         HAVING m.id = MAX(m.id)",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ThreadTip {
            thread_id: r.get(0)?,
            message_id: r.get(1)?,
            from_email: r.get(2)?,
            to_emails: r.get::<_, Option<String>>(3)?.unwrap_or_else(|| "[]".into()),
            subject: r.get(4)?,
            body_text: r.get(5)?,
            sent_at: r.get(6)?,
            is_from_me: r.get::<_, i64>(7)? != 0,
            is_automated: r.get::<_, i64>(8)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// My own sent messages, for the kind-3 "promised" scan.
#[derive(Debug, Clone)]
pub struct SentMessage {
    pub message_id: i64,
    pub thread_id: i64,
    pub to_emails: String,
    pub subject: Option<String>,
    pub body_text: Option<String>,
    pub sent_at: i64,
}

pub fn sent_messages(conn: &Connection) -> Result<Vec<SentMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, to_emails, subject, body_text, sent_at
         FROM messages WHERE is_from_me = 1 AND thread_id IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SentMessage {
            message_id: r.get(0)?,
            thread_id: r.get(1)?,
            to_emails: r.get::<_, Option<String>>(2)?.unwrap_or_else(|| "[]".into()),
            subject: r.get(3)?,
            body_text: r.get(4)?,
            sent_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// --- Loop persistence -------------------------------------------------------

/// Existing loop keyed by (kind, thread_id), for lifecycle reconciliation.
#[derive(Debug, Clone)]
pub struct ExistingLoop {
    pub id: i64,
    pub kind: String,
    pub thread_id: Option<i64>,
    pub status: String,
}

pub fn existing_loops(conn: &Connection) -> Result<Vec<ExistingLoop>> {
    let mut stmt =
        conn.prepare("SELECT id, kind, thread_id, status FROM loops")?;
    let rows = stmt.query_map([], |r| {
        Ok(ExistingLoop {
            id: r.get(0)?,
            kind: r.get(1)?,
            thread_id: r.get(2)?,
            status: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[allow(clippy::too_many_arguments)]
pub fn insert_loop(
    conn: &Connection,
    kind: &str,
    thread_id: i64,
    contact_id: Option<i64>,
    message_id: i64,
    detected_at: i64,
    age_anchor: i64,
    confidence: f64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO loops
            (kind, thread_id, contact_id, message_id, detected_at, age_anchor, status, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7)",
        params![kind, thread_id, contact_id, message_id, detected_at, age_anchor, confidence],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn resolve_loop(conn: &Connection, loop_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE loops SET status = 'resolved' WHERE id = ?1 AND status IN ('open','snoozed')",
        params![loop_id],
    )?;
    Ok(())
}

pub fn snooze_loop(conn: &Connection, loop_id: i64, until: i64) -> Result<()> {
    conn.execute(
        "UPDATE loops SET status = 'snoozed', snoozed_until = ?2 WHERE id = ?1",
        params![loop_id, until],
    )?;
    Ok(())
}

pub fn dismiss_loop(conn: &Connection, loop_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE loops SET status = 'dismissed' WHERE id = ?1",
        params![loop_id],
    )?;
    Ok(())
}

/// Count of currently-actionable loops (open, or snoozed past their wake time).
pub fn count_active_loops(conn: &Connection, now: i64) -> Result<i64> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM loops
         WHERE status = 'open' OR (status = 'snoozed' AND snoozed_until <= ?1)",
        params![now],
        |r| r.get(0),
    )?;
    Ok(n)
}

/// Counts of currently-actionable loops broken down by kind, for the briefing.
/// Same "active" predicate as [`count_active_loops`].
pub fn active_loop_counts(conn: &Connection, now: i64) -> Result<LoopKindCounts> {
    let mut stmt = conn.prepare(
        "SELECT kind, count(*) FROM loops
         WHERE status = 'open' OR (status = 'snoozed' AND snoozed_until <= ?1)
         GROUP BY kind",
    )?;
    let mut counts = LoopKindCounts::default();
    let rows = stmt.query_map(params![now], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (kind, n) = row?;
        match kind.as_str() {
            "waiting_on" => counts.waiting_on = n,
            "owe_reply" => counts.owe_reply = n,
            "promised" => counts.promised = n,
            _ => {}
        }
    }
    Ok(counts)
}

/// Active-loop tallies by kind. The `total` is derived by the caller.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoopKindCounts {
    pub waiting_on: i64,
    pub owe_reply: i64,
    pub promised: i64,
}

/// Most recent `last_synced` across all accounts (None if never synced).
pub fn latest_sync(conn: &Connection) -> Result<Option<i64>> {
    let ts: Option<i64> = conn.query_row(
        "SELECT MAX(last_synced) FROM accounts",
        [],
        |r| r.get(0),
    )?;
    Ok(ts)
}

/// Number of configured accounts.
pub fn count_accounts(conn: &Connection) -> Result<i64> {
    let n: i64 = conn.query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))?;
    Ok(n)
}

/// Display-ready loops for the main screen (spec §11): contact name/email,
/// subject, and a pre-rendered human age string so the frontend computes nothing.
pub fn list_loop_views(
    conn: &Connection,
    now: i64,
    kind_filter: Option<&str>,
) -> Result<Vec<LoopViewRow>> {
    let mut sql = String::from(
        "SELECT l.id, l.kind, l.thread_id, l.age_anchor, l.confidence,
                COALESCE(c.display_name, c.email, '(unknown)') AS contact_name,
                COALESCE(c.email, '') AS contact_email,
                COALESCE(t.subject, '(no subject)') AS subject
         FROM loops l
         LEFT JOIN contacts c ON c.id = l.contact_id
         LEFT JOIN threads t ON t.id = l.thread_id
         WHERE (l.status = 'open' OR (l.status = 'snoozed' AND l.snoozed_until <= ?1))",
    );
    if kind_filter.is_some() {
        sql.push_str(" AND l.kind = ?2");
    }
    sql.push_str(" ORDER BY l.age_anchor ASC");

    let mut stmt = conn.prepare(&sql)?;
    let map = |r: &rusqlite::Row| -> rusqlite::Result<LoopViewRow> {
        Ok(LoopViewRow {
            id: r.get(0)?,
            kind: r.get(1)?,
            thread_id: r.get(2)?,
            age_anchor: r.get(3)?,
            confidence: r.get(4)?,
            contact_name: r.get(5)?,
            contact_email: r.get(6)?,
            subject: r.get(7)?,
        })
    };
    let rows: Vec<LoopViewRow> = match kind_filter {
        Some(k) => stmt
            .query_map(params![now, k], map)?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        None => stmt
            .query_map(params![now], map)?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    };
    Ok(rows)
}

/// Raw loop row before the human-age string is rendered (done in the loops module).
pub struct LoopViewRow {
    pub id: i64,
    pub kind: String,
    pub thread_id: i64,
    pub age_anchor: i64,
    pub confidence: f64,
    pub contact_name: String,
    pub contact_email: String,
    pub subject: String,
}

impl LoopViewRow {
    pub fn into_view(self, kind: crate::models::LoopKind, age: String) -> LoopView {
        LoopView {
            id: self.id,
            kind,
            contact_name: self.contact_name,
            contact_email: self.contact_email,
            subject: self.subject,
            age,
            thread_id: self.thread_id,
            confidence: self.confidence,
        }
    }
}
