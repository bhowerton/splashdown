//! What the Settings window and the setup guide show (`Snapshot`): the
//! settings with everything worked out from them, such as the provider
//! and model that actually run, keys, permissions, models and spending.

use serde::Serialize;
use splashdown_config::{
    AiProvider, Destination, KeyName, KeySource, Secrets, Settings, Situation, SpeechChoice,
    SpeechProvider,
};
use splashdown_core::gesture::PhysicalKey;
use splashdown_core::sync::Locked;
use splashdown_platform::Access;
use splashdown_providers::apple::AppleStatus;
use splashdown_providers::{LocalModel, SpeechModel};

use crate::engine;
use crate::models::*;

/// Built-in default models, so the UI can show what an empty field means.
#[derive(Serialize)]
pub struct Defaults {
    pub(super) openai_stt: &'static str,
    pub(super) openrouter_stt: &'static str,
    pub(super) local_speech: &'static str,
    pub(super) openai_ai: &'static str,
    pub(super) openrouter_ai: &'static str,
    pub(super) anthropic_ai: &'static str,
    pub(super) sound_volume: f32,
}

pub(super) const DEFAULTS: Defaults = Defaults {
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
    pub(super) name: KeyName,
    pub(super) source: Option<KeySource>,
    /// Last four characters, so the user can tell which key it is.
    pub(super) hint: Option<String>,
}

#[derive(Serialize)]
pub struct ModelStatus {
    pub(super) id: &'static str,
    pub(super) mb: u32,
    pub(super) downloaded: bool,
    /// Short description (built-in text models only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) label: Option<&'static str>,
    /// Memory while loaded (built-in text models only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ram_mb: Option<u32>,
    /// Speech models: the license shown next to them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) license: Option<&'static str>,
    /// Speech models that only understand English.
    pub(super) english_only: bool,
}

#[derive(Serialize)]
pub struct KeyOption {
    pub(super) value: PhysicalKey,
    pub(super) label: &'static str,
}

#[derive(Serialize)]
pub struct Permissions {
    pub(super) accessibility: bool,
    /// "granted", "denied" or "not-asked".
    pub(super) microphone: &'static str,
}

#[derive(Serialize)]
pub struct Spending {
    pub(super) month: crate::spend::Period,
    pub(super) today: crate::spend::Period,
    pub(super) state: crate::spend::BudgetState,
    /// When the built-in price list was last checked.
    pub(super) prices_as_of: &'static str,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub(super) settings: Settings,
    pub(super) keys: Vec<KeyStatus>,
    pub(super) permissions: Permissions,
    pub(super) speech_models: Vec<ModelStatus>,
    /// Built-in local text models (`AiProvider::Local`).
    pub(super) local_models: Vec<ModelStatus>,
    /// Models downloading now, so a window opened mid-download shows them.
    pub(super) downloads: Vec<DownloadProgress>,
    pub(super) key_options: Vec<KeyOption>,
    /// What `auto` resolves to right now, or why it can't.
    pub(super) resolved_speech: Result<String, String>,
    pub(super) resolved_ai: Result<String, String>,
    /// Where words go with the current settings.
    pub(super) speech_destination: Destination,
    pub(super) ai_destination: Destination,
    /// The cloud model that handles instructions while cleanup runs
    /// locally ("OpenRouter · model"), if any.
    pub(super) resolved_instructions: Option<String>,
    /// Whether "send instructions to the cloud" could apply: cleanup is
    /// local and a cloud key is saved.
    pub(super) can_route_instructions: bool,
    pub(super) engine_error: Option<String>,
    /// What cloud providers have cost, and where that stands against the
    /// budget.
    pub(super) spending: Spending,
    /// Apple's on-device models; `None` off macOS.
    pub(super) apple: Option<AppleStatus>,
    /// The presets in effect (built-ins until edited).
    pub(super) presets: Vec<splashdown_config::Preset>,
    /// The transforms in effect (built-ins until edited).
    pub(super) transforms: Vec<splashdown_config::Transform>,
    pub(super) defaults: Defaults,
    pub(super) platform: &'static str,
    pub(super) version: &'static str,
    pub(super) config_path: String,
    /// New releases (Settings → About).
    pub(super) update: crate::updates::Status,
    /// What the organization set: Settings shows these as theirs, not
    /// the user's to change.
    pub(super) managed: splashdown_config::managed::Managed,
    /// AI providers a preset can choose as its own model, usable now.
    pub(super) preset_ai_options: Vec<AiOption>,
    /// The speech model the user chose is still loading.
    pub(super) speech_loading: bool,
    /// Presets (by id) and transforms (`transform:` and id) whose shortcut
    /// couldn't be registered, and why.
    pub(super) shortcut_errors: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
pub(super) struct AiOption {
    pub(super) value: AiProvider,
    pub(super) label: &'static str,
}

/// The AI providers that would work right now for a preset's own model.
pub(super) fn preset_ai_options(
    settings: &Settings,
    secrets: &Secrets,
    on_device: Situation,
) -> Vec<AiOption> {
    let cloud = !settings.general.local_only;
    let options = [
        (AiProvider::Apple, on_device.apple_language_model),
        (AiProvider::Local, on_device.local_model),
        (AiProvider::Openai, cloud && secrets.openai.is_some()),
        (
            AiProvider::Openrouter,
            cloud && secrets.openrouter.is_some(),
        ),
        (AiProvider::Anthropic, cloud && secrets.anthropic.is_some()),
        (
            AiProvider::Custom,
            settings.ai.base_url.is_some() || settings.ai.provider == AiProvider::Custom,
        ),
    ];
    options
        .into_iter()
        .filter(|(_, usable)| *usable)
        .map(|(value, _)| AiOption {
            value,
            label: value.label(),
        })
        .collect()
}

pub(super) fn snapshot(
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
        .map(|c| with_model(c.provider.label(), &c.model))
        .map_err(|e| e.to_string());
    let resolved_ai = ai
        .map(|c| match c {
            Some(c) => with_model(c.provider.label(), &c.model),
            None => "Off".to_string(),
        })
        .map_err(|e| e.to_string());

    let resolved_instructions = settings
        .resolve_instruction_ai_with(&secrets, on_device)
        .map(|c| with_model(c.provider.label(), &c.model));
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
        downloads: DOWNLOADS.locked().clone(),
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
        managed: splashdown_config::managed::Managed::current(),
        preset_ai_options,
        shortcut_errors: crate::shortcuts::errors(),
        speech_loading: splashdown_providers::speech_model_loading(),
    }
}

/// "Provider · model", or just the provider when the OS picks the model.
pub(super) fn with_model(label: &str, model: &str) -> String {
    if model == splashdown_config::defaults::SYSTEM_MODEL {
        label.to_string()
    } else {
        format!("{label} · {model}")
    }
}

/// What a command that changed something answers: the new snapshot, and
/// why dictation can't use it yet if it can't.
#[derive(Serialize)]
pub struct Saved {
    pub(crate) state: Snapshot,
    /// Set when the new settings can't be used yet (missing key, model not
    /// downloaded); they're still saved.
    pub(crate) engine_error: Option<String>,
}
