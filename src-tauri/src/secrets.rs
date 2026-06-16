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

/// Stable keychain reference for an account, derived from its email.
pub fn cred_ref(email: &str) -> String {
    format!("imap:{}", email.trim().to_lowercase())
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
