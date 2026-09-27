//! Application error type shared by every command.
//!
//! Errors cross the IPC boundary as `{ kind, message, ... }` objects so the UI
//! can react to the *kind* of failure (e.g. a hotkey conflict vs. an invalid
//! combination) instead of parsing message strings.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// User input failed validation. The message is safe to show directly.
    #[error("{0}")]
    Validation(String),

    #[error("That Spark no longer exists.")]
    NotFound,

    /// A Spark with an identical body already exists.
    #[error("A Spark with this exact content already exists: \"{existing_title}\".")]
    Duplicate {
        existing_id: i64,
        existing_title: String,
    },

    /// The library could not be opened or is not currently available.
    #[error("{0}")]
    LibraryUnavailable(String),

    /// A database operation failed.
    #[error("Library error: {0}")]
    Database(String),

    /// The requested shortcut is not a valid combination.
    #[error("{0}")]
    HotkeyInvalid(String),

    /// The requested shortcut could not be registered (usually owned by another app).
    #[error("{0}")]
    HotkeyConflict(String),

    #[error("Couldn't copy to the clipboard: {0}")]
    Clipboard(String),

    /// Local intelligence failed. Never fatal to core functionality.
    #[error("{0}")]
    Ai(String),

    #[error("{0}")]
    Io(String),

    #[error("{0}")]
    Internal(String),
}

impl AppError {
    fn kind(&self) -> &'static str {
        match self {
            AppError::Validation(_) => "validation",
            AppError::NotFound => "notFound",
            AppError::Duplicate { .. } => "duplicate",
            AppError::LibraryUnavailable(_) => "libraryUnavailable",
            AppError::Database(_) => "database",
            AppError::HotkeyInvalid(_) => "hotkeyInvalid",
            AppError::HotkeyConflict(_) => "hotkeyConflict",
            AppError::Clipboard(_) => "clipboard",
            AppError::Ai(_) => "ai",
            AppError::Io(_) => "io",
            AppError::Internal(_) => "internal",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 4)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        if let AppError::Duplicate {
            existing_id,
            existing_title,
        } = self
        {
            s.serialize_field("existingId", existing_id)?;
            s.serialize_field("existingTitle", existing_title)?;
        }
        s.end()
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Database(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
