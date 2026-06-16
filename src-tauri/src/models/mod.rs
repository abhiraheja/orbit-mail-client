//! Domain types shared across modules and serialized across the IPC bridge.
//!
//! `*View` types are display-ready: the frontend renders them directly and does
//! zero computation (spec §11).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String, // 'gmail' | 'm365' | 'imap'
    pub auth_kind: String, // 'oauth' | 'password'
    pub last_synced: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub account_id: i64,
    pub thread_id: Option<i64>,
    pub message_id: Option<String>,
    pub from_email: String,
    pub to_emails: Vec<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub body_text: Option<String>,
    pub sent_at: i64,
    pub is_from_me: bool,
    pub is_automated: bool,
}

/// The three kinds of open loop (spec §2, §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    WaitingOn,
    OweReply,
    Promised,
}

impl LoopKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LoopKind::WaitingOn => "waiting_on",
            LoopKind::OweReply => "owe_reply",
            LoopKind::Promised => "promised",
        }
    }

    /// Parse a DB `kind` string. Unknown values fall back to `Promised` (the
    /// lowest-confidence kind) rather than failing — kinds are a closed set we
    /// write ourselves, so this only guards against future/typo'd rows.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "waiting_on" => LoopKind::WaitingOn,
            "owe_reply" => LoopKind::OweReply,
            _ => LoopKind::Promised,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopStatus {
    Open,
    Snoozed,
    Dismissed,
    Resolved,
}

/// Display-ready loop for the main screen. No raw timestamps that need
/// formatting — `age` is a pre-rendered human string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopView {
    pub id: i64,
    pub kind: LoopKind,
    pub contact_name: String,
    pub contact_email: String,
    pub subject: String,
    pub age: String, // e.g. "5 days"
    pub thread_id: i64,
    pub confidence: f64,
}

/// The daily briefing — a display-ready snapshot of where things stand (spec
/// §14, Phase 2). Every string is pre-rendered; the frontend renders verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingView {
    /// One-line summary, e.g. "7 open loops — 2 to reply, 4 waiting, 1 promised".
    pub headline: String,
    pub total_active: i64,
    pub waiting_on: i64,
    pub owe_reply: i64,
    pub promised: i64,
    pub account_count: i64,
    /// Human "last synced" string, e.g. "5 minutes ago", or None if never synced.
    pub last_synced: Option<String>,
    /// The most urgent loops (oldest first), capped — the "do these first" list.
    pub top_loops: Vec<LoopView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadView {
    pub id: i64,
    pub subject: Option<String>,
    pub messages: Vec<Message>,
}

/// What a search hit points at. Drives the icon and the open action in the
/// Ctrl+K palette (spec §6 — search is Rust's job, the frontend only renders).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Thread,
    Contact,
}

/// One display-ready search hit. The frontend renders `title`/`subtitle`
/// verbatim and uses `thread_id`/`contact_email` to open the right view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub kind: SearchKind,
    pub title: String,
    pub subtitle: String,
    pub thread_id: Option<i64>,
    pub contact_email: Option<String>,
}

/// One row of the privacy audit log — powers the "what left my machine" view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub timestamp: i64,
    pub provider: String,
    pub model: Option<String>,
    pub purpose: String,
    pub data_summary: String,
    pub was_local: bool,
}
