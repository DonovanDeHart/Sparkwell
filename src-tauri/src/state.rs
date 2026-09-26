//! Process-wide state managed by Tauri.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use crate::ai::ollama::OllamaClient;
use crate::ai::AiStatus;
use crate::error::{AppError, AppResult};
use crate::hotkey::HotkeyStatus;
use crate::search::vectors::VectorIndex;
use crate::settings::AppConfig;
use crate::storage::Library;

/// The currently configured library and whether it opened.
pub struct LibrarySlot {
    pub dir: PathBuf,
    pub db: Option<Library>,
    pub error: Option<String>,
}

pub struct WindowFlags {
    pub last_shown: Mutex<Instant>,
    pub last_autohide: Mutex<Option<Instant>>,
    /// While > 0 (e.g. a native folder dialog is open) blur must not hide.
    pub suppress_autohide: AtomicU32,
}

pub struct AppState {
    pub config_path: PathBuf,
    pub default_library_dir: PathBuf,
    pub config: Mutex<AppConfig>,
    pub library: Mutex<LibrarySlot>,
    /// Bumped whenever a different library is opened, so background work
    /// started against the previous library can detect it and stop.
    pub library_generation: AtomicU64,
    pub vectors: RwLock<VectorIndex>,
    /// Calibration-goal vectors per embedding model (model, vectors).
    pub pool: Mutex<Option<(String, Vec<Vec<f32>>)>>,
    /// Until when the embedding model is believed to be loaded in Ollama.
    pub embed_warm_until: Mutex<Option<Instant>>,
    pub ai: Mutex<AiStatus>,
    pub ai_wake: tokio::sync::Notify,
    pub ollama: OllamaClient,
    pub hotkey: Mutex<HotkeyStatus>,
    pub window: WindowFlags,
}

impl AppState {
    pub fn new(
        config_path: PathBuf,
        default_library_dir: PathBuf,
        config: AppConfig,
        library: LibrarySlot,
        hotkey: HotkeyStatus,
    ) -> Self {
        Self {
            config_path,
            default_library_dir,
            config: Mutex::new(config),
            library: Mutex::new(library),
            library_generation: AtomicU64::new(1),
            vectors: RwLock::new(VectorIndex::default()),
            pool: Mutex::new(None),
            embed_warm_until: Mutex::new(None),
            ai: Mutex::new(AiStatus::default()),
            ai_wake: tokio::sync::Notify::new(),
            ollama: OllamaClient::default(),
            hotkey: Mutex::new(hotkey),
            window: WindowFlags {
                last_shown: Mutex::new(Instant::now()),
                last_autohide: Mutex::new(None),
                suppress_autohide: AtomicU32::new(0),
            },
        }
    }

    /// Runs `f` against the open library, or reports why it is unavailable.
    pub fn with_library<T>(&self, f: impl FnOnce(&mut Library) -> AppResult<T>) -> AppResult<T> {
        let mut slot = self
            .library
            .lock()
            .map_err(|_| AppError::Internal("library lock poisoned".into()))?;
        let dir = slot.dir.clone();
        let error = slot.error.clone();
        match slot.db.as_mut() {
            Some(lib) => f(lib),
            None => Err(AppError::LibraryUnavailable(error.unwrap_or_else(|| {
                format!("The library at {} is not available.", dir.display())
            }))),
        }
    }

    /// Like [`Self::with_library`] for writes: afterwards the change is also
    /// checkpointed into `sparkwell.db`, so the file is current on its own.
    pub fn write_library<T>(&self, f: impl FnOnce(&mut Library) -> AppResult<T>) -> AppResult<T> {
        self.with_library(|lib| {
            let out = f(lib)?;
            lib.checkpoint();
            Ok(out)
        })
    }

    /// Closes the open library cleanly (on quit). Safe to call more than once.
    pub fn close_library(&self) {
        let Ok(mut slot) = self.library.lock() else {
            return;
        };
        if let Some(lib) = slot.db.take() {
            match lib.close() {
                Ok(()) => log::info!("library closed cleanly"),
                Err(e) => log::warn!("{e}"),
            }
        }
    }

    pub fn config_snapshot(&self) -> AppConfig {
        self.config.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Applies a change to the config and saves it; the in-memory copy only
    /// changes if the save succeeded.
    pub fn update_config(&self, change: impl FnOnce(&mut AppConfig)) -> AppResult<AppConfig> {
        let mut cfg = self
            .config
            .lock()
            .map_err(|_| AppError::Internal("config lock poisoned".into()))?;
        let mut next = cfg.clone();
        change(&mut next);
        next.save(&self.config_path)?;
        *cfg = next.clone();
        Ok(next)
    }

    pub fn ai_status(&self) -> AiStatus {
        self.ai.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn generation(&self) -> u64 {
        self.library_generation.load(Ordering::SeqCst)
    }

    pub fn bump_generation(&self) {
        self.library_generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn is_pinned(&self) -> bool {
        self.config.lock().map(|c| c.pinned).unwrap_or(false)
    }
}
