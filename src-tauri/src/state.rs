//! Shared application state, held behind Tauri's managed `State`.
//!
//! v1 uses a single SQLite connection behind a `Mutex` for simplicity. SQLite
//! serializes writes anyway, and the open-loops workload is light. If contention
//! ever shows up (e.g. long sync writes blocking UI reads), revisit with a pool
//! or a dedicated writer thread — do not pre-optimize.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::Result;

/// User-tunable settings that affect loop detection (spec §10).
#[derive(Debug, Clone)]
pub struct Config {
    /// Days a sent message sits unanswered before it counts as `waiting_on`.
    pub waiting_on_days: i64,
    /// Days an inbound message sits unanswered before it counts as `owe_reply`.
    pub owe_reply_grace_days: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            waiting_on_days: 3,
            owe_reply_grace_days: 1,
        }
    }
}

pub struct AppState {
    pub db: Mutex<Connection>,
    pub config: Mutex<Config>,
    /// Selected AI provider, if any. Empty by default — the app is fully
    /// functional with no provider configured (spec §3.3).
    pub ai: crate::ai::AiRegistry,
    /// Location of the SQLite file on disk; surfaced in diagnostics.
    pub db_path: PathBuf,
}

impl AppState {
    /// Open (creating if needed) the database at `db_path` and run migrations.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)?;
        // Pragmas: WAL for concurrent reads during a sync write; foreign keys on.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        crate::db::migrations::run(&conn)?;
        Ok(Self {
            db: Mutex::new(conn),
            config: Mutex::new(Config::default()),
            ai: crate::ai::AiRegistry::default(),
            db_path,
        })
    }
}
