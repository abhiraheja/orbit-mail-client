//! THIN Tauri command handlers. They parse input, call a domain module, and
//! return — no business logic lives here (spec §6).
//!
//! Phase 0 exposes only the bridge smoke-test: a request→response command and a
//! command that triggers a one-shot event, proving both IPC patterns work.

use tauri::{AppHandle, State};

use crate::db::queries;
use crate::error::Result;
use crate::events::{self, LoopsUpdated};
use crate::state::AppState;

/// Request→response: returns the live schema version, proving the DB is open and
/// migrated and that a value can cross the bridge.
#[tauri::command]
pub fn ping(state: State<'_, AppState>) -> Result<i64> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::schema_version(&conn)
}

/// Event smoke-test: emits a `loops:updated` event so the frontend can prove it
/// receives Rust-pushed events. Real emission happens after sync in Phase 1.
#[tauri::command]
pub fn emit_test_event(app: AppHandle) -> Result<()> {
    events::loops_updated(&app, LoopsUpdated { count: 0 });
    Ok(())
}
