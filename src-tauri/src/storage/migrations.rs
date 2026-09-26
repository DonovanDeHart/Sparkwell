//! Ordered, transactional schema migrations tracked with `PRAGMA user_version`.

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

/// All migrations in order. Index + 1 is the schema version it produces.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_initial.sql"),
    include_str!("../../migrations/0002_embedding_profile_version.sql"),
];

pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn apply(conn: &Connection) -> AppResult<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > LATEST_VERSION {
        return Err(AppError::LibraryUnavailable(format!(
            "This library was created by a newer version of Sparkwell (schema {current}). \
             Update Sparkwell to open it."
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }
        // Each migration and its version bump commit atomically.
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)
            .map_err(|e| AppError::Database(format!("migration {version} failed: {e}")))?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        log::info!("library migrated to schema {version}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_all_migrations_once() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, LATEST_VERSION);
        // Idempotent.
        apply(&conn).unwrap();
        for table in [
            "sparks",
            "tags",
            "spark_tags",
            "embeddings",
            "settings",
            "sparks_fts",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing table {table}");
        }
    }

    #[test]
    fn rejects_future_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", LATEST_VERSION + 1)
            .unwrap();
        assert!(matches!(apply(&conn), Err(AppError::LibraryUnavailable(_))));
    }
}
