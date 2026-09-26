//! Safe library relocation.
//!
//! Rules (from the technical spec): relocation must be transactional or safely
//! recoverable, and the existing library is never silently moved or deleted.
//! The current library is *copied* into the new folder with `VACUUM INTO`
//! (a consistent snapshot), the copy is verified, and only then does the app
//! switch over. The original file is left untouched as a backup.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use super::{integrity_check, library_file, Library, APPLICATION_ID};
use crate::error::{AppError, AppResult};

const TEMP_SUFFIX: &str = "migrating";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub path: PathBuf,
    /// A Sparkwell library already exists in the folder.
    pub has_existing_library: bool,
    /// Number of Sparks in the existing library, when readable.
    pub existing_spark_count: Option<i64>,
    /// The folder is the library currently in use.
    pub is_current: bool,
}

/// Validates a user-chosen folder and describes what is already there.
pub fn inspect_target(target: &Path, current_dir: &Path) -> AppResult<TargetInfo> {
    if !target.is_absolute() {
        return Err(AppError::Validation(
            "Choose an absolute folder path.".into(),
        ));
    }
    if !target.exists() {
        return Err(AppError::Validation("That folder doesn't exist.".into()));
    }
    if !target.is_dir() {
        return Err(AppError::Validation("Choose a folder, not a file.".into()));
    }
    let target = fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let is_current = fs::canonicalize(current_dir)
        .map(|c| c == target)
        .unwrap_or(false);

    let file = library_file(&target);
    let (has_existing_library, existing_spark_count) = if file.exists() {
        (true, read_spark_count(&file).ok())
    } else {
        (false, None)
    };

    Ok(TargetInfo {
        path: strip_verbatim(&target),
        has_existing_library,
        existing_spark_count,
        is_current,
    })
}

/// Confirms we can create files in the folder before attempting a migration.
pub fn ensure_writable(dir: &Path) -> AppResult<()> {
    let probe = dir.join(".sparkwell-write-test");
    fs::write(&probe, b"ok")
        .map_err(|e| AppError::Validation(format!("Sparkwell can't write to that folder: {e}")))?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Copies `current` into `target_dir/sparkwell.db` and verifies the copy.
/// On any failure the partial copy is removed and the original is untouched.
pub fn copy_library_to(current: &Library, target_dir: &Path) -> AppResult<PathBuf> {
    let final_path = library_file(target_dir);
    if final_path.exists() {
        return Err(AppError::Validation(
            "A Sparkwell library already exists in that folder.".into(),
        ));
    }
    ensure_writable(target_dir)?;

    let temp_path = final_path.with_extension(format!("db.{TEMP_SUFFIX}"));
    // Only ever remove our own temp artefact.
    let _ = fs::remove_file(&temp_path);

    let result = (|| -> AppResult<()> {
        let temp_str = temp_path
            .to_str()
            .ok_or_else(|| AppError::Validation("That folder path isn't supported.".into()))?;
        current
            .conn
            .execute("VACUUM INTO ?1", [temp_str])
            .map_err(|e| AppError::Database(format!("couldn't copy the library: {e}")))?;

        let expected = current.spark_count()?;
        let copy = Connection::open_with_flags(&temp_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        integrity_check(&copy)?;
        let app_id: i32 = copy.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let count: i64 = copy.query_row("SELECT COUNT(*) FROM sparks", [], |r| r.get(0))?;
        drop(copy);
        if app_id != APPLICATION_ID || count != expected {
            return Err(AppError::Database(
                "the copied library didn't verify; nothing was changed".into(),
            ));
        }
        fs::rename(&temp_path, &final_path)?;
        Ok(())
    })();

    if let Err(err) = result {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    Ok(final_path)
}

fn read_spark_count(file: &Path) -> AppResult<i64> {
    let conn = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let app_id: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    if app_id != APPLICATION_ID {
        return Err(AppError::LibraryUnavailable(
            "not a Sparkwell library".into(),
        ));
    }
    Ok(conn.query_row("SELECT COUNT(*) FROM sparks", [], |r| r.get(0))?)
}

/// `fs::canonicalize` on Windows yields `\\?\C:\...` paths; show users the
/// familiar form.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparks::{self, SparkInput};
    use crate::storage::OpenMode;

    fn library_with_sparks(dir: &Path, n: usize) -> Library {
        let mut lib = Library::open(dir, OpenMode::CreateIfMissing).unwrap();
        for i in 0..n {
            sparks::create(
                &mut lib,
                SparkInput {
                    title: format!("Spark {i}"),
                    body: format!("Body {i}"),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        lib
    }

    #[test]
    fn copies_and_verifies() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        fs::create_dir_all(&dst).unwrap();
        let lib = library_with_sparks(&src, 3);

        let copied = copy_library_to(&lib, &dst).unwrap();
        assert!(copied.exists());
        // Original untouched.
        assert!(library_file(&src).exists());
        assert_eq!(lib.spark_count().unwrap(), 3);

        let reopened = Library::open(&dst, OpenMode::ExistingOnly).unwrap();
        assert_eq!(reopened.spark_count().unwrap(), 3);
        // Full-text index travels with the copy.
        let hits: i64 = reopened
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sparks_fts WHERE sparks_fts MATCH 'spark'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 3);
        assert!(!dst.join("sparkwell.db.migrating").exists());
    }

    #[test]
    fn refuses_to_overwrite_existing_library() {
        let root = tempfile::tempdir().unwrap();
        let a = library_with_sparks(&root.path().join("a"), 1);
        let _b = library_with_sparks(&root.path().join("b"), 2);
        let err = copy_library_to(&a, &root.path().join("b")).err().unwrap();
        assert!(matches!(err, AppError::Validation(_)));
        let b = Library::open(&root.path().join("b"), OpenMode::ExistingOnly).unwrap();
        assert_eq!(
            b.spark_count().unwrap(),
            2,
            "existing library must be untouched"
        );
    }

    #[test]
    fn failed_copy_leaves_no_partial_file() {
        let root = tempfile::tempdir().unwrap();
        let lib = library_with_sparks(&root.path().join("a"), 1);
        let missing = root.path().join("does-not-exist");
        assert!(copy_library_to(&lib, &missing).is_err());
        assert!(!missing.join("sparkwell.db.migrating").exists());
        assert!(!missing.join("sparkwell.db").exists());
    }

    #[test]
    fn inspect_reports_existing_and_current() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let _lib = library_with_sparks(&a, 2);
        let info = inspect_target(&a, &a).unwrap();
        assert!(info.has_existing_library);
        assert_eq!(info.existing_spark_count, Some(2));
        assert!(info.is_current);

        let empty = root.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        let info = inspect_target(&empty, &a).unwrap();
        assert!(!info.has_existing_library);
        assert!(!info.is_current);
    }

    #[test]
    fn inspect_rejects_bad_paths() {
        let root = tempfile::tempdir().unwrap();
        assert!(inspect_target(Path::new("relative/path"), root.path()).is_err());
        assert!(inspect_target(&root.path().join("missing"), root.path()).is_err());
        let file = root.path().join("file.txt");
        fs::write(&file, "x").unwrap();
        assert!(inspect_target(&file, root.path()).is_err());
    }

    #[test]
    fn strips_verbatim_prefixes() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\C:\Users\me")),
            PathBuf::from(r"C:\Users\me")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\server\share")
        );
        assert_eq!(
            strip_verbatim(Path::new("/home/me")),
            PathBuf::from("/home/me")
        );
    }
}
