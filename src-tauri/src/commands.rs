//! Typed IPC commands. This is the only surface the webview can call.
//!
//! Commands are `async` so they run off the main (UI) thread.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

use crate::ai::metadata::MetadataSuggestion;
use crate::ai::{self, AiStatus};
use crate::error::{AppError, AppResult};
use crate::hotkey::{self, HotkeyStatus};
use crate::library::{self, LibraryInfo, SwitchMode};
use crate::search::{self, SemanticScores};
use crate::sparks::{self, SparkDetail, SparkInput, SparkSummary};
use crate::state::AppState;
use crate::storage::now_ms;
use crate::storage::relocate::{self, TargetInfo};
use crate::window;

/// Longest goal text considered by retrieval.
const MAX_GOAL_CHARS: usize = 2_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub version: String,
    pub platform: &'static str,
    pub pinned: bool,
    pub hotkey: HotkeyStatus,
    pub launch_at_startup: bool,
    pub library: LibraryInfo,
    pub ai: AiStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyResult {
    pub id: i64,
    pub title: String,
    pub characters: usize,
}

fn launch_at_startup_enabled<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub async fn get_app_snapshot<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> AppResult<AppSnapshot> {
    Ok(AppSnapshot {
        version: app.package_info().version.to_string(),
        platform: std::env::consts::OS,
        pinned: state.is_pinned(),
        hotkey: state
            .hotkey
            .lock()
            .map(|h| h.clone())
            .map_err(|_| AppError::Internal("hotkey lock".into()))?,
        launch_at_startup: launch_at_startup_enabled(&app),
        library: library::info(&state),
        ai: state.ai_status(),
    })
}

// ---------------------------------------------------------------- Sparks ---

#[tauri::command]
pub async fn list_favorites(state: State<'_, AppState>) -> AppResult<Vec<SparkSummary>> {
    state.with_library(|lib| sparks::list_favorites(lib))
}

#[tauri::command]
pub async fn get_spark(state: State<'_, AppState>, id: i64) -> AppResult<SparkDetail> {
    state.with_library(|lib| sparks::get_detail(lib, id))
}

#[tauri::command]
pub async fn create_spark<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    input: SparkInput,
) -> AppResult<SparkSummary> {
    let spark = state.with_library(|lib| sparks::create(lib, input))?;
    ai::on_spark_changed(&app, spark.id);
    Ok(spark)
}

#[tauri::command]
pub async fn update_spark<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: i64,
    input: SparkInput,
) -> AppResult<SparkSummary> {
    let spark = state.with_library(|lib| sparks::update(lib, id, input))?;
    ai::on_spark_changed(&app, spark.id);
    Ok(spark)
}

#[tauri::command]
pub async fn delete_spark<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: i64,
) -> AppResult<()> {
    state.with_library(|lib| sparks::delete(lib, id))?;
    ai::on_spark_changed(&app, id);
    Ok(())
}

#[tauri::command]
pub async fn set_favorite(
    state: State<'_, AppState>,
    id: i64,
    favorite: bool,
) -> AppResult<SparkSummary> {
    state.with_library(|lib| sparks::set_favorite(lib, id, favorite))
}

/// Copies the complete stored body (never the summary) to the clipboard.
#[tauri::command]
pub async fn copy_spark<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: i64,
) -> AppResult<CopyResult> {
    let (title, body) = state.with_library(|lib| sparks::body(lib, id))?;
    app.clipboard()
        .write_text(body.clone())
        .map_err(|e| AppError::Clipboard(e.to_string()))?;
    // Usage is recorded only after the clipboard write succeeded.
    if let Err(e) = state.with_library(|lib| sparks::record_copy(lib, id)) {
        log::warn!("copied but couldn't record usage: {e}");
    }
    Ok(CopyResult {
        id,
        title,
        characters: body.chars().count(),
    })
}

#[tauri::command]
pub async fn search_sparks<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    query: String,
) -> AppResult<search::SearchOutcome> {
    // Goals are a sentence or two; bound pathological pastes.
    let query: String = query.trim().chars().take(MAX_GOAL_CHARS).collect();
    let query_vector = ai::embed_query(&app, &query).await;
    let (scores, indexed) = match &query_vector {
        Some(qv) => {
            let index = state
                .vectors
                .read()
                .map_err(|_| AppError::Internal("vector lock".into()))?;
            (SemanticScores::from_index(&index, qv), index.len())
        }
        None => (None, 0),
    };
    state.with_library(|lib| {
        let total = lib.spark_count()? as usize;
        search::search(lib, &query, scores.as_ref(), indexed < total, now_ms())
    })
}

#[tauri::command]
pub async fn suggest_metadata<R: Runtime>(
    app: AppHandle<R>,
    body: String,
) -> AppResult<MetadataSuggestion> {
    ai::suggest_metadata(&app, &body).await
}

#[tauri::command]
pub async fn get_ai_status(state: State<'_, AppState>) -> AppResult<AiStatus> {
    Ok(state.ai_status())
}

// ---------------------------------------------------------------- Window ---

#[tauri::command]
pub async fn set_pinned<R: Runtime>(app: AppHandle<R>, pinned: bool) -> AppResult<bool> {
    window::set_pinned(&app, pinned)
}

#[tauri::command]
pub async fn hide_panel<R: Runtime>(app: AppHandle<R>) -> AppResult<()> {
    window::hide(&app);
    Ok(())
}

#[tauri::command]
pub async fn quit_app<R: Runtime>(app: AppHandle<R>) -> AppResult<()> {
    app.exit(0);
    Ok(())
}

// ---------------------------------------------------------------- Hotkey ---

#[tauri::command]
pub async fn set_hotkey<R: Runtime>(
    app: AppHandle<R>,
    accelerator: String,
) -> AppResult<HotkeyStatus> {
    hotkey::change(&app, &accelerator)
}

#[tauri::command]
pub async fn begin_hotkey_capture<R: Runtime>(app: AppHandle<R>) -> AppResult<()> {
    hotkey::suspend(&app)
}

#[tauri::command]
pub async fn end_hotkey_capture<R: Runtime>(app: AppHandle<R>) -> AppResult<HotkeyStatus> {
    hotkey::resume(&app)
}

// -------------------------------------------------------------- Settings ---

#[tauri::command]
pub async fn set_launch_at_startup<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> AppResult<bool> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = result {
        return Err(AppError::Io(format!(
            "Couldn't update the startup setting: {e}"
        )));
    }
    // Report the actual registered state, not the requested one.
    Ok(launch_at_startup_enabled(&app))
}

/// Opens the native folder picker and describes the chosen folder.
#[tauri::command]
pub async fn choose_library_folder<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> AppResult<Option<TargetInfo>> {
    let current = state
        .library
        .lock()
        .map(|s| s.dir.clone())
        .map_err(|_| AppError::Internal("library lock".into()))?;
    let picked = {
        let _guard = window::AutohideSuppressed::new(&state);
        let mut dialog = app
            .dialog()
            .file()
            .set_title("Choose a folder for your Spark library");
        if current.exists() {
            dialog = dialog.set_directory(&current);
        }
        if let Some(w) = window::main_window(&app) {
            dialog = dialog.set_parent(&w);
        }
        dialog.blocking_pick_folder()
    };
    if let Some(w) = window::main_window(&app) {
        let _ = w.set_focus();
    }
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path: PathBuf = picked
        .into_path()
        .map_err(|_| AppError::Validation("That location isn't a local folder.".into()))?;
    relocate::inspect_target(&path, &current).map(Some)
}

#[tauri::command]
pub async fn change_library_location<R: Runtime>(
    app: AppHandle<R>,
    path: PathBuf,
    mode: SwitchMode,
) -> AppResult<LibraryInfo> {
    library::switch(&app, &path, mode)
}

#[tauri::command]
pub async fn use_default_library<R: Runtime>(app: AppHandle<R>) -> AppResult<LibraryInfo> {
    library::use_default(&app)
}

#[tauri::command]
pub async fn retry_library<R: Runtime>(app: AppHandle<R>) -> AppResult<LibraryInfo> {
    library::retry(&app)
}

#[tauri::command]
pub async fn get_library_info<R: Runtime>(app: AppHandle<R>) -> AppResult<LibraryInfo> {
    Ok(library::info(&app.state::<AppState>()))
}
