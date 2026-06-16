//! THIN Tauri command handlers. They parse input, call a domain module, and
//! return — no business logic lives here (spec §6).

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

use crate::ai::provider::AiRequest;
use crate::db::queries;
use crate::error::{AppError, Result};
use crate::events::{self, AiDone, AiToken, LoopsUpdated, SyncComplete, SyncError, SyncProgress};
use crate::loops::rules;
use crate::models::{Account, AuditEntry, Contact, LoopKind, LoopView, ThreadView};
use crate::secrets::{self, ImapSecret};
use crate::state::AppState;
use crate::sync::imap::{ImapConfig, ImapSource};
use crate::sync::runner;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn loop_kind_from_str(s: &str) -> LoopKind {
    match s {
        "waiting_on" => LoopKind::WaitingOn,
        "owe_reply" => LoopKind::OweReply,
        _ => LoopKind::Promised,
    }
}

// --- Health / bridge --------------------------------------------------------

/// Request→response smoke-test: returns the live schema version.
#[tauri::command]
pub fn ping(state: State<'_, AppState>) -> Result<i64> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::schema_version(&conn)
}

/// Event smoke-test: emits a `loops:updated` event.
#[tauri::command]
pub fn emit_test_event(app: AppHandle) -> Result<()> {
    events::loops_updated(&app, LoopsUpdated { count: 0 });
    Ok(())
}

// --- Accounts ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AddAccountInput {
    pub email: String,
    pub display_name: Option<String>,
    /// IMAP host, e.g. "imap.fastmail.com".
    pub host: String,
    pub port: u16,
    /// App password (stored in the OS keychain, never in SQLite).
    pub password: String,
}

#[tauri::command]
pub fn add_account(state: State<'_, AppState>, input: AddAccountInput) -> Result<Account> {
    let email = input.email.trim().to_lowercase();
    if email.is_empty() || input.host.trim().is_empty() {
        return Err(AppError::Invalid("email and host are required".into()));
    }
    let cref = secrets::cred_ref(&email);
    secrets::store_imap(
        &cref,
        &ImapSecret { host: input.host.trim().to_string(), port: input.port, password: input.password },
    )?;

    let now = now_unix();
    let conn = state.db.lock().expect("db mutex poisoned");
    let id = queries::insert_account(
        &conn,
        &email,
        input.display_name.as_deref(),
        "imap",
        "password",
        Some(&cref),
        now,
    )?;
    Ok(Account {
        id,
        email,
        display_name: input.display_name,
        provider: "imap".into(),
        auth_kind: "password".into(),
        last_synced: None,
        created_at: now,
    })
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::list_accounts(&conn)
}

#[tauri::command]
pub fn remove_account(state: State<'_, AppState>, account_id: i64) -> Result<()> {
    let conn = state.db.lock().expect("db mutex poisoned");
    if let Some((email, _)) = queries::account_cred_ref(&conn, account_id)? {
        secrets::delete(&secrets::cred_ref(&email))?;
    }
    queries::remove_account(&conn, account_id)
}

/// Start a background sync for an account. Returns immediately; progress and
/// completion are pushed as `sync:*` events (spec §5).
#[tauri::command]
pub fn sync_account(app: AppHandle, account_id: i64) -> Result<()> {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_sync(&app, account_id).await {
            events::sync_error(
                &app,
                SyncError { account_id, message: e.to_string() },
            );
        }
    });
    Ok(())
}

async fn run_sync(app: &AppHandle, account_id: i64) -> Result<()> {
    let state = app.state::<AppState>();

    // Load connection details from the keychain.
    let (cred_ref, email) = {
        let conn = state.db.lock().expect("db mutex poisoned");
        match queries::account_cred_ref(&conn, account_id)? {
            Some((email, cref)) => (cref, email),
            None => return Err(AppError::NotFound(format!("account {account_id}"))),
        }
    };
    let secret = secrets::load_imap(&cred_ref)?;
    let mut source = ImapSource::new(ImapConfig {
        host: secret.host,
        port: secret.port,
        email,
        password: secret.password,
    });

    let cfg = state.config.lock().expect("config mutex poisoned").clone();
    let now = now_unix();

    let mut emit_progress = |done: u64, total: u64| {
        events::sync_progress(app, SyncProgress { account_id, done, total });
    };

    let outcome = runner::sync_with_source(
        &mut source,
        &state.db,
        account_id,
        &cfg,
        now,
        &mut emit_progress,
    )
    .await?;

    events::sync_complete(
        app,
        SyncComplete { account_id, new_messages: outcome.new_messages as u64 },
    );
    events::loops_updated(app, LoopsUpdated { count: outcome.active_loops as u64 });
    Ok(())
}

// --- Loops ------------------------------------------------------------------

#[tauri::command]
pub fn list_loops(state: State<'_, AppState>, kind: Option<String>) -> Result<Vec<LoopView>> {
    let now = now_unix();
    let conn = state.db.lock().expect("db mutex poisoned");
    let rows = queries::list_loop_views(&conn, now, kind.as_deref())?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let kind = loop_kind_from_str(&r.kind);
            let age = rules::format_age(now, r.age_anchor);
            r.into_view(kind, age)
        })
        .collect())
}

#[tauri::command]
pub fn snooze_loop(state: State<'_, AppState>, loop_id: i64, until: i64) -> Result<()> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::snooze_loop(&conn, loop_id, until)
}

#[tauri::command]
pub fn dismiss_loop(state: State<'_, AppState>, loop_id: i64) -> Result<()> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::dismiss_loop(&conn, loop_id)
}

// --- Context ----------------------------------------------------------------

#[tauri::command]
pub fn get_thread(state: State<'_, AppState>, thread_id: i64) -> Result<ThreadView> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::thread_view(&conn, thread_id)?
        .ok_or_else(|| AppError::NotFound(format!("thread {thread_id}")))
}

#[tauri::command]
pub fn list_contacts(state: State<'_, AppState>) -> Result<Vec<Contact>> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::list_contacts(&conn)
}

// --- AI (post-heuristic; optional) ------------------------------------------

/// Draft a reply for a thread, streaming tokens as `ai:token` events and a final
/// `ai:done`. Returns the request_id immediately. Errors clearly when no provider
/// is configured — heuristic loops never depend on this (spec §9).
#[tauri::command]
pub fn draft_reply(app: AppHandle, thread_id: i64, instructions: String) -> Result<String> {
    let state = app.state::<AppState>();
    if !state.ai.is_configured() {
        return Err(AppError::Ai("no AI provider configured".into()));
    }

    // Build the prompt from thread context. The data_summary honestly describes
    // what will leave the machine, for the audit log.
    let thread = {
        let conn = state.db.lock().expect("db mutex poisoned");
        queries::thread_view(&conn, thread_id)?
            .ok_or_else(|| AppError::NotFound(format!("thread {thread_id}")))?
    };
    let subject = thread.subject.clone().unwrap_or_else(|| "(no subject)".into());
    let transcript = thread
        .messages
        .iter()
        .map(|m| format!("From: {}\n{}", m.from_email, m.body_text.as_deref().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n---\n");
    let req = AiRequest {
        purpose: "draft_reply".into(),
        system: Some("You draft concise, professional email replies.".into()),
        prompt: format!("Thread subject: {subject}\n\n{transcript}\n\nInstructions: {instructions}"),
        data_summary: format!(
            "thread '{subject}' ({} messages) + user instructions",
            thread.messages.len()
        ),
        model: None,
    };

    let request_id = format!("draft-{thread_id}-{}", now_unix());
    let rid = request_id.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let now = now_unix();
        match state.ai.complete(&state.db, now, req).await {
            Ok(mut stream) => {
                use futures::StreamExt;
                while let Some(tok) = stream.next().await {
                    match tok {
                        Ok(token) => events::ai_token(&app, AiToken { request_id: rid.clone(), token }),
                        Err(e) => {
                            events::ai_token(&app, AiToken { request_id: rid.clone(), token: format!("\n[error: {e}]") });
                            break;
                        }
                    }
                }
            }
            Err(e) => events::ai_token(&app, AiToken { request_id: rid.clone(), token: format!("[error: {e}]") }),
        }
        events::ai_done(&app, AiDone { request_id: rid });
    });

    Ok(request_id)
}

/// The "what left my machine" transparency view (spec §3.3).
#[tauri::command]
pub fn get_ai_audit_log(state: State<'_, AppState>) -> Result<Vec<AuditEntry>> {
    let conn = state.db.lock().expect("db mutex poisoned");
    queries::list_audit(&conn)
}
