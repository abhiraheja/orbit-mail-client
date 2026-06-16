//! OS-keychain credential storage. SQLite stores only a `cred_ref`; the secret
//! itself (IMAP password + connection details) lives here (spec §7).

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

const SERVICE: &str = "com.rahej.orbit";

/// What we keep in the keychain for a plain-IMAP account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapSecret {
    pub host: String,
    pub port: u16,
    pub password: String,
}

/// What we keep in the keychain for an OAuth account: the long-lived refresh
/// token plus the client credentials needed to redeem it. Access tokens are
/// short-lived and re-minted per sync, never stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthSecret {
    pub provider: String, // 'gmail' | 'm365'
    pub refresh_token: String,
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// Stable keychain reference for an account, derived from its email.
pub fn cred_ref(email: &str) -> String {
    format!("imap:{}", email.trim().to_lowercase())
}

/// Keychain reference for an OAuth account.
pub fn oauth_cred_ref(email: &str) -> String {
    format!("oauth:{}", email.trim().to_lowercase())
}

pub fn store_oauth(cred_ref: &str, secret: &OAuthSecret) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, cred_ref)
        .map_err(|e| AppError::Other(format!("keychain: {e}")))?;
    let json = serde_json::to_string(secret).map_err(|e| AppError::Other(e.to_string()))?;
    entry
        .set_password(&json)
        .map_err(|e| AppError::Other(format!("keychain set: {e}")))?;
    Ok(())
}

pub fn load_oauth(cred_ref: &str) -> Result<OAuthSecret> {
    let entry = keyring::Entry::new(SERVICE, cred_ref)
        .map_err(|e| AppError::Other(format!("keychain: {e}")))?;
    let json = entry
        .get_password()
        .map_err(|e| AppError::NotFound(format!("oauth credentials for {cred_ref}: {e}")))?;
    serde_json::from_str(&json).map_err(|e| AppError::Other(e.to_string()))
}

pub fn store_imap(cred_ref: &str, secret: &ImapSecret) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, cred_ref)
        .map_err(|e| AppError::Other(format!("keychain: {e}")))?;
    let json = serde_json::to_string(secret).map_err(|e| AppError::Other(e.to_string()))?;
    entry
        .set_password(&json)
        .map_err(|e| AppError::Other(format!("keychain set: {e}")))?;
    Ok(())
}

pub fn load_imap(cred_ref: &str) -> Result<ImapSecret> {
    let entry = keyring::Entry::new(SERVICE, cred_ref)
        .map_err(|e| AppError::Other(format!("keychain: {e}")))?;
    let json = entry
        .get_password()
        .map_err(|e| AppError::NotFound(format!("credentials for {cred_ref}: {e}")))?;
    serde_json::from_str(&json).map_err(|e| AppError::Other(e.to_string()))
}

pub fn delete(cred_ref: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, cred_ref)
        .map_err(|e| AppError::Other(format!("keychain: {e}")))?;
    // Missing entry on delete is fine (idempotent account removal).
    if let Err(e) = entry.delete_credential() {
        log::debug!("keychain delete ({cred_ref}): {e}");
    }
    Ok(())
}

// --- AI provider keys -------------------------------------------------------

/// Fixed keychain reference for the active AI provider's API key. The provider's
/// config (kind/base_url/model) lives in `app_settings`; only the key is here.
pub const AI_CRED_REF: &str = "ai:provider-key";

pub fn store_ai_key(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, AI_CRED_REF)
        .map_err(|e| AppError::Other(format!("keychain: {e}")))?;
    entry
        .set_password(key)
        .map_err(|e| AppError::Other(format!("keychain set: {e}")))?;
    Ok(())
}

/// Load the stored API key, or None if none was saved (e.g. a local provider).
pub fn load_ai_key() -> Result<Option<String>> {
    let entry = keyring::Entry::new(SERVICE, AI_CRED_REF)
        .map_err(|e| AppError::Other(format!("keychain: {e}")))?;
    match entry.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Other(format!("keychain get: {e}"))),
    }
}
