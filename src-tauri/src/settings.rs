//! Typed application preferences.
//!
//! Preferences that must be known *before* the library opens (the library
//! location itself, and the activation hotkey so Sparkwell is reachable even if
//! the library is broken) live in a small JSON file in the per-user local app
//! data directory. Launch-at-startup is not stored here: the OS registration is
//! the source of truth and is queried directly.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const CONFIG_FILE_NAME: &str = "config.json";

fn yes() -> bool {
    true
}

/// `Default` is the first-run state: no shortcut, welcome not yet shown.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// Accelerator string in `global-hotkey` syntax, e.g. `Ctrl+Shift+Space`.
    /// Empty until the user chooses one: no single shortcut is free on every
    /// machine, so Sparkwell asks on first run instead of assuming.
    pub hotkey: String,
    /// Pinned panels stay visible and always on top.
    pub pinned: bool,
    /// Custom library directory. `None` means the default location.
    pub library_dir: Option<PathBuf>,
    /// First-run welcome finished (shortcut chosen or skipped). Files written
    /// before this field existed belong to existing users, so it defaults to
    /// true when missing; a brand-new install has no file and starts at false.
    #[serde(default = "yes")]
    pub onboarded: bool,
}

impl AppConfig {
    /// Loads the config file. A missing file yields first-run defaults; an
    /// unreadable or corrupt file is preserved as `config.json.bak` and
    /// defaults are used, so a bad preferences file can never stop Sparkwell
    /// from launching.
    pub fn load(path: &Path) -> AppConfig {
        match fs::read_to_string(path) {
            // Tolerate a byte-order mark from editors that add one.
            Ok(text) => {
                match serde_json::from_str::<AppConfig>(text.trim_start_matches('\u{feff}')) {
                    Ok(mut cfg) => {
                        cfg.hotkey = cfg.hotkey.trim().to_string();
                        cfg
                    }
                    Err(err) => {
                        log::warn!("config file is invalid ({err}); using defaults");
                        let _ = fs::copy(path, path.with_extension("json.bak"));
                        AppConfig::default()
                    }
                }
            }
            Err(_) => AppConfig::default(),
        }
    }

    /// Atomically writes the config (write to a temp file, then rename).
    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| AppError::Internal(e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Resolves the effective library directory.
pub fn effective_library_dir(cfg: &AppConfig, default_dir: &Path) -> PathBuf {
    cfg.library_dir
        .clone()
        .unwrap_or_else(|| default_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_has_no_shortcut_and_asks() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = AppConfig::load(&dir.path().join("config.json"));
        assert_eq!(cfg, AppConfig::default());
        assert_eq!(cfg.hotkey, "", "no shortcut is assumed to be free");
        assert!(!cfg.onboarded);
    }

    #[test]
    fn existing_users_keep_their_shortcut_and_skip_the_welcome() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // Written by 0.1.0, before `onboarded` existed.
        fs::write(
            &path,
            r#"{"hotkey":"Ctrl+Shift+Space","pinned":true,"libraryDir":null}"#,
        )
        .unwrap();
        let cfg = AppConfig::load(&path);
        assert_eq!(cfg.hotkey, "Ctrl+Shift+Space");
        assert!(cfg.pinned);
        assert!(cfg.onboarded);
    }

    #[test]
    fn a_byte_order_mark_is_not_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            "\u{feff}{\"hotkey\":\"Ctrl+Shift+K\",\"onboarded\":true}",
        )
        .unwrap();
        let cfg = AppConfig::load(&path);
        assert_eq!(cfg.hotkey, "Ctrl+Shift+K");
        assert!(cfg.onboarded);
        assert!(!dir.path().join("config.json.bak").exists());
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.json");
        let cfg = AppConfig {
            hotkey: "Ctrl+Shift+K".into(),
            pinned: true,
            library_dir: Some(dir.path().join("lib")),
            onboarded: true,
        };
        cfg.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path), cfg);
        let unset = AppConfig {
            onboarded: true,
            ..AppConfig::default()
        };
        unset.save(&path).unwrap();
        assert_eq!(
            AppConfig::load(&path),
            unset,
            "a skipped shortcut stays unset"
        );
    }

    #[test]
    fn corrupt_file_falls_back_and_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{not json").unwrap();
        let cfg = AppConfig::load(&path);
        assert_eq!(cfg, AppConfig::default());
        assert!(dir.path().join("config.json.bak").exists());
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"pinned":true,"hotkey":" "}"#).unwrap();
        let cfg = AppConfig::load(&path);
        assert!(cfg.pinned);
        assert_eq!(cfg.hotkey, "");
        assert_eq!(cfg.library_dir, None);
    }
}
