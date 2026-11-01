//! Commands the Settings window calls (`window.__TAURI__.core.invoke`).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use splashdown_config::{
    AiProvider, Destination, KeyName, KeySource, Secrets, Settings, Situation, SpeechChoice,
    SpeechProvider,
};
use splashdown_core::gesture::PhysicalKey;
use splashdown_core::{Fallback, Mode, TextJob};
use splashdown_platform::{Access, PermissionKind};
use splashdown_providers::apple::AppleStatus;
use splashdown_providers::{Endpoint, LocalModel, SpeechModel};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::engine;
use crate::history::{Entry, History};
use crate::session;

/// Shared app state for commands.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub session: session::Handle,
    /// Why dictation can't run with the current settings, if it can't.
    pub engine_error: Mutex<Option<String>>,
    pub history: Arc<History>,
    /// The menu bar menu, to keep its language ticks in sync.
    pub tray: crate::tray::TrayMenu,
    pub updates: crate::updates::Updates,
}

impl AppState {
    /// Flips whisper mode (from the menu bar), saves, and rebuilds.
    pub async fn toggle_whisper_mode(&self) -> Result<(), String> {
        let mut settings = self.settings();
        let on = !settings.general.whisper_mode();
        settings.general.whisper_mode = Some(on);
        settings.save(&engine::paths()?.config_file())?;
        self.tray.show_whisper(on);
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
        if let Some(e) = self.apply().await {
            tracing::warn!("after changing whisper mode: {e}");
        }
        Ok(())
    }

    /// Shows or hides the mini bar (from the menu bar or the bar's own
    /// menu) and saves the setting. Nothing to rebuild.
    pub fn set_mini_bar(&self, on: bool) -> Result<(), String> {
        let mut settings = self.settings();
        settings.general.mini_bar = on.then_some(true);
        settings.save(&engine::paths()?.config_file())?;
        self.tray.show_mini_bar(on);
        crate::minibar::set_visible(on);
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
        Ok(())
    }

    /// Sets the spoken language (from the menu bar), saves, and rebuilds.
    pub async fn set_language(&self, language: Option<String>) -> Result<(), String> {
        let mut settings = self.settings();
        settings.general.language = language;
        settings.save(&engine::paths()?.config_file())?;
        self.tray
            .show_language(settings.general.language.as_deref());
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
        if let Some(e) = self.apply().await {
            tracing::warn!("after changing the language: {e}");
        }
        Ok(())
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Rebuilds the engine from current settings and keys, and hands it to
    /// the session. Returns the engine error, if any.
    pub async fn apply(&self) -> Option<String> {
        let settings = self.settings();
        self.history.set_keep_days(settings.general.history_days());
        let keys = engine::keys(&settings);
        let built = tauri::async_runtime::spawn_blocking(move || engine::build(&settings))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        let error = built.as_ref().err().cloned();
        *self.engine_error.lock().unwrap_or_else(|e| e.into_inner()) = error.clone();
        self.session.reconfigure(built, keys);
        error
    }

    fn snapshot(&self) -> Snapshot {
        let error = self
            .engine_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        snapshot(self.settings(), error, self.updates.status())
    }

    /// Opens a new bug report, prefilled with the version, OS and the
    /// providers in use. Never keys, dictated text or file paths.
    pub fn send_feedback(&self) -> Result<(), String> {
        let snap = self.snapshot();
        let speech = snap.resolved_speech.as_deref().unwrap_or("not set up");
        let ai = snap.resolved_ai.as_deref().unwrap_or("not set up");
        let url = crate::feedback::issue_url(
            snap.version,
            splashdown_platform::os_version().as_deref(),
            &format!("Speech to text: {speech} · Cleanup: {ai}"),
        );
        splashdown_platform::open_url(&url).map_err(|e| e.to_string())
    }
}

/// Built-in default models, so the UI can show what an empty field means.
#[derive(Serialize)]
pub struct Defaults {
    openai_stt: &'static str,
    openrouter_stt: &'static str,
    local_speech: &'static str,
    openai_ai: &'static str,
    openrouter_ai: &'static str,
    anthropic_ai: &'static str,
    sound_volume: f32,
}

const DEFAULTS: Defaults = Defaults {
    openai_stt: splashdown_config::defaults::OPENAI_STT,
    openrouter_stt: splashdown_config::defaults::OPENROUTER_STT,
    local_speech: splashdown_config::defaults::LOCAL_SPEECH,
    openai_ai: splashdown_config::defaults::OPENAI_AI,
    openrouter_ai: splashdown_config::defaults::OPENROUTER_AI,
    anthropic_ai: splashdown_config::defaults::ANTHROPIC_AI,
    sound_volume: splashdown_config::DEFAULT_SOUND_VOLUME,
};

#[derive(Serialize)]
pub struct KeyStatus {
    name: KeyName,
    source: Option<KeySource>,
    /// Last four characters, so the user can tell which key it is.
    hint: Option<String>,
}

#[derive(Serialize)]
pub struct ModelStatus {
    id: &'static str,
    mb: u32,
    downloaded: bool,
    /// Short description (built-in text models only).
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'static str>,
    /// Memory while loaded (built-in text models only).
    #[serde(skip_serializing_if = "Option::is_none")]
    ram_mb: Option<u32>,
    /// Speech models: the license shown next to them.
    #[serde(skip_serializing_if = "Option::is_none")]
    license: Option<&'static str>,
    /// Speech models that only understand English.
    english_only: bool,
}

#[derive(Serialize)]
pub struct KeyOption {
    value: PhysicalKey,
    label: &'static str,
}

#[derive(Serialize)]
pub struct Permissions {
    accessibility: bool,
    /// "granted", "denied" or "not-asked".
    microphone: &'static str,
}

#[derive(Serialize)]
pub struct Spending {
    month: crate::spend::Period,
    today: crate::spend::Period,
    state: crate::spend::BudgetState,
    /// When the built-in price list was last checked.
    prices_as_of: &'static str,
}

#[derive(Serialize)]
pub struct Snapshot {
    settings: Settings,
    keys: Vec<KeyStatus>,
    permissions: Permissions,
    speech_models: Vec<ModelStatus>,
    /// Built-in local text models (`AiProvider::Local`).
    local_models: Vec<ModelStatus>,
    key_options: Vec<KeyOption>,
    /// What `auto` resolves to right now, or why it can't.
    resolved_speech: Result<String, String>,
    resolved_ai: Result<String, String>,
    /// Where words go with the current settings.
    speech_destination: Destination,
    ai_destination: Destination,
    /// The cloud model that handles instructions while cleanup runs
    /// locally ("OpenRouter · model"), if any.
    resolved_instructions: Option<String>,
    /// Whether "send instructions to the cloud" could apply: cleanup is
    /// local and a cloud key is saved.
    can_route_instructions: bool,
    engine_error: Option<String>,
    /// What cloud providers have cost, and where that stands against the
    /// budget.
    spending: Spending,
    /// Apple's on-device models; `None` off macOS.
    apple: Option<AppleStatus>,
    /// The presets in effect (built-ins until edited).
    presets: Vec<splashdown_config::Preset>,
    /// The transforms in effect (built-ins until edited).
    transforms: Vec<splashdown_config::Transform>,
    defaults: Defaults,
    platform: &'static str,
    version: &'static str,
    config_path: String,
    /// New releases (Settings → About).
    update: crate::updates::Status,
    /// AI providers a preset can choose as its own model, usable now.
    preset_ai_options: Vec<AiOption>,
    /// The speech model the user chose is still loading.
    speech_loading: bool,
    /// Presets (by id) and transforms (`transform:` and id) whose shortcut
    /// couldn't be registered, and why.
    shortcut_errors: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
struct AiOption {
    value: AiProvider,
    label: &'static str,
}

/// The AI providers that would work right now for a preset's own model.
fn preset_ai_options(
    settings: &Settings,
    secrets: &Secrets,
    on_device: Situation,
) -> Vec<AiOption> {
    let cloud = !settings.general.local_only;
    let options = [
        (
            AiProvider::Apple,
            "Apple on-device",
            on_device.apple_language_model,
        ),
        (
            AiProvider::Local,
            "Built-in local model",
            on_device.local_model,
        ),
        (
            AiProvider::Openai,
            "OpenAI",
            cloud && secrets.openai.is_some(),
        ),
        (
            AiProvider::Openrouter,
            "OpenRouter",
            cloud && secrets.openrouter.is_some(),
        ),
        (
            AiProvider::Anthropic,
            "Anthropic (Claude)",
            cloud && secrets.anthropic.is_some(),
        ),
        (
            AiProvider::Custom,
            "Local server",
            settings.ai.base_url.is_some() || settings.ai.provider == AiProvider::Custom,
        ),
    ];
    options
        .into_iter()
        .filter(|(_, _, usable)| *usable)
        .map(|(value, label, _)| AiOption { value, label })
        .collect()
}

fn snapshot(
    mut settings: Settings,
    engine_error: Option<String>,
    update: crate::updates::Status,
) -> Snapshot {
    // An older id for a local speech model shows (and is saved) as today's.
    if settings.speech.provider == SpeechProvider::WhisperLocal
        && let Some(model) = settings
            .speech
            .model
            .as_deref()
            .and_then(splashdown_providers::SpeechModel::from_id)
    {
        settings.speech.model = Some(model.id().to_string());
    }
    let presets = settings.presets();
    let transforms = settings.transforms();
    let budget_state =
        crate::spend::shared().budget_state(&settings.general.budget, chrono::Local::now());
    let secrets = Secrets::load();
    let paths = engine::paths().ok();
    let models_dir = paths.as_ref().map(|p| p.models_dir.clone());
    let perms = splashdown_platform::permissions();
    let apple = splashdown_providers::apple::status(settings.general.language.as_deref());
    let on_device = Situation {
        local_model: models_dir
            .as_deref()
            .is_some_and(splashdown_providers::factory::local_model_available),
        local_speech: models_dir.as_deref().is_some_and(|d| {
            SpeechModel::from_id(splashdown_config::defaults::LOCAL_SPEECH)
                .is_some_and(|m| m.is_downloaded(d))
        }),
        over_budget: engine::budget_reached(&settings),
        ..apple
            .as_ref()
            .map_or_else(Default::default, |a| a.on_device())
    };

    let preset_ai_options = preset_ai_options(&settings, &secrets, on_device);
    let keys = KeyName::ALL
        .into_iter()
        .map(|name| KeyStatus {
            name,
            source: splashdown_config::key_source(name),
            hint: secrets.get(name).map(|k| {
                let tail: String = k.chars().rev().take(4).collect();
                tail.chars().rev().collect()
            }),
        })
        .collect();

    let speech_models = SpeechModel::ALL
        .into_iter()
        .map(|m| ModelStatus {
            id: m.id(),
            mb: m.approx_mb(),
            downloaded: models_dir.as_ref().is_some_and(|d| m.is_downloaded(d)),
            label: Some(m.label()),
            ram_mb: None,
            license: Some(m.license()),
            english_only: m.english_only(),
        })
        .collect();
    let local_models = LocalModel::ALL
        .into_iter()
        .map(|m| ModelStatus {
            id: m.id(),
            mb: m.approx_mb(),
            downloaded: models_dir.as_ref().is_some_and(|d| m.is_downloaded(d)),
            label: Some(m.label()),
            ram_mb: Some(m.approx_ram_mb()),
            license: None,
            english_only: false,
        })
        .collect();

    let key_options = [
        PhysicalKey::RightAlt,
        PhysicalKey::RightMeta,
        PhysicalKey::RightControl,
        PhysicalKey::RightShift,
        PhysicalKey::LeftAlt,
        PhysicalKey::LeftMeta,
        PhysicalKey::LeftControl,
        PhysicalKey::LeftShift,
        PhysicalKey::Fn,
        PhysicalKey::MouseBack,
        PhysicalKey::MouseForward,
    ]
    .into_iter()
    // Fn and the mouse buttons are seen only by the macOS key listener.
    .filter(|k| cfg!(target_os = "macos") || !(*k == PhysicalKey::Fn || k.is_mouse()))
    .map(|value| KeyOption {
        value,
        label: value.label(),
    })
    .collect();

    let speech = settings.resolve_speech_with(&secrets, on_device);
    let ai = settings.resolve_ai_with(&secrets, on_device);
    let speech_destination = speech
        .as_ref()
        .map_or(Destination::Off, SpeechChoice::destination);
    let ai_destination = match &ai {
        Ok(Some(c)) => c.destination(),
        Ok(None) | Err(_) => Destination::Off,
    };
    let resolved_speech = speech
        .map(|c| with_model(speech_label(c.provider), &c.model))
        .map_err(|e| e.to_string());
    let resolved_ai = ai
        .map(|c| match c {
            Some(c) => with_model(ai_label(c.provider), &c.model),
            None => "Off".to_string(),
        })
        .map_err(|e| e.to_string());

    let resolved_instructions = settings
        .resolve_instruction_ai_with(&secrets, on_device)
        .map(|c| with_model(ai_label(c.provider), &c.model));
    let can_route_instructions = {
        let mut on = settings.clone();
        on.ai.cloud_for_instructions = true;
        on.resolve_instruction_ai_with(&secrets, on_device)
            .is_some()
    };

    Snapshot {
        settings,
        keys,
        permissions: Permissions {
            accessibility: perms.accessibility,
            microphone: match perms.microphone {
                Access::Granted => "granted",
                Access::Denied => "denied",
                Access::NotAsked => "not-asked",
            },
        },
        speech_models,
        local_models,
        key_options,
        resolved_speech,
        resolved_ai,
        speech_destination,
        ai_destination,
        resolved_instructions,
        can_route_instructions,
        engine_error,
        spending: Spending {
            month: crate::spend::shared().this_month(chrono::Local::now()),
            today: crate::spend::shared().today(chrono::Local::now()),
            state: budget_state,
            prices_as_of: splashdown_providers::pricing::PRICES_AS_OF,
        },
        apple,
        presets,
        transforms,
        defaults: DEFAULTS,
        platform: splashdown_platform::backend_name(),
        version: env!("CARGO_PKG_VERSION"),
        config_path: paths
            .map(|p| p.config_file().display().to_string())
            .unwrap_or_default(),
        update,
        preset_ai_options,
        shortcut_errors: crate::shortcuts::errors(),
        speech_loading: splashdown_providers::speech_model_loading(),
    }
}

/// "Provider · model", or just the provider when the OS picks the model.
fn with_model(label: &str, model: &str) -> String {
    if model == splashdown_config::defaults::SYSTEM_MODEL {
        label.to_string()
    } else {
        format!("{label} · {model}")
    }
}

fn ai_label(p: AiProvider) -> &'static str {
    match p {
        AiProvider::Auto => "Auto",
        AiProvider::Openai => "OpenAI",
        AiProvider::Openrouter => "OpenRouter",
        AiProvider::Anthropic => "Anthropic",
        AiProvider::Custom => "Local server",
        AiProvider::Apple => "Apple on-device",
        AiProvider::Local => "Built-in local model",
        AiProvider::Off => "Off",
    }
}

fn speech_label(p: SpeechProvider) -> &'static str {
    match p {
        SpeechProvider::Auto => "Auto",
        SpeechProvider::Openai => "OpenAI",
        SpeechProvider::Openrouter => "OpenRouter",
        SpeechProvider::WhisperLocal => "On this computer",
        SpeechProvider::Apple => "Apple on-device",
    }
}

/// Async so it runs off the main thread: checking Apple's models takes a
/// few tens of milliseconds, and Settings polls this.
#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> Result<Snapshot, String> {
    Ok(state.snapshot())
}

#[derive(Serialize)]
pub struct Saved {
    state: Snapshot,
    /// Set when the new settings can't be used yet (missing key, model not
    /// downloaded); they're still saved.
    engine_error: Option<String>,
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
    if let Some(problem) = crate::replacements::check(&settings.replacements) {
        return Err(problem);
    }
    // The notes folder is changed from the Notes window, so a Settings
    // window opened earlier mustn't put the old one back.
    let mut settings = settings;
    let current = state.settings();
    settings.notes.folder = current.notes.folder;
    // Set by the first meeting, which Settings may not have seen.
    settings.notes.meeting_reminded |= current.notes.meeting_reminded;
    settings.save(&engine::paths()?.config_file())?;
    show_settings(&app, &state, &settings);
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings.clone();
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
    })
}

/// Brings what settings show outside the engine up to date: the menu bar,
/// the mini bar and global shortcuts.
pub fn show_settings(app: &AppHandle, state: &AppState, settings: &Settings) {
    state
        .tray
        .show_language(settings.general.language.as_deref());
    state.tray.show_whisper(settings.general.whisper_mode());
    state.tray.show_mini_bar(settings.general.mini_bar());
    crate::minibar::set_visible(settings.general.mini_bar());
    crate::shortcuts::sync(app, &settings.presets(), &settings.transforms());
}

/// Settings were saved from somewhere other than the Settings window (the
/// menu bar, a meeting's question, the call offer): an open Settings
/// window reads them again rather than saving its older copy over them.
pub fn settings_changed_elsewhere(app: &AppHandle) {
    let _ = app.emit_to("settings", "settings-reloaded", ());
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

#[derive(Clone, Serialize)]
struct DownloadProgress {
    id: String,
    done: u64,
    total: Option<u64>,
}

#[tauri::command]
pub async fn download_model(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Saved, String> {
    if SpeechModel::from_id(&id).is_none() && LocalModel::from_id(&id).is_none() {
        return Err("Unknown model.".into());
    }
    // Two downloads of one model would write the same partial file.
    static DOWNLOADING: Mutex<Vec<String>> = Mutex::new(Vec::new());
    struct Downloading(String);
    impl Drop for Downloading {
        fn drop(&mut self) {
            DOWNLOADING
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|d| *d != self.0);
        }
    }
    {
        let mut downloading = DOWNLOADING.lock().unwrap_or_else(|e| e.into_inner());
        if downloading.contains(&id) {
            return Err("That model is already downloading.".into());
        }
        downloading.push(id.clone());
    }
    let _downloading = Downloading(id.clone());
    let dir = engine::paths()?.models_dir;
    let mut last_emit = Instant::now();
    splashdown_providers::models::download_by_id(&id, &dir, |done, total| {
        // Throttle progress events to ~10 per second.
        if last_emit.elapsed().as_millis() >= 100 || total == Some(done) {
            last_emit = Instant::now();
            let _ = app.emit(
                "model-download",
                DownloadProgress {
                    id: id.clone(),
                    done,
                    total,
                },
            );
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
    })
}

/// Downloads Apple's speech model for the configured language (from
/// Apple), then rebuilds the engine so "Automatic" can pick it up.
#[tauri::command]
pub async fn prepare_apple_speech(state: State<'_, AppState>) -> Result<Saved, String> {
    let language = state.settings().general.language;
    splashdown_providers::apple::prepare_speech(language.as_deref()).await?;
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
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

/// "Install and restart": downloads the update found by the last check,
/// installs it and relaunches. Returns only if that fails.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    crate::updates::install(&app).await
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

/// Opens the microphone for a moment, which makes macOS show its
/// microphone prompt now rather than during the first dictation.
#[tauri::command]
pub async fn request_microphone() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let recorder = splashdown_audio::Recorder::start(None).map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        recorder.stop();
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_history(query: String, state: State<'_, AppState>) -> Vec<Entry> {
    state.history.list(&query)
}

#[derive(Serialize)]
pub struct Retranscribed {
    /// The new final text.
    text: String,
    /// What the speech model heard.
    raw: String,
    stt: String,
}

/// Transcribes a kept recording again with the current speech-to-text and
/// AI settings, and returns the result (the history entry is unchanged).
#[tauri::command]
pub async fn retranscribe_history(
    at: u64,
    state: State<'_, AppState>,
) -> Result<Retranscribed, String> {
    let entry = state.history.find(at).ok_or("That entry is gone.")?;
    if entry.mode == splashdown_core::Mode::Transform {
        return Err("Edits of selected text can't be redone: the selection isn't stored.".into());
    }
    let (speech, instruction) = state
        .history
        .audio(at)
        .ok_or("The recording for this entry wasn't kept.")?;
    let settings = state.settings();
    let engine = tauri::async_runtime::spawn_blocking(move || engine::build(&settings))
        .await
        .map_err(|e| e.to_string())??;
    let outcome = engine
        .pipeline
        .run(splashdown_core::AudioJob {
            mode: entry.mode,
            speech,
            instruction_speech: instruction,
            selection: None,
            context: engine.context.clone(),
            hints: engine.hints.clone(),
            standing_instruction: None,
            prefix: None,
        })
        .await
        .map_err(|e| e.to_string())?;
    crate::spend::record_outcome(&outcome);
    Ok(Retranscribed {
        text: outcome.text,
        raw: outcome.raw,
        stt: engine.pipeline.stt_name(),
    })
}

#[tauri::command]
pub fn delete_history(at: u64, state: State<'_, AppState>) -> Result<(), String> {
    state.history.delete(at).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pin_history(at: u64, pinned: bool, state: State<'_, AppState>) -> Result<(), String> {
    state.history.pin(at, pinned).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    state.history.clear().map_err(|e| e.to_string())
}

/// Copies text from the Settings window (the webview's own clipboard API
/// needs extra permissions).
#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    splashdown_platform::copy_to_clipboard(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_recent_apps(state: State<'_, AppState>) -> Vec<crate::history::RecentApp> {
    state.history.recent_apps()
}

/// Re-runs the AI pass on a history entry's words with another preset and
/// returns the new text (nothing is inserted).
#[tauri::command]
pub async fn redo_history(
    at: u64,
    preset_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let entry = state.history.find(at).ok_or("That entry is gone.")?;
    if entry.mode == splashdown_core::Mode::Transform {
        return Err("Transforms can't be redone: the original selection isn't stored.".into());
    }
    let session = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || {
        session.redo(entry.raw, entry.instruction, preset_id)
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
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
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
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, info.clone());
    info
}

/// The most recent dictation (kept even when history is off). The setup
/// guide uses it to notice the first successful dictation.
#[tauri::command]
pub fn last_result(state: State<'_, AppState>) -> Option<Entry> {
    state.history.last()
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

/// Marks setup as done, applies "start at login", and closes the guide.
#[tauri::command]
pub async fn finish_onboarding(
    launch_at_login: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut settings = state.settings();
    settings.general.onboarded = true;
    settings.save(&engine::paths()?.config_file())?;
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
    if let Err(e) = set_launch_at_login(launch_at_login, app.clone()) {
        tracing::warn!("couldn't change launch at login: {e}");
    }
    if let Some(w) = app.get_webview_window(crate::tray::ONBOARDING_WINDOW) {
        let _ = w.close();
    }
    Ok(())
}

#[tauri::command]
pub fn open_onboarding(app: AppHandle) {
    crate::tray::open_onboarding(&app);
}

#[tauri::command]
pub fn get_vocab_suggestions(state: State<'_, AppState>) -> Vec<String> {
    state.history.suggestions()
}

/// Removes a suggestion (after it was added to the vocabulary, or dismissed).
#[tauri::command]
pub fn dismiss_vocab_suggestion(word: String, state: State<'_, AppState>) {
    state.history.remove_suggestion(&word);
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
    let automatic = splashdown_audio::devices::choose(&[], &inputs)
        .or_else(|| inputs.iter().find(|i| i.is_default).map(|i| i.name.clone()));
    Microphones {
        devices: inputs
            .into_iter()
            .map(|i| Microphone {
                name: i.name,
                bluetooth: i.bluetooth,
                built_in: i.built_in,
                is_default: i.is_default,
            })
            .collect(),
        automatic,
    }
}
