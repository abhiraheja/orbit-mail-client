//! Typed event names + emit helpers.
//!
//! Events are how Rust pushes to the frontend unprompted (sync progress, loop
//! updates, streamed AI tokens). Every event name in the IPC contract (spec §11)
//! lives here as a constant so the string is defined exactly once and the
//! frontend's listener names can be kept in sync with these.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

// --- Event name constants ---------------------------------------------------

pub const SYNC_PROGRESS: &str = "sync:progress";
pub const SYNC_COMPLETE: &str = "sync:complete";
pub const SYNC_ERROR: &str = "sync:error";
pub const LOOPS_UPDATED: &str = "loops:updated";
pub const AI_TOKEN: &str = "ai:token";
pub const AI_DONE: &str = "ai:done";

// --- Payloads ---------------------------------------------------------------

#[derive(Clone, Serialize)]
pub struct SyncProgress {
    pub account_id: i64,
    pub done: u64,
    pub total: u64,
}

#[derive(Clone, Serialize)]
pub struct SyncComplete {
    pub account_id: i64,
    pub new_messages: u64,
}

#[derive(Clone, Serialize)]
pub struct SyncError {
    pub account_id: i64,
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct LoopsUpdated {
    pub count: u64,
}

#[derive(Clone, Serialize)]
pub struct AiToken {
    pub request_id: String,
    pub token: String,
}

#[derive(Clone, Serialize)]
pub struct AiDone {
    pub request_id: String,
}

// --- Emit helpers -----------------------------------------------------------
//
// Helpers swallow emit errors deliberately: a failed UI notification must never
// crash a background job. They log instead.

fn emit<P: Serialize + Clone>(app: &AppHandle, name: &str, payload: P) {
    if let Err(e) = app.emit(name, payload) {
        log::warn!("failed to emit {name}: {e}");
    }
}

pub fn sync_progress(app: &AppHandle, payload: SyncProgress) {
    emit(app, SYNC_PROGRESS, payload);
}

pub fn sync_complete(app: &AppHandle, payload: SyncComplete) {
    emit(app, SYNC_COMPLETE, payload);
}

pub fn sync_error(app: &AppHandle, payload: SyncError) {
    emit(app, SYNC_ERROR, payload);
}

pub fn loops_updated(app: &AppHandle, payload: LoopsUpdated) {
    emit(app, LOOPS_UPDATED, payload);
}

pub fn ai_token(app: &AppHandle, payload: AiToken) {
    emit(app, AI_TOKEN, payload);
}

pub fn ai_done(app: &AppHandle, payload: AiDone) {
    emit(app, AI_DONE, payload);
}
