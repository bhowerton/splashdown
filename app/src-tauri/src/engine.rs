//! Builds the processing engine from settings and keys. Used at startup and
//! whenever Settings changes something.

use std::collections::HashMap;
use std::sync::Arc;

use splashdown_config::{
    AiProvider, Budget, Destination, OtherAudio, Paths, Preset, PresetModel, Secrets, Settings,
    Situation, SpeechProvider,
};
use splashdown_core::fallback::{FallbackModel, FallbackStt};
use splashdown_core::{Pipeline, PromptContext, SpeechToText, SttHints, TextModel};
use splashdown_providers::SpeechModel;

use crate::session::Keys;

/// Everything needed to process a recording.
pub struct Engine {
    pub pipeline: Pipeline,
    /// Pipelines for presets with their own AI model, by preset id.
    preset_pipelines: HashMap<String, Pipeline>,
    /// The spending limits, checked after each dictation.
    pub budget: Budget,
    /// Built over the limit: on-device models only.
    pub over_budget: bool,
    pub context: PromptContext,
    pub hints: SttHints,
    /// Local model for the live preview, if one is available and enabled.
    pub preview: Option<Arc<dyn SpeechToText>>,
    /// Save results to the history file (otherwise only the latest is kept,
    /// in memory).
    pub keep_history: bool,
    /// Also keep each recording with its history entry.
    pub keep_audio: bool,
    /// Place the overlay by the text cursor.
    pub overlay_near_cursor: bool,
    /// Raise quiet speech before transcribing it.
    pub whisper_mode: bool,
    /// Remove background noise before transcribing.
    pub noise_reduction: bool,
    /// Count words and speaking time for Insights.
    pub keep_insights: bool,
    /// Find-and-replace rules for the final text.
    pub replacements: crate::replacements::Replacements,
    /// Per-app presets, in the order shown in Settings (number keys pick
    /// them while recording).
    pub presets: Vec<Preset>,
    /// Saved instructions for Notes and transform shortcuts.
    pub transforms: Vec<splashdown_config::Transform>,
    /// Watch for corrections after inserting and learn the corrected names.
    pub learn_vocabulary: bool,
    /// Tell the AI which app the text is going into.
    pub share_app_name: bool,
    /// Send recent dictations into the same app along with the next one.
    pub share_recent: bool,
    /// Compose may read the conversation around the field.
    pub compose_reads_window: bool,
    /// Names and terms on screen join each dictation's vocabulary.
    pub screen_words: bool,
    /// Compose writes in the voice of the user's messages on screen, and
    /// follows the style learned from their edits.
    pub compose_style: bool,
    /// Keep what the user sends instead of Compose's drafts, to learn
    /// their style.
    pub learn_style: bool,
    /// Compose shows its draft in a panel instead of writing it in.
    pub compose_panel: bool,
    /// Type the AI's text as it streams in.
    pub stream_typing: bool,
    /// Quiet other sounds while recording.
    pub quiet_others: Option<splashdown_platform::Quieten>,
    /// Play sounds when recording starts and stops.
    pub sounds: bool,
    pub sound_volume: f32,
    /// Preferred microphones in order (none: automatic).
    pub microphones: Vec<String>,
    /// Voice actions: whether they're on, and which.
    pub actions: splashdown_config::ActionSettings,
}

impl Engine {
    /// The pipeline for a preset: its own AI model if it has one that's
    /// usable, otherwise the main pipeline.
    pub fn pipeline_for(&self, preset: &Preset) -> &Pipeline {
        self.preset_pipelines
            .get(&preset.id)
            .unwrap_or(&self.pipeline)
    }

    /// Turns a recording into what speech-to-text gets (see
    /// `splashdown_audio::prepare`): background noise removed if noise
    /// reduction is on, then quiet speech raised in whisper mode.
    pub fn prepare_audio(
        &self,
        clip: &splashdown_core::AudioClip,
    ) -> Option<splashdown_core::AudioClip> {
        splashdown_audio::prepare_with(clip, self.audio_cleanup())
    }

    /// The optional clean-up steps `prepare_audio` applies.
    pub fn audio_cleanup(&self) -> splashdown_audio::Cleanup {
        splashdown_audio::Cleanup {
            denoise: self.noise_reduction,
            quiet: self.whisper_mode,
        }
    }

    /// Speech-to-text hints for a preset: its own spoken language, if set.
    pub fn hints_for(&self, preset: &Preset) -> SttHints {
        let mut hints = self.hints.clone();
        if let Some(language) = preset.language.as_ref().filter(|l| !l.is_empty()) {
            hints.language = Some(language.clone());
        }
        hints
    }
}

pub fn paths() -> Result<Paths, String> {
    Paths::resolve().ok_or_else(|| "couldn't find the home folder".to_string())
}

pub fn load_settings() -> Settings {
    let Ok(paths) = paths() else {
        return Settings::default();
    };
    let mut settings = Settings::load(&paths.config_file()).unwrap_or_else(|e| {
        tracing::warn!("ignoring config: {e}");
        Settings::default()
    });
    repair(&mut settings);
    settings
}

/// Settings an earlier version saved wrong. 0.2 and 0.3 saved a built-in
/// text model's id as the speech model when it was downloaded in Settings,
/// which stopped speech-to-text: back to the default speech model.
fn repair(settings: &mut Settings) {
    if settings
        .speech
        .model
        .as_deref()
        .is_some_and(|m| splashdown_providers::LocalModel::from_id(m).is_some())
    {
        tracing::info!("the speech model was set to a text model: back to the default");
        settings.speech.model = None;
    }
}

pub fn keys(settings: &Settings) -> Keys {
    Keys {
        dictation: settings.hotkeys.dictation_key(),
        instruction: settings.hotkeys.instruction_key(),
        hands_free: settings.hotkeys.hands_free_key(),
        double_tap: settings.hotkeys.double_tap(),
        note: settings.hotkeys.note_key(),
        ask: settings.hotkeys.ask_key(),
        compose: settings
            .hotkeys
            .compose_key()
            .unwrap_or(splashdown_core::gesture::PhysicalKey::Other),
    }
}

/// Which of Apple's on-device models this Mac can use for `settings`.
pub fn on_device(settings: &Settings) -> Situation {
    let models_dir = paths().map(|p| p.models_dir).unwrap_or_default();
    Situation {
        over_budget: budget_reached(settings),
        ..splashdown_providers::factory::on_device(
            settings.general.language.as_deref(),
            &models_dir,
        )
    }
}

/// Whether a spending limit is reached right now.
pub fn budget_reached(settings: &Settings) -> bool {
    crate::spend::shared().budget_state(&settings.general.budget, chrono::Local::now())
        == crate::spend::BudgetState::Reached
}

/// Builds providers for `settings`. Loading a local Whisper model takes a
/// second or two, so call this off the UI thread.
pub fn build(settings: &Settings) -> Result<Engine, String> {
    let paths = paths()?;
    let secrets = Secrets::load();
    let on_device = on_device(settings);

    let speech = settings
        .resolve_speech_with(&secrets, on_device)
        .map_err(|e| e.to_string())?;
    let stt = splashdown_providers::factory::speech(&speech, &secrets, &paths.models_dir)
        .map_err(|e| e.to_string())?;
    // The live preview uses the speech model the user chose, when it runs on
    // this computer. No other model is loaded for it.
    let preview = (settings.general.live_preview()
        && speech.provider == SpeechProvider::WhisperLocal)
        .then(|| stt.clone());
    // A cloud provider that fails hands the request to one on this
    // computer, if there is one, rather than losing the dictation.
    let stt = match (
        speech.destination(),
        on_device_speech(settings, &secrets, on_device, &paths),
    ) {
        (Destination::Cloud, Some(backup)) => {
            tracing::info!("speech falls back to {}", backup.name());
            Arc::new(FallbackStt {
                primary: stt,
                backup,
            }) as Arc<dyn SpeechToText>
        }
        _ => stt,
    };
    let local_llm = on_device_text_model(settings, &secrets, on_device);
    match &local_llm {
        Some(m) => tracing::info!("cloud AI falls back to {}", m.name()),
        None => tracing::info!("no AI on this computer to fall back to"),
    }
    let ai_choice = settings
        .resolve_ai_with(&secrets, on_device)
        .map_err(|e| e.to_string())?;
    let llm = ai_choice
        .as_ref()
        .map(|choice| {
            splashdown_providers::factory::text_model(choice, &secrets)
                .map(|llm| with_backup(llm, choice.destination(), &local_llm))
        })
        .transpose()
        .map_err(|e| e.to_string())?;
    let instruction_llm = settings
        .resolve_instruction_ai_with(&secrets, on_device)
        .map(|choice| {
            splashdown_providers::factory::text_model(&choice, &secrets)
                // Instructions go to the cloud only when cleanup is local,
                // so the main model is the stand-in.
                .map(|llm| with_backup(llm, choice.destination(), &local_llm))
        })
        .transpose()
        .map_err(|e| e.to_string())?;
    tracing::info!(
        "speech: {} · ai: {}{}",
        stt.name(),
        llm.as_ref().map_or("off".to_string(), |l| l.name()),
        instruction_llm
            .as_ref()
            .map(|l| format!(" · instructions: {}", l.name()))
            .unwrap_or_default()
    );

    let presets = settings.presets();
    let mut preset_pipelines = HashMap::new();
    for preset in presets.iter().filter(|p| p.ai) {
        let Some(choice) = &preset.model else {
            continue;
        };
        match preset_model(settings, &secrets, on_device, choice, &local_llm) {
            Ok(Some(llm)) => {
                tracing::info!("preset {}: ai: {}", preset.id, llm.name());
                preset_pipelines.insert(
                    preset.id.clone(),
                    Pipeline::new(stt.clone(), Some(llm)).with_actions(settings.actions.enabled),
                );
            }
            // Off, or nothing usable: the main pipeline.
            Ok(None) => {}
            Err(e) => tracing::warn!("preset {} keeps the main AI model: {e}", preset.id),
        }
    }

    let vocabulary = settings.general.vocabulary.clone();
    Ok(Engine {
        budget: settings.general.budget,
        over_budget: on_device.over_budget,
        preview,
        keep_history: settings.general.keep_history(),
        keep_audio: settings.general.keep_audio(),
        overlay_near_cursor: settings.general.overlay_near_cursor(),
        whisper_mode: settings.general.whisper_mode(),
        noise_reduction: settings.general.noise_reduction(),
        keep_insights: settings.general.keep_insights(),
        replacements: crate::replacements::Replacements::new(&settings.replacements),
        learn_vocabulary: settings.general.learn_vocabulary(),
        share_app_name: settings.general.share_app_name(),
        // Apple's on-device model repeats and even obeys earlier text
        // (eval: earlier-*), so it never gets any.
        quiet_others: match settings.general.other_sounds() {
            OtherAudio::Leave => None,
            OtherAudio::Lower => Some(splashdown_platform::Quieten::Lower),
            OtherAudio::Mute => Some(splashdown_platform::Quieten::Mute),
            OtherAudio::Pause => Some(splashdown_platform::Quieten::Pause),
        },
        compose_reads_window: settings.general.compose_reads_window(),
        compose_panel: settings.general.compose_panel(),
        compose_style: settings.general.compose_style(),
        learn_style: settings.general.learn_style(),
        screen_words: settings.general.screen_words(),
        share_recent: settings.general.share_recent()
            && ai_choice
                .as_ref()
                .is_none_or(|c| c.provider != AiProvider::Apple),
        stream_typing: settings.general.stream_typing(),
        sounds: settings.general.sounds(),
        sound_volume: settings.general.sound_volume(),
        microphones: settings.general.microphones(),
        actions: settings.actions.clone(),
        presets,
        transforms: settings.transforms(),
        preset_pipelines,
        pipeline: Pipeline::new(stt, llm)
            .with_instruction_model(instruction_llm)
            .with_actions(settings.actions.enabled),
        context: PromptContext {
            user_name: settings.general.user_name.clone(),
            vocabulary: vocabulary.clone(),
            // Only to a cloud model: the built-in one (and, untested,
            // Apple's) turns the ordinary word into the name too ("the TRUX
            // are blocking"), which is worse than missing the name.
            misheard: if ai_choice.as_ref().is_some_and(|c| {
                !matches!(
                    c.provider,
                    AiProvider::Local | AiProvider::Apple | AiProvider::Custom
                )
            }) {
                settings
                    .general
                    .misheard
                    .iter()
                    .flat_map(|(word, heard)| heard.iter().map(move |h| (word.clone(), h.clone())))
                    .collect()
            } else {
                Vec::new()
            },
            snippets: settings.snippets.clone(),
            mixed_languages: SttHints {
                language: settings.general.language.clone(),
                languages: settings.general.languages.clone(),
                ..SttHints::default()
            }
            .mixed_languages(),
            ..PromptContext::default()
        },
        hints: SttHints {
            vocabulary,
            language: settings.general.language.clone(),
            languages: settings.general.languages.clone(),
        },
    })
}

/// A preset's own AI model, resolved like the main one (keys, local-only
/// mode and the spending limit all apply).
fn preset_model(
    settings: &Settings,
    secrets: &Secrets,
    on_device: Situation,
    choice: &PresetModel,
    local_llm: &Option<Arc<dyn TextModel>>,
) -> Result<Option<Arc<dyn TextModel>>, String> {
    let mut s = settings.clone();
    s.ai.provider = choice.provider;
    s.ai.model = choice.model.clone().filter(|m| !m.trim().is_empty());
    s.resolve_ai_with(secrets, on_device)
        .map_err(|e| e.to_string())?
        .map(|c| {
            splashdown_providers::factory::text_model(&c, secrets)
                .map(|llm| with_backup(llm, c.destination(), local_llm))
        })
        .transpose()
        .map_err(|e| e.to_string())
}

/// `llm`, falling back to the on-device model when it's a cloud one.
fn with_backup(
    llm: Arc<dyn TextModel>,
    destination: Destination,
    local: &Option<Arc<dyn TextModel>>,
) -> Arc<dyn TextModel> {
    match (destination, local) {
        (Destination::Cloud, Some(backup)) => Arc::new(FallbackModel {
            primary: llm,
            backup: backup.clone(),
        }),
        _ => llm,
    }
}

/// The AI model on this computer, if there is one: Apple's, else the
/// built-in one (its helper only starts if it's ever used).
fn on_device_text_model(
    settings: &Settings,
    secrets: &Secrets,
    on_device: Situation,
) -> Option<Arc<dyn TextModel>> {
    let provider = if on_device.apple_language_model {
        AiProvider::Apple
    } else if on_device.local_model {
        AiProvider::Local
    } else {
        return None;
    };
    let mut s = settings.clone();
    s.ai.provider = provider;
    s.ai.model = None;
    let choice = s.resolve_ai_with(secrets, on_device).ok().flatten()?;
    splashdown_providers::factory::text_model(&choice, secrets).ok()
}

/// Speech-to-text on this computer for when the cloud fails: Apple's, else
/// a downloaded local model, which is only loaded if the cloud does fail.
fn on_device_speech(
    settings: &Settings,
    secrets: &Secrets,
    on_device: Situation,
    paths: &Paths,
) -> Option<Arc<dyn SpeechToText>> {
    if on_device.apple_speech {
        let mut s = settings.clone();
        s.speech.provider = SpeechProvider::Apple;
        s.speech.model = None;
        if let Some(apple) = s
            .resolve_speech_with(secrets, on_device)
            .ok()
            .and_then(|c| {
                splashdown_providers::factory::speech(&c, secrets, &paths.models_dir).ok()
            })
        {
            return Some(apple);
        }
    }
    // For a language other than English, an English-only model would turn
    // the speech into English-sounding words: prefer one that knows it.
    let other_language = settings
        .general
        .language
        .as_deref()
        .is_some_and(|l| !l.eq_ignore_ascii_case("en"));
    let downloaded = || {
        SpeechModel::ALL
            .into_iter()
            .filter(|m| m.is_downloaded(&paths.models_dir))
    };
    let model = downloaded()
        .find(|m| !(other_language && m.english_only()))
        .or_else(|| downloaded().next().filter(|_| !other_language))?;
    splashdown_providers::factory::local_speech_on_demand(model, &paths.models_dir).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_model_saved_as_the_speech_model_is_undone() {
        let mut settings = Settings::default();
        settings.speech.model = Some("qwen3-4b".into());
        repair(&mut settings);
        assert_eq!(settings.speech.model, None);

        settings.speech.model = Some("parakeet-v3".into());
        repair(&mut settings);
        assert_eq!(settings.speech.model.as_deref(), Some("parakeet-v3"));
    }
}
