//! Schema versioning. Migrations run on startup, in order, exactly once each.
//!
//! `user_version` (a SQLite PRAGMA) records how many migrations have been
//! applied. To evolve the schema, append a new `&str` to `MIGRATIONS` — never
//! edit an existing entry, or installed databases will diverge.

use rusqlite::Connection;

use crate::error::Result;

/// Ordered DDL statements. Index + 1 is the resulting `user_version`.
const MIGRATIONS: &[&str] = &[
    // --- v1: the open-loops wedge schema (spec §7) -------------------------
    r#"
    -- Connected email accounts. Credentials live in the OS keychain, not here.
    CREATE TABLE accounts (
        id            INTEGER PRIMARY KEY,
        email         TEXT NOT NULL UNIQUE,
        display_name  TEXT,
        provider      TEXT NOT NULL,        -- 'gmail' | 'm365' | 'imap'
        auth_kind     TEXT NOT NULL,        -- 'oauth' | 'password'
        cred_ref      TEXT,                 -- keychain reference, NOT the secret
        last_synced   INTEGER,              -- UTC unix seconds
        created_at    INTEGER NOT NULL
    );

    -- People. The seed of the future knowledge graph.
    CREATE TABLE contacts (
        id            INTEGER PRIMARY KEY,
        email         TEXT NOT NULL UNIQUE, -- normalized lowercase
        display_name  TEXT,
        last_seen     INTEGER,
        created_at    INTEGER NOT NULL
    );

    -- Threads (conversations).
    CREATE TABLE threads (
        id            INTEGER PRIMARY KEY,
        account_id    INTEGER NOT NULL REFERENCES accounts(id),
        subject       TEXT,
        norm_subject  TEXT,                 -- subject with re:/fwd: stripped, for grouping
        last_message  INTEGER,              -- UTC unix seconds of newest message
        UNIQUE(account_id, id)
    );

    CREATE INDEX idx_threads_norm_subject ON threads(account_id, norm_subject);

    -- Individual messages.
    CREATE TABLE messages (
        id            INTEGER PRIMARY KEY,
        account_id    INTEGER NOT NULL REFERENCES accounts(id),
        thread_id     INTEGER REFERENCES threads(id),
        message_id    TEXT,                 -- RFC 822 Message-ID, for dedup
        from_email    TEXT NOT NULL,        -- normalized lowercase
        to_emails     TEXT,                 -- JSON array of normalized addresses
        subject       TEXT,
        snippet       TEXT,
        body_text     TEXT,
        sent_at       INTEGER NOT NULL,     -- UTC unix seconds
        is_from_me    INTEGER NOT NULL,     -- 1 if sent by an owned account
        is_automated  INTEGER NOT NULL DEFAULT 0,
        UNIQUE(account_id, message_id)
    );

    -- Detected open loops.
    CREATE TABLE loops (
        id            INTEGER PRIMARY KEY,
        kind          TEXT NOT NULL,        -- 'waiting_on' | 'owe_reply' | 'promised'
        thread_id     INTEGER REFERENCES threads(id),
        contact_id    INTEGER REFERENCES contacts(id),
        message_id    INTEGER REFERENCES messages(id),
        detected_at   INTEGER NOT NULL,
        age_anchor    INTEGER NOT NULL,
        status        TEXT NOT NULL DEFAULT 'open', -- open|snoozed|dismissed|resolved
        snoozed_until INTEGER,
        confidence    REAL DEFAULT 1.0
    );

    -- The privacy chokepoint: every outbound AI call is logged here.
    CREATE TABLE ai_audit_log (
        id            INTEGER PRIMARY KEY,
        timestamp     INTEGER NOT NULL,
        provider      TEXT NOT NULL,
        model         TEXT,
        purpose       TEXT NOT NULL,
        data_summary  TEXT NOT NULL,
        was_local     INTEGER NOT NULL
    );

    -- Full-text search over messages (FTS5, contentless-external).
    CREATE VIRTUAL TABLE messages_fts USING fts5(
        subject, body_text, content='messages', content_rowid='id'
    );

    -- Per-account incremental sync state (UID validity / last seen UID per folder).
    CREATE TABLE sync_state (
        account_id    INTEGER NOT NULL REFERENCES accounts(id),
        folder        TEXT NOT NULL,        -- e.g. 'INBOX', 'Sent'
        uid_validity  INTEGER,
        last_uid      INTEGER,
        PRIMARY KEY (account_id, folder)
    );

    -- Helpful indexes for loop detection (newest-message-per-thread scans).
    CREATE INDEX idx_messages_thread ON messages(thread_id, sent_at);
    CREATE INDEX idx_messages_account ON messages(account_id);
    CREATE INDEX idx_loops_status ON loops(status, kind);
    "#,
    // --- v2: key/value app settings (e.g. the selected AI provider config) ---
    // Non-secret only. API keys/tokens still live in the OS keychain.
    r#"
    CREATE TABLE app_settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
];

/// Apply any migrations newer than the database's current `user_version`.
pub fn run(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let current = current as usize;

    for (i, ddl) in MIGRATIONS.iter().enumerate().skip(current) {
        let version = i + 1;
        log::info!("applying migration {version}");
        conn.execute_batch(ddl)?;
        // user_version takes a literal, not a bind parameter.
        conn.pragma_update(None, "user_version", version as i64)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_and_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        // Running again is a no-op (version already current).
        run(&conn).unwrap();

        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());

        // All v1 tables exist.
        for table in [
            "accounts", "contacts", "threads", "messages", "loops",
            "ai_audit_log", "messages_fts", "sync_state",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "table {table} should exist");
        }
    }
}
