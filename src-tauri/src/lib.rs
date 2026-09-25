//! Sparkwell desktop core.
//!
//! Module map (see docs/ARCHITECTURE.md):
//! - `window`, `platform`: sidebar docking, show/hide/pin, auto-hide
//! - `hotkey`: activation shortcut validation + registration lifecycle
//! - `sparks`: Spark repository (CRUD, favorites, copy usage)
//! - `search`: lexical + semantic retrieval and fusion
//! - `ai`: optional Ollama integration (embeddings, Smart Add)
//! - `library`, `storage`: SQLite lifecycle, migrations, safe relocation
//! - `settings`: typed app preferences
//! - `commands`: the typed IPC surface exposed to the UI

pub mod ai;
pub mod commands;
pub mod error;
pub mod hotkey;
pub mod library;
pub mod platform;
pub mod search;
pub mod settings;
pub mod sparks;
pub mod state;
pub mod storage;
pub mod window;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::ShortcutState;

use settings::{effective_library_dir, AppConfig, CONFIG_FILE_NAME};
use state::AppState;

/// Passed by the OS autostart entry so Sparkwell starts quietly in the tray.
pub const HIDDEN_ARG: &str = "--hidden";

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    // User data lives outside the install and outside the WebView cache
    // folder, so uninstalling or clearing app data never touches the library.
    let data_root = app.path().local_data_dir()?.join("Sparkwell");
    let config_path = data_root.join(CONFIG_FILE_NAME);
    let default_library_dir = data_root.join("Library");

    let config = AppConfig::load(&config_path);
    let library_dir = effective_library_dir(&config, &default_library_dir);
    let is_default = config.library_dir.is_none();
    let slot = library::open_slot(&library_dir, is_default);

    let hotkey_status = hotkey::register_initial(app.handle(), &config.hotkey);
    let pinned = config.pinned;
    app.manage(AppState::new(config_path, default_library_dir, config, slot, hotkey_status));

    build_tray(app)?;

    if let Some(w) = window::main_window(app.handle()) {
        let _ = w.set_always_on_top(pinned);
    }
    let start_hidden = std::env::args().any(|a| a == HIDDEN_ARG);
    if !start_hidden {
        window::show(app.handle());
    }

    ai::spawn_service(app.handle().clone());
    Ok(())
}

fn build_tray(app: &mut App) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide Sparkwell", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Sparkwell", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id("sparkwell")
        .tooltip("Sparkwell")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => window::toggle_from_tray(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::toggle_from_tray(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be first: a second launch just reveals the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| window::show(app)))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(512 * 1024)
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        window::toggle(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![HIDDEN_ARG])))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(setup)
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(false) if window.label() == window::MAIN => {
                window::on_focus_lost(window.app_handle());
            }
            // Closing the sidebar collapses it; Quit lives in the tray/settings.
            WindowEvent::CloseRequested { api, .. } if window.label() == window::MAIN => {
                api.prevent_close();
                window::hide(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::list_favorites,
            commands::get_spark,
            commands::create_spark,
            commands::update_spark,
            commands::delete_spark,
            commands::set_favorite,
            commands::copy_spark,
            commands::search_sparks,
            commands::suggest_metadata,
            commands::get_ai_status,
            commands::set_pinned,
            commands::hide_panel,
            commands::quit_app,
            commands::set_hotkey,
            commands::begin_hotkey_capture,
            commands::end_hotkey_capture,
            commands::set_launch_at_startup,
            commands::choose_library_folder,
            commands::change_library_location,
            commands::use_default_library,
            commands::retry_library,
            commands::get_library_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Sparkwell");
}
