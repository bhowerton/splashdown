//! The Splashdown app: starts the menu bar (tray) icon, the dictation
//! session, the windows and the background jobs, and registers every Tauri
//! command the UI (`app/ui`) calls. See docs/ARCHITECTURE.md for a map.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod ask;
mod audio;
mod calls;
mod chunks;
mod coding_agents;
mod commands;
mod compose;
mod did;
mod engine;
mod errors;
mod export;
mod feedback;
mod help;
mod history;
mod insights;
mod learning;
mod meeting;
mod minibar;
mod models;
mod my_actions;
mod notes;
mod pill;
mod placement;
mod preview;
mod project;
mod prompt;
mod reading;
mod recent;
mod replacements;
mod rescue;
mod session;
mod setup;
mod shortcuts;
mod snapshot;
mod spend;
mod state;
mod stream_typing;
mod style;
mod tray;
mod triggers;
mod ui;
mod updates;
mod voice_actions;

use std::sync::{Arc, Mutex};

use splashdown_config::Paths;
use tauri::{Listener, Manager, RunEvent};

use crate::history::History;

fn main() {
    // Already running: that copy shows Settings, and this one quits
    // before it takes the microphone or the dictation key.
    // No logging here: starting it could move the running copy's log
    // aside (it's rotated at startup), and that copy notes the hand-over.
    if triggers::hand_over() {
        return;
    }
    // Before logging starts, so the old logs folder can still move.
    let moved = Paths::resolve()
        .map(|p| p.migrate_legacy())
        .unwrap_or_default();
    init_logging();
    for to in moved {
        tracing::info!("moved files from the app's former name to {}", to.display());
    }
    let keys = splashdown_config::migrate_legacy_keys();
    if keys > 0 {
        tracing::info!("moved {keys} API key(s) from the app's former keychain entry");
    }
    load_dev_keys();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        // Meeting recordings, for the Notes window's player.
        .register_asynchronous_uri_scheme_protocol(
            notes::media::SCHEME,
            |ctx, request, responder| {
                let app = ctx.app_handle().clone();
                std::thread::spawn(move || {
                    responder.respond(notes::media::serve(&app, &request));
                });
            },
        )
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::save_settings,
            commands::save_key,
            commands::delete_key,
            commands::test_ai,
            models::download_model,
            setup::local_setup_plan,
            setup::local_setup_status,
            setup::start_local_setup,
            models::prepare_apple_speech,
            commands::show_log_file,
            commands::open_limits_page,
            commands::open_permission,
            commands::request_accessibility,
            commands::request_microphone,
            history::get_history,
            history::delete_history,
            history::retranscribe_history,
            history::clear_history,
            history::pin_history,
            commands::try_preset,
            commands::get_insights,
            commands::clear_insights,
            commands::app_info,
            history::copy_text,
            coding_agents::coding_agents,
            coding_agents::coding_agent_install,
            history::get_recent_apps,
            history::redo_history,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
            commands::finish_onboarding,
            commands::open_onboarding,
            commands::forget_learned_word,
            style::style_profiles,
            style::style_set_profile,
            style::style_forget,
            commands::pause_hotkeys,
            commands::preview_sound,
            commands::list_microphones,
            commands::check_for_updates,
            commands::install_update,
            commands::send_feedback,
            notes::notes_load,
            notes::notes_list,
            notes::notes_open,
            notes::notes_save,
            notes::notes_new,
            notes::notes_delete,
            notes::notes_pin,
            notes::notes_rename,
            notes::notes_versions,
            notes::notes_restore,
            notes::notes_reveal,
            notes::notes_folder,
            notes::notes_pick_folder,
            notes::notes_set_folder,
            notes::notes_send,
            notes::notes_transforms,
            notes::notes_transform,
            notes::notes_ask,
            notes::notes_keep_on_top,
            notes::notes_folders,
            notes::notes_create_folder,
            notes::notes_move_folder,
            notes::notes_delete_folder,
            notes::notes_move,
            notes::notes_set_tags,
            notes::notes_set_hidden_from_agents,
            notes::notes_daily,
            notes::notes_edit_transforms,
            notes::notes_add,
            notes::files::notes_transcribe_pick,
            notes::links::notes_transcribe_url,
            notes::links::notes_install_link_helper,
            notes::links::notes_transcribe_stop,
            notes::files::notes_transcribing,
            notes::files::notes_transcribe_again,
            notes::files::notes_transcribe_again_models,
            meeting::meeting_toggle,
            meeting::meeting_keep_going,
            meeting::meeting_set_notes,
            meeting::meeting_start_app,
            meeting::meeting_start_microphone,
            meeting::calendar_access,
            meeting::audio_apps,
            meeting::meeting_status,
            notes::open_notes,
            notes::notes_meaning_progress,
            export::export_history,
            export::export_settings,
            export::check_settings_file,
            export::import_settings,
            export::import_words,
            export::export_preset,
            export::read_preset_file,
            ask::ask_state,
            ask::ask_insert,
            ask::ask_copy,
            ask::ask_close,
            ask::ask_followup,
            compose::compose_state,
            compose::compose_change,
            compose::compose_version,
            compose::compose_insert,
            compose::compose_copy,
            compose::compose_close,
            reading::reading_voices,
            reading::reading_natural,
            reading::reading_try,
            reading::reading_toggle,
            reading::reading_state,
            help::help_sheet,
            help::help_close,
            help::help_open_actions,
            my_actions::my_action_problem,
            my_actions::my_action_pick_program,
            did::did_since,
            prompt::prompt_state,
            prompt::prompt_answer,
            calls::call_offer_state,
            calls::call_offer_answer,
        ])
        .setup(|app| {
            // Menu bar app: no Dock icon, no app menu.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            #[cfg(target_os = "macos")]
            migrate_login_item(app.handle());

            let history = Arc::new(History::open(
                engine::paths()
                    .map(|p| p.history_file())
                    .unwrap_or_else(|_| std::env::temp_dir().join("splashdown-history.jsonl")),
            ));
            // A dictation the app closed on is kept for Retry.
            rescue::recover();
            let tray_menu = tray::install(app.handle(), history.clone())?;
            ui::init_overlay();
            splashdown_platform::refresh_shortcut_keys();
            shortcuts::install(app, history.clone());

            if let Ok(paths) = engine::paths() {
                splashdown_providers::factory::use_speech_detector(&paths.models_dir);
            }
            let settings = engine::load_settings();
            if settings.actions.enabled {
                voice_actions::warm_up();
            }
            minibar::init(app.handle(), settings.general.mini_bar());
            // A meeting the app closed on: offer to finish its note.
            meeting::recover(app.handle());
            calls::watch(app.handle());
            notes::watch_front_app(app.handle());
            history.set_keep_days(settings.general.history_days());
            let onboarded = settings.general.onboarded;
            let built = engine::build(&settings);
            let engine_error = built.as_ref().err().cloned();
            if let Some(e) = &engine_error {
                tracing::warn!("dictation isn't set up: {e}");
            }
            let updates = updates::Updates::new(tray_menu.clone());
            let ui = ui::Ui::new(app.handle().clone(), tray_menu.clone());
            reading::init(ui.clone());
            let session = session::spawn(ui, built, engine::keys(&settings), history.clone());
            let only_copy = triggers::serve(session.clone());
            // The session asks for a rebuild when spending crosses a limit.
            let handle = app.handle().clone();
            app.listen_any(ui::REBUILD_ENGINE, move |_| {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<crate::state::AppState>();
                    if let Some(e) = state.apply().await {
                        tracing::warn!("rebuilding after a spending change: {e}");
                    }
                });
            });
            let presets = settings.presets();
            let transforms = settings.transforms();
            app.manage(crate::state::AppState {
                settings: Mutex::new(settings),
                session,
                engine_error: Mutex::new(engine_error),
                history,
                tray: tray_menu,
                updates,
            });
            updates::start(app.handle());
            shortcuts::sync(app.handle(), &presets, &transforms);

            if !only_copy {
                app.handle().exit(0);
                return Ok(());
            }
            // First run: the setup guide. Later, if a permission was
            // revoked: Settings, which opens on the Permissions tab.
            if !onboarded {
                tray::open_onboarding(app.handle());
            } else if !splashdown_platform::permissions().accessibility {
                tray::open_settings(app.handle());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Splashdown app");

    app.run(|app, event| {
        triggers::on_run_event(app, &event);
        // Quitting mid-recording mustn't leave other sounds turned down.
        if let RunEvent::Exit = event {
            splashdown_platform::restore_output();
        }
        // Closing the last window (Settings) must not quit a tray app.
        // An explicit exit (Quit menu item) carries an exit code.
        if let RunEvent::ExitRequested { api, code, .. } = event
            && code.is_none()
        {
            api.prevent_exit();
        }
    });
}

/// The login item from before the app was renamed to Splashdown points at
/// the old app. If there is one, replace it with ours.
#[cfg(target_os = "macos")]
fn migrate_login_item(app: &tauri::AppHandle) {
    use tauri_plugin_autostart::ManagerExt;

    let Some(home) = std::env::home_dir() else {
        return;
    };
    let legacy = home.join("Library/LaunchAgents/just-say-it.plist");
    if legacy.exists() && std::fs::remove_file(&legacy).is_ok() {
        match app.autolaunch().enable() {
            Ok(()) => tracing::info!("moved the login item to Splashdown"),
            Err(e) => tracing::warn!("couldn't re-create the login item: {e}"),
        }
    }
}

/// A log file larger than this is moved aside (to `.old.log`) at startup.
const LOG_ROTATE_BYTES: u64 = 2 * 1024 * 1024;

/// Logs to stdout and to `splashdown.log` in the platform's logs folder
/// (`Paths::logs_dir`), which "Show log file" in Settings opens. Logs hold
/// timings, provider names and errors, never what the user said or typed.
fn init_logging() {
    use tracing_subscriber::prelude::*;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,splashdown=debug,splashdown_platform=debug".into());
    let file = Paths::resolve().and_then(|p| open_log_file(&p.logs_dir).ok());
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file.map(|f| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(f))
        }))
        .init();
}

fn open_log_file(dir: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(LOG_FILE);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
        let _ = std::fs::rename(&path, dir.join("splashdown.old.log"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// The log file's name inside `Paths::logs_dir`.
pub const LOG_FILE: &str = "splashdown.log";

/// Keys normally live in the OS keychain (set in Settings). For development
/// they can also come from the file named by `SPLASHDOWN_ENV_FILE` (set by
/// `scripts/run-mac-app.sh`) and, in debug builds only, from `.env.local` in
/// the working directory or `.env` in the app's config folder. Environment
/// keys override the keychain.
fn load_dev_keys() {
    if let Ok(path) = std::env::var("SPLASHDOWN_ENV_FILE") {
        let _ = dotenvy::from_path(path);
    }
    if cfg!(debug_assertions) {
        let _ = dotenvy::from_filename(".env.local");
        if let Some(paths) = Paths::resolve() {
            let _ = dotenvy::from_path(paths.config_dir.join(".env"));
        }
    }
}
