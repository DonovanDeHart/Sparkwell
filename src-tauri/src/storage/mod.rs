//! SQLite library lifecycle: opening, identification, migrations, integrity.

pub mod migrations;
pub mod relocate;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::{AppError, AppResult};

/// File name of the library inside the library directory.
pub const LIBRARY_FILE_NAME: &str = "sparkwell.db";

/// `PRAGMA application_id` marker ("SPKW") identifying Sparkwell libraries so we
/// never adopt or migrate an unrelated SQLite database.
pub const APPLICATION_ID: i32 = 0x5350_4B57;

pub fn library_file(dir: &Path) -> PathBuf {
    dir.join(LIBRARY_FILE_NAME)
}

/// An open Sparkwell library.
pub struct Library {
    pub conn: Connection,
    pub path: PathBuf,
    /// True when this open created a brand-new, empty library.
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// Create the directory and database when missing (default library).
    CreateIfMissing,
    /// Fail unless the database already exists (user-chosen locations).
    ExistingOnly,
}

impl Library {
    pub fn open(dir: &Path, mode: OpenMode) -> AppResult<Library> {
        let path = library_file(dir);
        let existed = path.exists();
        if !existed {
            match mode {
                OpenMode::ExistingOnly => {
                    return Err(AppError::LibraryUnavailable(format!(
                        "No Sparkwell library was found at {}.",
                        dir.display()
                    )))
                }
                OpenMode::CreateIfMissing => std::fs::create_dir_all(dir).map_err(|e| {
                    AppError::LibraryUnavailable(format!(
                        "Couldn't create the library folder {}: {e}",
                        dir.display()
                    ))
                })?,
            }
        }

        let conn = open_connection(&path).map_err(|e| {
            AppError::LibraryUnavailable(format!("Couldn't open the library at {}: {e}", path.display()))
        })?;
        verify_identity(&conn, existed)?;
        migrations::apply(&conn)?;

        Ok(Library { conn, path, created: !existed })
    }

    /// Opens an in-memory library (tests).
    #[cfg(test)]
    pub fn open_in_memory() -> Library {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        verify_identity(&conn, false).unwrap();
        migrations::apply(&conn).unwrap();
        Library { conn, path: PathBuf::from(":memory:"), created: true }
    }

    pub fn spark_count(&self) -> AppResult<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM sparks", [], |r| r.get(0))?)
    }

    pub fn setting(&self, key: &str) -> AppResult<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}

fn open_connection(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    // WAL keeps reads fast while the background indexer writes. It is a no-op
    // for in-memory databases.
    let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// Ensures the database is a Sparkwell library (or a brand-new empty file).
fn verify_identity(conn: &Connection, existed: bool) -> AppResult<()> {
    let app_id: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    if app_id == APPLICATION_ID {
        return Ok(());
    }
    let table_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
        [],
        |r| r.get(0),
    )?;
    if app_id == 0 && table_count == 0 {
        conn.pragma_update(None, "application_id", APPLICATION_ID)?;
        return Ok(());
    }
    Err(AppError::LibraryUnavailable(if existed {
        "This file is not a Sparkwell library. Choose a different folder.".into()
    } else {
        "The library file could not be initialised.".into()
    }))
}

/// Runs SQLite's integrity check. Returns Ok(()) only for a healthy database.
pub fn integrity_check(conn: &Connection) -> AppResult<()> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(AppError::Database(format!("integrity check failed: {result}")))
    }
}

/// Current time as Unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Stable 64-bit FNV-1a hash rendered as hex. Used for content hashes; it only
/// needs to be deterministic across runs and platforms, not cryptographic.
pub fn content_hash(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            // Separator so ("ab","c") and ("a","bc") differ.
            hash ^= 0x1f;
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        for b in part.as_bytes() {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_new_library_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let lib_dir = dir.path().join("lib");
        let lib = Library::open(&lib_dir, OpenMode::CreateIfMissing).unwrap();
        assert!(lib.created);
        assert_eq!(lib.spark_count().unwrap(), 0);
        drop(lib);
        let lib = Library::open(&lib_dir, OpenMode::ExistingOnly).unwrap();
        assert!(!lib.created);
    }

    #[test]
    fn existing_only_fails_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = Library::open(dir.path(), OpenMode::ExistingOnly).err().unwrap();
        assert!(matches!(err, AppError::LibraryUnavailable(_)));
    }

    #[test]
    fn refuses_foreign_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = library_file(dir.path());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE something_else (x INTEGER);").unwrap();
        drop(conn);
        let err = Library::open(dir.path(), OpenMode::ExistingOnly).err().unwrap();
        assert!(matches!(err, AppError::LibraryUnavailable(_)));
    }

    #[test]
    fn refuses_garbage_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(library_file(dir.path()), b"this is not sqlite at all, not even close......").unwrap();
        assert!(Library::open(dir.path(), OpenMode::ExistingOnly).is_err());
    }

    #[test]
    fn settings_round_trip() {
        let lib = Library::open_in_memory();
        assert_eq!(lib.setting("k").unwrap(), None);
        lib.set_setting("k", "v1").unwrap();
        lib.set_setting("k", "v2").unwrap();
        assert_eq!(lib.setting("k").unwrap().as_deref(), Some("v2"));
    }

    #[test]
    fn content_hash_is_stable_and_separated() {
        assert_eq!(content_hash(&["abc"]), content_hash(&["abc"]));
        assert_ne!(content_hash(&["ab", "c"]), content_hash(&["a", "bc"]));
        // Pin the algorithm: a change here would silently invalidate stored hashes.
        assert_eq!(content_hash(&[""]), "cbf29ce484222325");
    }
}
