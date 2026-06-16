//! THIN Tauri command handlers. They parse input, call a domain module, and
//! return — no business logic lives here (spec §6).

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

use serde::Serialize;

use crate::ai::provider::AiRequest;
use crate::ai::{self, AiConfig};
use crate::db::queries;
use crate::error::{AppError, Result};
use crate::events::{self, AiDone, AiToken, LoopsUpdated, SyncComplete, SyncError, SyncProgress};
use crate::loops::{briefing, rules};
use crate::models::{
    Account, AuditEntry, BriefingView, Contact, LoopKind, LoopView, SearchResult, ThreadView,
};
use crate::search;
use crate::secrets::{self, ImapSecret, OAuthSecret};
use crate::state::AppState;
use crate::sync::discovery::{self, ProviderHint};
use crate::sync::imap::{ImapAuth, ImapConfig, ImapSource};
use crate::sync::oauth::{self, Pkce};
use crate::sync::runner;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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

/// Email-first onboarding: from the address alone, tell the UI whether to launch
/// OAuth, ask for an app password (with the host pre-filled), or fall back to a
/// manual IMAP form. No network — instant (spec §8).
#[tauri::command]
pub fn detect_account(email: String) -> Result<ProviderHint> {
    Ok(discovery::detect(&email))
}

/// Open the system browser to a URL (the OAuth consent page).
fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    let spawned = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(url).spawn();
    spawned
        .map(|_| ())
        .map_err(|e| AppError::Sync(format!("could not open browser: {e}")))
}

/// Drive the full OAuth login (Gmail / M365): open consent in the browser, catch
/// the loopback redirect, exchange the code, discover the email, persist the
/// refresh token, create the account, and kick off a first sync. Returns the new
/// account. Needs an OAuth client ID configured for the provider.
#[tauri::command]
pub async fn start_oauth_login(app: AppHandle, provider: String) -> Result<Account> {
    let def = oauth::provider_def(&provider)
        .ok_or_else(|| AppError::Invalid(format!("unsupported OAuth provider: {provider}")))?;

    // Client credentials come from settings (set in the UI) or env vars. A public
    // client (M365) has no secret; Google desktop clients usually do.
    let (client_id, client_secret) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().expect("db mutex poisoned");
        let id = queries::get_setting(&conn, &format!("oauth_{provider}_client_id"))?
            .or_else(|| std::env::var(format!("ORBIT_{}_CLIENT_ID", provider.to_uppercase())).ok());
        let secret = queries::get_setting(&conn, &format!("oauth_{provider}_client_secret"))?
            .or_else(|| std::env::var(format!("ORBIT_{}_CLIENT_SECRET", provider.to_uppercase())).ok());
        (id, secret)
    };
    let client_id = client_id.ok_or_else(|| {
        AppError::Invalid(format!(
            "{provider} sign-in needs an OAuth client ID. Set 'oauth_{provider}_client_id' in Settings."
        ))
    })?;

    let pkce = Pkce::generate();
    let csrf = oauth::random_token(24);
    let (listener, redirect_uri) = oauth::start_loopback().await?;
    let url = oauth::authorization_url(&def, &client_id, &redirect_uri, &csrf, &pkce);
    open_url(&url)?;

    let params = oauth::await_redirect(listener).await?;
    if params.get("state").map(String::as_str) != Some(csrf.as_str()) {
        return Err(AppError::Sync("OAuth state mismatch (possible CSRF) — aborted".into()));
    }
    let code = params.get("code").ok_or_else(|| {
        let why = params.get("error").cloned().unwrap_or_else(|| "consent was denied".into());
        AppError::Sync(format!("sign-in failed: {why}"))
    })?;

    let tokens = oauth::exchange_code(
        &def,
        &client_id,
        client_secret.as_deref(),
        code,
        &redirect_uri,
        &pkce.verifier,
    )
    .await?;
    let refresh_token = tokens
        .refresh_token
        .clone()
        .ok_or_else(|| AppError::Sync("provider returned no refresh token".into()))?;
    let email = oauth::fetch_email(&def, &tokens.access_token).await?;

    let cref = secrets::oauth_cred_ref(&email);
    secrets::store_oauth(
        &cref,
        &OAuthSecret { provider: provider.clone(), refresh_token, client_id, client_secret },
    )?;

    let now = now_unix();
    let id = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().expect("db mutex poisoned");
        queries::insert_account(&conn, &email, None, &provider, "oauth", Some(&cref), now)?
    };

    // Kick off the first sync in the background.
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_sync(&app2, id).await {
            events::sync_error(&app2, SyncError { account_id: id, message: e.to_string() });
        }
    });

    Ok(Account {
        id,
        email,
        display_name: None,
        provider,
        auth_kind: "oauth".into(),
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

    // Load auth details, then build the IMAP config — app-password or OAuth.
    let (email, provider, auth_kind, cred_ref) = {
        let conn = state.db.lock().expect("db mutex poisoned");
        queries::account_auth(&conn, account_id)?
            .ok_or_else(|| AppError::NotFound(format!("account {account_id}")))?
    };

    let config = match auth_kind.as_str() {
        "oauth" => {
            let secret = secrets::load_oauth(&cred_ref)?;
            let def = oauth::provider_def(&provider)
                .ok_or_else(|| AppError::Sync(format!("unknown oauth provider {provider}")))?;
            // Mint a fresh access token from the stored refresh token.
            let tokens = oauth::refresh_access_token(
                &def,
                &secret.client_id,
                secret.client_secret.as_deref(),
                &secret.refresh_token,
            )
            .await?;
            ImapConfig {
                host: def.imap_host.to_string(),
                port: def.imap_port,
                email,
                auth: ImapAuth::OAuth { access_token: tokens.access_token },
            }
        }
        _ => {
            let secret = secrets::load_imap(&cred_ref)?;
            ImapConfig {
                host: secret.host,
                port: secret.port,
                email,
                auth: ImapAuth::Password(secret.password),
            }
        }
    };
    let mut source = ImapSource::new(config);

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
            let kind = LoopKind::from_db_str(&r.kind);
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

/// Display-ready daily briefing: counts by kind, last-synced, and the most
/// urgent loops. Reads already-detected loops; runs no detection (spec §14).
#[tauri::command]
pub fn get_daily_briefing(state: State<'_, AppState>) -> Result<BriefingView> {
    let now = now_unix();
    let conn = state.db.lock().expect("db mutex poisoned");
    briefing::build(&conn, now)
}

/// Keyword search across threads and contacts for the Ctrl+K palette. Raw input
/// is sanitized into FTS/LIKE patterns inside the search module (spec §6).
#[tauri::command]
pub fn search(state: State<'_, AppState>, query: String) -> Result<Vec<SearchResult>> {
    let conn = state.db.lock().expect("db mutex poisoned");
    search::search(&conn, &query)
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

/// Settings key under which the (non-secret) AI provider config is persisted.
const AI_SETTING_KEY: &str = "ai_provider";

#[derive(Debug, Deserialize)]
pub struct SetAiProviderInput {
    /// 'openai' | 'openrouter' | 'deepseek' | 'ollama' | 'lmstudio' | 'custom'.
    pub kind: String,
    /// Required for 'custom'; otherwise overrides the kind's default endpoint.
    pub base_url: Option<String>,
    pub model: String,
    /// API key for hosted providers (stored in the keychain). Omit for local.
    pub api_key: Option<String>,
}

/// Current AI configuration, for the settings + transparency UI.
#[derive(Debug, Clone, Serialize)]
pub struct AiStatus {
    pub configured: bool,
    pub kind: Option<String>,
    pub model: Option<String>,
    pub local: bool,
}

/// Select and persist an AI provider. The API key goes to the OS keychain; only
/// the non-secret config is written to the DB. Takes effect immediately.
#[tauri::command]
pub fn set_ai_provider(state: State<'_, AppState>, input: SetAiProviderInput) -> Result<AiStatus> {
    // Resolve base_url + locality from the kind's defaults, allowing an override.
    let (default_url, local) = AiConfig::defaults_for(&input.kind).unwrap_or(("", false));
    let base_url = input
        .base_url
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| default_url.to_string());
    if base_url.trim().is_empty() {
        return Err(AppError::Invalid("a base_url is required for this provider".into()));
    }
    if input.model.trim().is_empty() {
        return Err(AppError::Invalid("a model is required".into()));
    }

    let config = AiConfig { kind: input.kind, base_url, model: input.model, local };

    // Persist the key (if any) to the keychain, never to SQLite.
    let api_key = input.api_key.filter(|k| !k.trim().is_empty());
    if config.needs_key() && api_key.is_none() && secrets::load_ai_key()?.is_none() {
        return Err(AppError::Invalid("this provider requires an API key".into()));
    }
    if let Some(key) = &api_key {
        secrets::store_ai_key(key)?;
    }
    let key_for_build = match api_key {
        Some(k) => Some(k),
        None => secrets::load_ai_key()?,
    };

    {
        let conn = state.db.lock().expect("db mutex poisoned");
        let json = serde_json::to_string(&config).map_err(|e| AppError::Other(e.to_string()))?;
        queries::set_setting(&conn, AI_SETTING_KEY, &json)?;
    }

    state.ai.set(ai::build_provider(&config, key_for_build));
    Ok(AiStatus {
        configured: true,
        kind: Some(config.kind),
        model: Some(config.model),
        local: config.local,
    })
}

/// Remove the active AI provider — config, key, and live registration. The app
/// keeps working on heuristics alone (spec §3.3).
#[tauri::command]
pub fn clear_ai_provider(state: State<'_, AppState>) -> Result<AiStatus> {
    {
        let conn = state.db.lock().expect("db mutex poisoned");
        queries::delete_setting(&conn, AI_SETTING_KEY)?;
    }
    secrets::delete(secrets::AI_CRED_REF)?;
    state.ai.clear();
    Ok(AiStatus { configured: false, kind: None, model: None, local: false })
}

/// Report the active provider (or none) for the settings UI.
#[tauri::command]
pub fn get_ai_status(state: State<'_, AppState>) -> Result<AiStatus> {
    let config = {
        let conn = state.db.lock().expect("db mutex poisoned");
        queries::get_setting(&conn, AI_SETTING_KEY)?
    };
    match config.and_then(|j| serde_json::from_str::<AiConfig>(&j).ok()) {
        Some(c) => Ok(AiStatus {
            configured: state.ai.is_configured(),
            kind: Some(c.kind),
            model: Some(c.model),
            local: c.local,
        }),
        None => Ok(AiStatus { configured: false, kind: None, model: None, local: false }),
    }
}

/// Rebuild the active provider from persisted settings + keychain at startup, so
/// the user's choice survives restarts. Best-effort: logs and continues on error.
pub fn restore_ai_provider(state: &AppState) {
    let config = {
        let conn = state.db.lock().expect("db mutex poisoned");
        queries::get_setting(&conn, AI_SETTING_KEY)
    };
    let Ok(Some(json)) = config else { return };
    let Ok(config) = serde_json::from_str::<AiConfig>(&json) else { return };
    let key = secrets::load_ai_key().unwrap_or(None);
    state.ai.set(ai::build_provider(&config, key));
    log::info!("restored AI provider: {}", config.kind);
}
