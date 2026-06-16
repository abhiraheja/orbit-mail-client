//! One app-wide error type. Every command returns `Result<T, AppError>`.
//!
//! Errors must cross the Tauri IPC boundary, so `AppError` serializes to a
//! plain string the frontend can display. The frontend never branches on error
//! *kind* — it only renders the message — so a flat serialization is enough.

use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("sync error: {0}")]
    Sync(String),

    #[error("ai error: {0}")]
    Ai(String),

    #[error("{0}")]
    Other(String),
}

// Serialize as the human-readable message; that is all the render-only frontend
// needs. If we ever want typed errors on the frontend, add a tagged enum here.
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
