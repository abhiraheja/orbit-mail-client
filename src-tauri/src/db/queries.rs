//! Typed query functions. The rest of the app calls these; it never writes SQL.
//!
//! Phase 1 fills this in (message/contact upserts, loop reads/writes). For
//! Phase 0 we expose only what the bridge smoke-test needs.

use rusqlite::Connection;

use crate::error::Result;

/// Current schema version (number of applied migrations). Used by the Phase 0
/// bridge test to prove the DB is live and migrated.
pub fn schema_version(conn: &Connection) -> Result<i64> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(v)
}
