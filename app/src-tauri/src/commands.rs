//! Commands the Settings window and the setup guide call
//! (`window.__TAURI__.core.invoke`) that no feature module owns: saving
//! settings and keys, permissions, updates, feedback, presets, launch at
//! login and the microphone list. Features own their commands (notes,
//! meeting, compose, history, models, …); all are registered in main.rs.

use splashdown_core::sync::Locked;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use splashdown_config::{KeyName, Settings};
use splashdown_core::{Fallback, Mode, TextJob};
use splashdown_platform::PermissionKind;
use splashdown_providers::Endpoint;
use tauri::{AppHandle, Manager, State};

use crate::engine;
use crate::snapshot::*;
use crate::state::*;

/// Async so it runs off the main thread: checking Apple's models takes a
/// few tens of milliseconds, and Settings polls this.
#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> Result<Snapshot, String> {
    Ok(state.snapshot())
}

#[tauri::command]
pub async fn save_settings(
    settings: Settings,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Saved, String> {
    if let Some(problem) = settings.hotkey_problem() {
        return Err(problem.into());
    }
    if let Some(problem) = splashdown_config::replacement_problem(&settings.replacements) {
        return Err(problem);
    }
    // Locked from reading to storing, so a change made elsewhere at the
    // same moment (`AppState::update`) isn't lost in between.
    let settings = {
        let mut current = state.settings.locked();
        let mut settings = settings;
        // The notes folder is changed from the Notes window, so a Settings
        // window opened earlier mustn't put the old one back.
        settings.notes.folder = current.notes.folder.clone();
        // Set by the first meeting, which Settings may not have seen.
        settings.notes.meeting_reminded |= current.notes.meeting_reminded;
        settings.general.settle_learned();
        // What the organization set stays, whatever the window sent.
        splashdown_config::managed::Managed::current().apply(&mut settings);
        settings.save(&engine::paths()?.config_file())?;
        *current = settings.clone();
        settings
    };
    reflect_settings(&app, &state, &settings);
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
    })
}

/// Checks a key with its provider, then stores it in the keychain.
#[tauri::command]
pub async fn save_key(
    name: KeyName,
    value: String,
    state: State<'_, AppState>,
) -> Result<Saved, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err("Paste a key first.".into());
    }
    let endpoint = match name {
        KeyName::Openai => Endpoint::openai(&value),
        KeyName::Openrouter => Endpoint::openrouter(&value),
        KeyName::Anthropic => Endpoint::anthropic(&value),
        KeyName::Custom => Endpoint::custom(
            state
                .settings()
                .ai
                .base_url
                .unwrap_or_else(|| splashdown_config::defaults::CUSTOM_BASE_URL.into()),
            Some(value.clone()),
        ),
    };
    endpoint.verify().await.map_err(|e| match e {
        splashdown_core::ProviderError::Auth => {
            "That key was rejected — check it and try again.".to_string()
        }
        other => format!("Couldn't verify the key: {other}"),
    })?;
    splashdown_config::store_key(name, &value)?;
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
    })
}

#[tauri::command]
pub async fn delete_key(name: KeyName, state: State<'_, AppState>) -> Result<Saved, String> {
    splashdown_config::delete_key(name)?;
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
    })
}

#[derive(Serialize)]
pub struct TestResult {
    output: String,
    millis: u128,
    model: Option<String>,
    note: Option<String>,
}

/// Runs the AI pass on sample text with the saved settings.
#[tauri::command]
pub async fn test_ai(text: String, state: State<'_, AppState>) -> Result<TestResult, String> {
    let settings = state.settings();
    let engine = tauri::async_runtime::spawn_blocking(move || engine::build(&settings))
        .await
        .map_err(|e| e.to_string())??;
    let started = Instant::now();
    let outcome = engine
        .pipeline
        .process_text(TextJob {
            mode: Mode::Dictate,
            content: text,
            instruction: None,
            selection: None,
            context: engine.context.clone(),
        })
        .await
        .map_err(|e| e.to_string())?;
    crate::spend::record_outcome(&outcome);
    let note = outcome.fallback.map(|f| match f {
        Fallback::Guard(r) => format!("Raw text kept: {}", r.describe()),
        Fallback::ModelError(e) => format!("AI unavailable: {e}"),
    });
    Ok(TestResult {
        millis: started.elapsed().as_millis(),
        model: outcome.llm_name,
        output: outcome.text,
        note,
    })
}

/// Where each provider lets you cap spending on its side. The only pages
/// `open_limits_page` opens.
const LIMIT_PAGES: [(KeyName, &str); 3] = [
    (
        KeyName::Openai,
        "https://platform.openai.com/settings/organization/limits",
    ),
    (KeyName::Openrouter, "https://openrouter.ai/settings/keys"),
    (
        KeyName::Anthropic,
        "https://platform.claude.com/settings/limits",
    ),
];

/// Opens a provider's spending-limits page in the browser.
#[tauri::command]
pub fn open_limits_page(provider: KeyName) -> Result<(), String> {
    let (_, url) = LIMIT_PAGES
        .iter()
        .find(|(k, _)| *k == provider)
        .ok_or("That provider has no limits page.")?;
    splashdown_platform::open_url(url).map_err(|e| e.to_string())
}

/// "Check now" in Settings → About. Returns the newer version, if any.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Option<String>, String> {
    crate::updates::check(&app, true).await
}

/// "What changed" beside a version in Settings → About: its release notes
/// on GitHub. Only digits and dots get into the address.
#[tauri::command]
pub fn show_release_notes(version: String) -> Result<(), String> {
    let version: String = version
        .trim_start_matches('v')
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if version.is_empty() {
        return Err("No version to show.".into());
    }
    let url = format!("https://github.com/bhowerton/splashdown/releases/tag/v{version}");
    splashdown_platform::open_url(&url).map_err(|e| e.to_string())
}

/// "Install and restart": downloads the update found by the last check,
/// installs it and relaunches. Returns only if that fails.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    crate::updates::install(&app, false).await
}

/// "Send feedback": a prefilled GitHub issue in the browser.
#[tauri::command]
pub async fn send_feedback(state: State<'_, AppState>) -> Result<(), String> {
    state.send_feedback()
}

/// Shows the log file in Finder / File Explorer, for attaching to a bug
/// report. It holds timings and errors, never dictated text.
#[tauri::command]
pub fn show_log_file() -> Result<(), String> {
    let path = engine::paths()?.logs_dir.join(crate::LOG_FILE);
    splashdown_platform::reveal_in_file_manager(&path).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Accessibility,
    Microphone,
}

#[tauri::command]
pub fn open_permission(permission: Permission) -> Result<(), String> {
    let kind = match permission {
        Permission::Accessibility => PermissionKind::Accessibility,
        Permission::Microphone => PermissionKind::Microphone,
    };
    splashdown_platform::open_permission_settings(kind).map_err(|e| e.to_string())
}

/// Shows macOS's Accessibility prompt. Its "Open System Settings" button
/// lands on the list with Splashdown already added, so the user only has
/// to flip the switch.
#[tauri::command]
pub fn request_accessibility() {
    splashdown_platform::request_accessibility();
}

/// Asks for the microphone now rather than during the first dictation:
/// the system's prompt on macOS; elsewhere, opening the microphone for a
/// moment.
#[tauri::command]
pub async fn request_microphone() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        if splashdown_platform::request_microphone().is_some() {
            return Ok(());
        }
        let recorder = splashdown_audio::Recorder::start(None).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(300));
        recorder.stop();
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Settings → Insights.
#[tauri::command]
pub fn get_insights() -> crate::insights::Summary {
    crate::insights::shared().summary(chrono::Local::now())
}

#[tauri::command]
pub fn clear_insights() {
    crate::insights::shared().clear();
}

/// Runs the AI pass on sample text with one preset (Presets → Try it).
/// Uses the saved settings; nothing is inserted.
#[tauri::command]
pub async fn try_preset(
    text: String,
    preset_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let session = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || session.redo(text, None, preset_id))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Clone, Serialize)]
pub struct AppInfo {
    id: String,
    /// What people call the app ("Mail"), if it's installed here.
    name: Option<String>,
    /// Its icon as a `data:image/png` URL.
    icon: Option<String>,
}

/// Names and icons for presets' app rules (bundle IDs or executable
/// names). Looked up once per app while Splashdown runs.
#[tauri::command]
pub async fn app_info(ids: Vec<String>) -> Vec<AppInfo> {
    tauri::async_runtime::spawn_blocking(move || ids.into_iter().map(look_up_app).collect())
        .await
        .unwrap_or_default()
}

fn look_up_app(id: String) -> AppInfo {
    use base64::Engine as _;
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<HashMap<String, AppInfo>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(hit) = cache.locked().get(&id) {
        return hit.clone();
    }
    // Wildcard rules ("com.jetbrains.*") name a family, not one app.
    let display = (!id.ends_with('*'))
        .then(|| splashdown_platform::app_display(&id))
        .flatten();
    let info = AppInfo {
        name: display.as_ref().map(|d| d.name.clone()),
        icon: display.and_then(|d| d.icon_png).map(|png| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            )
        }),
        id: id.clone(),
    };
    cache.locked().insert(id, info.clone());
    info
}

#[tauri::command]
pub fn get_launch_at_login(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub fn set_launch_at_login(enabled: bool, app: AppHandle) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| e.to_string())
}

/// Marks setup as done, applies "start at login", and closes the guide
/// (unless `close` is false: the optional tour carries on in it).
#[tauri::command]
pub async fn finish_onboarding(
    launch_at_login: bool,
    close: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.update(&app, |s| {
        s.general.onboarded = true;
        s.general.setup_step = None;
        s.general.setup_choice = None;
    })?;
    // "Finish setting up…" goes, unless dictation still can't start for
    // want of the speech download.
    state.session.setup_finished();
    crate::tray::stop_pointing(&app);
    if let Err(e) = set_launch_at_login(launch_at_login, app.clone()) {
        tracing::warn!("couldn't change launch at login: {e}");
    }
    if close.unwrap_or(true)
        && let Some(w) = app.get_webview_window(crate::tray::ONBOARDING_WINDOW)
    {
        let _ = w.close();
    }
    Ok(())
}

/// Remembers where the setup guide is (`step`) and how the person chose to
/// set up (`choice`), so it opens there again if it's closed part-way.
#[tauri::command]
pub fn set_onboarding_step(
    step: String,
    choice: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let current = state.settings().general;
    if current.onboarded
        || (current.setup_step.as_deref() == Some(step.as_str()) && current.setup_choice == choice)
    {
        return Ok(());
    }
    state
        .update(&app, |s| {
            s.general.setup_step = Some(step);
            s.general.setup_choice = choice;
        })
        .map(|_| ())
}

/// The guide's last screen points at the menu bar icon for as long as it's
/// shown (`on`), in a small window of its own. Answers whether it could:
/// not where the icon's place can't be known (Windows, where it may be
/// tucked away under ^, or a Mac whose menu bar has no room for it).
/// Async: it builds a window, which deadlocks from the main thread on
/// Windows.
#[tauri::command]
pub async fn point_at_tray(on: bool, app: AppHandle) -> bool {
    if on {
        crate::tray::point_at_icon(&app)
    } else {
        crate::tray::stop_pointing(&app);
        false
    }
}

/// Windows' taskbar settings, where "Other system tray icons" keeps
/// Splashdown's icon by the clock.
#[tauri::command]
pub fn open_taskbar_settings() -> Result<(), String> {
    if cfg!(target_os = "windows") {
        splashdown_platform::open_url("ms-settings:taskbar").map_err(|e| e.to_string())
    } else {
        Err("Only on Windows.".into())
    }
}

#[tauri::command]
// Async: a synchronous command runs on the main thread, and building a
// window from there deadlocks on Windows (here: the setup guide).
pub async fn open_onboarding(app: AppHandle) {
    crate::tray::open_onboarding(&app);
}

/// Settings' ✕ on a learned word: removes it, and it isn't learned again.
#[tauri::command]
pub fn forget_learned_word(word: String, app: AppHandle) -> Result<(), String> {
    crate::learning::forget(&app, &word)
}

/// While Settings listens for a new hotkey, pressing the current one
/// mustn't start a recording.
#[tauri::command]
pub fn pause_hotkeys(paused: bool, state: State<'_, AppState>) {
    state.session.pause_hotkeys(paused);
}

/// Plays the start sound at `volume`, so the slider in Settings can be heard.
#[tauri::command]
pub fn preview_sound(volume: f32, app: AppHandle) {
    let _ = app.run_on_main_thread(move || {
        splashdown_platform::play_sound(splashdown_platform::Sound::Start, volume.clamp(0.0, 1.0));
    });
}

#[derive(Serialize)]
pub struct Microphone {
    name: String,
    bluetooth: bool,
    built_in: bool,
    is_default: bool,
    /// Never carries a microphone, so Automatic doesn't use it.
    no_microphone: bool,
}

#[derive(Serialize)]
pub struct Microphones {
    devices: Vec<Microphone>,
    /// What "Automatic" opens right now (`None`: the system default).
    automatic: Option<String>,
}

#[tauri::command]
pub fn list_microphones() -> Microphones {
    let inputs = splashdown_audio::devices::inputs();
    let automatic = splashdown_audio::devices::resolve(&[], &inputs);
    Microphones {
        devices: inputs
            .into_iter()
            .map(|i| Microphone {
                no_microphone: splashdown_audio::devices::never_a_microphone(&i.name),
                name: i.name,
                bluetooth: i.bluetooth,
                built_in: i.built_in,
                is_default: i.is_default,
            })
            .collect(),
        automatic,
    }
}
