//! Library lifecycle service: open at startup, switch location safely, recover.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::error::{AppError, AppResult};
use crate::sparks::seed::seed_starter_sparks;
use crate::state::{AppState, LibrarySlot};
use crate::storage::relocate::{self, strip_verbatim};
use crate::storage::{library_file, Library, OpenMode};

pub const EVENT_LIBRARY_CHANGED: &str = "sparkwell://library-changed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryInfo {
    /// Folder that holds the library.
    pub dir: PathBuf,
    /// Full path of the database file.
    pub file: PathBuf,
    pub is_default: bool,
    pub available: bool,
    pub error: Option<String>,
    pub spark_count: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SwitchMode {
    /// Copy the current library into the folder, verify, then switch to it.
    Copy,
    /// Switch to the Sparkwell library that already exists in the folder.
    Open,
    /// Start a new library in the folder (recovery when the current one is unavailable).
    Create,
}

/// Opens a library directory. The default location is created on demand; a
/// custom location must already contain a library (a missing drive must never
/// silently produce an empty library somewhere else).
pub fn open_slot(dir: &Path, create_if_missing: bool) -> LibrarySlot {
    let mode = if create_if_missing { OpenMode::CreateIfMissing } else { OpenMode::ExistingOnly };
    match Library::open(dir, mode) {
        Ok(mut lib) => {
            if lib.created {
                if let Err(e) = seed_starter_sparks(&mut lib) {
                    log::warn!("could not add starter Sparks: {e}");
                }
            }
            LibrarySlot { dir: dir.to_path_buf(), db: Some(lib), error: None }
        }
        Err(e) => {
            log::error!("library unavailable at {}: {e}", dir.display());
            LibrarySlot { dir: dir.to_path_buf(), db: None, error: Some(e.to_string()) }
        }
    }
}

pub fn info(state: &AppState) -> LibraryInfo {
    let slot = state.library.lock().expect("library lock poisoned");
    let is_default = paths_equal(&slot.dir, &state.default_library_dir);
    LibraryInfo {
        dir: strip_verbatim(&slot.dir),
        file: strip_verbatim(&library_file(&slot.dir)),
        is_default,
        available: slot.db.is_some(),
        error: slot.error.clone(),
        spark_count: slot.db.as_ref().and_then(|l| l.spark_count().ok()).unwrap_or(0),
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Installs `slot` as the active library and refreshes dependent state.
fn install<R: Runtime>(app: &AppHandle<R>, slot: LibrarySlot) {
    let state = app.state::<AppState>();
    {
        let mut current = state.library.lock().expect("library lock poisoned");
        *current = slot;
    }
    state.bump_generation();
    if let Some(model) = state.ai_status().embed_model {
        crate::ai::reload_vectors(app, &model);
    } else if let Ok(mut v) = state.vectors.write() {
        *v = crate::search::vectors::VectorIndex::default();
    }
    crate::ai::wake(app);
    let _ = app.emit(EVENT_LIBRARY_CHANGED, info(&state));
}

/// Switches to `target` using `mode`. The previous library file is never
/// modified or deleted; on any failure the previous library stays active.
pub fn switch<R: Runtime>(app: &AppHandle<R>, target: &Path, mode: SwitchMode) -> AppResult<LibraryInfo> {
    let state = app.state::<AppState>();
    let current_dir = state.library.lock().map(|s| s.dir.clone()).map_err(|_| AppError::Internal("library lock poisoned".into()))?;
    let target_info = relocate::inspect_target(target, &current_dir)?;
    if target_info.is_current && state.library.lock().map(|s| s.db.is_some()).unwrap_or(false) {
        return Err(AppError::Validation("That folder is already your library location.".into()));
    }
    let target_dir = target_info.path.clone();

    let mut created_copy: Option<PathBuf> = None;
    let new_lib = match mode {
        SwitchMode::Copy => {
            if target_info.has_existing_library {
                return Err(AppError::Validation(
                    "That folder already has a Sparkwell library. Choose \"Use that library\" or pick an empty folder.".into(),
                ));
            }
            let copied = state.with_library(|current| relocate::copy_library_to(current, &target_dir))?;
            created_copy = Some(copied);
            Library::open(&target_dir, OpenMode::ExistingOnly)
        }
        SwitchMode::Open => {
            if !target_info.has_existing_library {
                return Err(AppError::Validation("No Sparkwell library was found in that folder.".into()));
            }
            Library::open(&target_dir, OpenMode::ExistingOnly)
        }
        SwitchMode::Create => {
            if target_info.has_existing_library {
                return Err(AppError::Validation("That folder already has a Sparkwell library.".into()));
            }
            relocate::ensure_writable(&target_dir)?;
            Library::open(&target_dir, OpenMode::CreateIfMissing).and_then(|mut lib| {
                if lib.created {
                    seed_starter_sparks(&mut lib)?;
                }
                Ok(lib)
            })
        }
    };

    let new_lib = match new_lib {
        Ok(lib) => lib,
        Err(e) => {
            if let Some(copy) = created_copy {
                // Roll back our own fresh copy; the original is untouched.
                let _ = std::fs::remove_file(copy);
            }
            return Err(e);
        }
    };

    // Persist the new location before switching, so disk and memory agree.
    let is_default = paths_equal(&target_dir, &state.default_library_dir);
    if let Err(e) = state.update_config(|c| {
        c.library_dir = if is_default { None } else { Some(target_dir.clone()) };
    }) {
        drop(new_lib);
        if let Some(copy) = created_copy {
            let _ = std::fs::remove_file(copy);
        }
        return Err(e);
    }

    install(app, LibrarySlot { dir: target_dir, db: Some(new_lib), error: None });
    Ok(info(&state))
}

/// Returns to the default library location (creating it if needed).
pub fn use_default<R: Runtime>(app: &AppHandle<R>) -> AppResult<LibraryInfo> {
    let state = app.state::<AppState>();
    let dir = state.default_library_dir.clone();
    let slot = open_slot(&dir, true);
    if let Some(err) = &slot.error {
        return Err(AppError::LibraryUnavailable(err.clone()));
    }
    state.update_config(|c| c.library_dir = None)?;
    install(app, slot);
    Ok(info(&state))
}

/// Retries opening the configured library (e.g. after reconnecting a drive).
pub fn retry<R: Runtime>(app: &AppHandle<R>) -> AppResult<LibraryInfo> {
    let state = app.state::<AppState>();
    let dir = state.library.lock().map(|s| s.dir.clone()).map_err(|_| AppError::Internal("library lock poisoned".into()))?;
    let is_default = paths_equal(&dir, &state.default_library_dir);
    let slot = open_slot(&dir, is_default);
    let failed = slot.error.clone();
    install(app, slot);
    match failed {
        Some(err) => Err(AppError::LibraryUnavailable(err)),
        None => Ok(info(&state)),
    }
}
