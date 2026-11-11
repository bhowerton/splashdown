//! `config.toml`: everything the user sets, one section per Settings tab.
//!
//! This file holds `Settings` itself (load, save, upgrade from older
//! files); each section is in `settings/`:
//!
//! | File | Section |
//! |---|---|
//! | `general.rs` | `[general]`: vocabulary, history, sounds, the overlay, privacy, Compose |
//! | `hotkeys.rs` | `[hotkeys]` |
//! | `providers.rs` | `[speech]` and `[ai]`, and the default models |
//! | `resolve.rs` | What "Automatic" comes to: the provider and model that run |
//! | `notes.rs` | `[notes]` |
//! | `actions.rs` | `[actions]` |
//! | `hooks.rs` | `[[hooks]]` |
//! | `replacements.rs` | `[[replacements]]` |
//!
//! A choice whose default may change later is an `Option`, unset (`None`)
//! meaning "the default", and has an accessor of the same name that says
//! what the default is (`general.sounds()`): read settings through those.
//! Only choices that can never change their default are plain values.
//!
//! ```toml
//! [speech]
//! provider = "auto"          # auto | openai | openrouter | whisper-local | apple
//!
//! [ai]
//! provider = "auto"          # auto | openai | openrouter | anthropic | custom | apple | local | off
//!
//! [hotkeys]
//! dictation = "right-option"   # see splashdown_core::gesture::PhysicalKey
//! instruction = "right-shift"
//!
//! [general]
//! user_name = "Alex"
//! vocabulary = ["Tauri", "OpenRouter"]
//! ```

mod actions;
mod general;
mod hooks;
mod hotkeys;
mod notes;
mod providers;
mod replacements;
mod resolve;

pub use actions::*;
pub use general::*;
pub use hooks::*;
pub use hotkeys::*;
pub use notes::*;
pub use providers::*;
pub use replacements::*;
pub use resolve::*;

use std::path::Path;

use splashdown_core::gesture::PhysicalKey;
use splashdown_core::spoken::Snippet;

use crate::presets::{Preset, builtin_presets};
use serde::{Deserialize, Serialize};

use crate::Secrets;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub speech: SpeechSettings,
    pub ai: AiSettings,
    pub general: GeneralSettings,
    pub hotkeys: HotkeySettings,
    /// Unset means the built-in presets; once edited, the full list is
    /// saved.
    pub presets: Option<Vec<Preset>>,
    /// Built-in presets added after a release that a saved preset list was
    /// given (`LATER_PRESETS`), so deleting one isn't undone.
    pub presets_given: Vec<String>,
    /// Saved text inserted when its trigger phrase is spoken.
    pub snippets: Vec<Snippet>,
    /// Find-and-replace rules applied to the final text.
    pub replacements: Vec<Replacement>,
    pub notes: NotesSettings,
    /// Saved instructions run over a note or selected text.
    /// Unset means the built-in ones; once edited, the full list is saved.
    pub transforms: Option<Vec<crate::Transform>>,
    pub actions: ActionSettings,
    /// Scripts, programs and web addresses run when something happens.
    pub hooks: Vec<Hook>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("couldn't read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
}

impl Settings {
    /// Why these hotkeys can't work together, if they can't: two keys or
    /// letters that are the same.
    pub fn hotkey_problem(&self) -> Option<&'static str> {
        let h = &self.hotkeys;
        if h.dictation_key() == h.instruction_key() {
            Some("The dictation and instruction keys must be different.")
        } else if h.note_key() == h.hands_free_key() {
            Some("The hands-free and note letters must be different.")
        } else if h.ask_key() == h.hands_free_key() || h.ask_key() == h.note_key() {
            Some("The ask letter must differ from the hands-free and note letters.")
        } else if h
            .compose_key()
            .is_some_and(|c| [h.hands_free_key(), h.note_key(), h.ask_key()].contains(&c))
        {
            Some("The compose letter must differ from the other letters.")
        } else if [
            Some(h.hands_free_key()),
            Some(h.note_key()),
            Some(h.ask_key()),
            h.compose_key(),
        ]
        .contains(&Some(h.translate_key()))
        {
            Some("The translate letter must differ from the other letters.")
        } else if [
            Some(h.hands_free_key()),
            Some(h.note_key()),
            Some(h.ask_key()),
            h.compose_key(),
            Some(h.translate_key()),
        ]
        .contains(&Some(h.draft_key()))
        {
            Some("The draft letter must differ from the other letters.")
        } else {
            None
        }
    }

    /// Forgets every "runs without asking": a hook's, an action's, a
    /// Shortcut's. For settings that came from a file rather than from what
    /// this user chose on this computer, so each asks before its first run.
    pub fn forget_trust(&mut self) {
        for hook in &mut self.hooks {
            hook.trusted = false;
        }
        for action in &mut self.actions.mine {
            action.trusted = false;
        }
        self.actions.trusted_shortcuts.clear();
    }

    /// Loads settings from `path`; a missing file means all defaults.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(LoadError::Io {
                path: path.display().to_string(),
                source,
            }),
        }
    }

    /// The presets in effect.
    pub fn presets(&self) -> Vec<Preset> {
        self.presets.clone().unwrap_or_else(builtin_presets)
    }

    pub fn transforms(&self) -> Vec<crate::Transform> {
        self.transforms
            .clone()
            .unwrap_or_else(crate::builtin_transforms)
    }

    /// The settings as `config.toml` text, e.g. to move them to another
    /// computer. API keys are never part of it (they're in the keychain).
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Reads `config.toml` text, e.g. a file exported on another computer.
    pub fn from_toml(text: &str) -> Result<Self, LoadError> {
        let mut settings: Self = toml::from_str(text)?;
        settings.upgrade();
        Ok(settings)
    }

    /// Settings saved by earlier versions, in today's terms.
    fn upgrade(&mut self) {
        // Other sounds are now turned down by default; a saved "leave"
        // under the old name was only ever the old default.
        let general = &mut self.general;
        if let Some(chosen) = general.legacy_other_audio.take()
            && general.other_sounds.is_none()
            && chosen != OtherAudio::Leave
        {
            general.other_sounds = Some(chosen);
        }
        // A saved preset list gets each later built-in once, in the place
        // it has in the built-in list.
        if let Some(presets) = &mut self.presets {
            let builtin = crate::presets::builtin_presets();
            for id in crate::presets::LATER_PRESETS {
                if self.presets_given.iter().any(|g| g == id) {
                    continue;
                }
                self.presets_given.push(id.to_string());
                if presets.iter().any(|p| p.id == *id) {
                    continue;
                }
                if let Some(at) = builtin.iter().position(|p| p.id == *id)
                    && let Some(preset) = builtin.get(at)
                {
                    presets.insert(at.min(presets.len()), preset.clone());
                }
            }
        }
    }

    /// Writes settings to `path`, creating its folder if needed. They go
    /// to a file beside it that then takes its place, so a crash or a full
    /// disk part way leaves the old settings, never half of the new ones.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let beside = beside(path, "tmp");
        let written = std::fs::File::create(&beside).and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        });
        match written.and_then(|()| std::fs::rename(&beside, path)) {
            Ok(()) => Ok(()),
            Err(_) => {
                let _ = std::fs::remove_file(&beside);
                // Something holds the file for a moment (a virus scanner,
                // on Windows): written in place instead.
                std::fs::write(path, text).map_err(|e| e.to_string())
            }
        }
    }

    /// Keeps a settings file that couldn't be read as `config.toml.bad`
    /// beside it, so the settings saved next don't destroy what the user
    /// had (a typo in a file edited by hand is fixed in a minute; presets
    /// and vocabulary aren't rebuilt in one). Where it was kept.
    pub fn keep_unreadable(path: &Path) -> std::io::Result<std::path::PathBuf> {
        let kept = beside(path, "bad");
        std::fs::copy(path, &kept)?;
        Ok(kept)
    }
}

/// `path` with `.suffix` added to its name: `config.toml.tmp`.
fn beside(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(openai: bool, openrouter: bool) -> Secrets {
        Secrets {
            openai: openai.then(|| "sk".into()),
            openrouter: openrouter.then(|| "or".into()),
            anthropic: None,
            custom: None,
        }
    }

    #[test]
    fn auto_prefers_openai_then_openrouter_then_local() {
        let s = Settings::default();
        assert_eq!(
            s.resolve_speech(&secrets(true, true)).unwrap().provider,
            SpeechProvider::Openai
        );
        assert_eq!(
            s.resolve_ai(&secrets(false, true))
                .unwrap()
                .unwrap()
                .provider,
            AiProvider::Openrouter
        );
        let none = secrets(false, false);
        assert_eq!(
            s.resolve_speech(&none).unwrap(),
            SpeechChoice {
                provider: SpeechProvider::WhisperLocal,
                model: "nemotron".into()
            }
        );
        assert_eq!(s.resolve_ai(&none).unwrap(), None);
    }

    #[test]
    fn explicit_provider_without_key_errors() {
        let mut s = Settings::default();
        s.ai.provider = AiProvider::Openai;
        assert_eq!(
            s.resolve_ai(&secrets(false, true)),
            Err(ResolveError::MissingKey("OpenAI", "OPENAI_API_KEY"))
        );
    }

    #[test]
    fn custom_needs_model_and_gets_default_url() {
        let mut s = Settings::default();
        s.ai.provider = AiProvider::Custom;
        assert_eq!(
            s.resolve_ai(&secrets(false, false)),
            Err(ResolveError::CustomNeedsModel)
        );
        s.ai.model = Some("llama3.2:3b".into());
        let c = s.resolve_ai(&secrets(false, false)).unwrap().unwrap();
        assert_eq!(c.base_url.as_deref(), Some(defaults::CUSTOM_BASE_URL));
    }

    #[test]
    fn local_only_refuses_cloud() {
        let mut s = Settings::default();
        s.general.local_only = true;
        let keys = secrets(true, true);
        assert_eq!(
            s.resolve_speech(&keys).unwrap().provider,
            SpeechProvider::WhisperLocal
        );
        assert_eq!(s.resolve_ai(&keys).unwrap(), None, "auto means AI off");

        s.ai.provider = AiProvider::Openrouter;
        assert!(matches!(
            s.resolve_ai(&keys),
            Err(ResolveError::LocalOnly(_))
        ));

        s.ai.provider = AiProvider::Custom;
        s.ai.model = Some("qwen2.5:3b".into());
        assert!(
            s.resolve_ai(&keys).unwrap().is_some(),
            "Ollama on localhost is fine"
        );
        s.ai.base_url = Some("https://api.groq.com/openai/v1".into());
        assert!(matches!(
            s.resolve_ai(&keys),
            Err(ResolveError::LocalOnly(_))
        ));
    }

    #[test]
    fn auto_prefers_keys_then_apple() {
        let s = Settings::default();
        let apple = Situation {
            apple_speech: true,
            apple_language_model: true,
            ..Situation::default()
        };
        assert_eq!(
            s.resolve_speech_with(&secrets(false, false), apple)
                .unwrap()
                .provider,
            SpeechProvider::Apple
        );
        assert_eq!(
            s.resolve_ai_with(&secrets(false, false), apple)
                .unwrap()
                .unwrap()
                .provider,
            AiProvider::Apple
        );
        assert_eq!(
            s.resolve_ai_with(&secrets(false, true), apple)
                .unwrap()
                .unwrap()
                .provider,
            AiProvider::Openrouter,
            "a saved key wins"
        );
        let mut local = Settings::default();
        local.general.local_only = true;
        assert_eq!(
            local
                .resolve_ai_with(&secrets(true, true), apple)
                .unwrap()
                .unwrap()
                .provider,
            AiProvider::Apple
        );
    }

    #[test]
    fn auto_prefers_the_built_in_speech_model_to_apples() {
        let s = Settings::default();
        let both = Situation {
            apple_speech: true,
            local_speech: true,
            ..Situation::default()
        };
        let pick = |s: &Settings, on: Situation| {
            s.resolve_speech_with(&secrets(false, false), on)
                .unwrap()
                .provider
        };
        assert_eq!(pick(&s, both), SpeechProvider::WhisperLocal);
        let mut local_only = s.clone();
        local_only.general.local_only = true;
        assert_eq!(pick(&local_only, both), SpeechProvider::WhisperLocal);
        let apple = Situation {
            apple_speech: true,
            ..Situation::default()
        };
        assert_eq!(pick(&s, apple), SpeechProvider::Apple, "not downloaded yet");
    }

    #[test]
    fn auto_uses_the_local_model_without_keys_then_apple() {
        let local = Situation {
            local_model: true,
            ..Situation::default()
        };
        let s = Settings::default();
        let pick = |secrets: &Secrets, on: Situation| {
            s.resolve_ai_with(secrets, on).unwrap().map(|c| c.provider)
        };
        assert_eq!(pick(&secrets(false, false), local), Some(AiProvider::Local));
        assert_eq!(
            pick(&secrets(false, true), local),
            Some(AiProvider::Openrouter),
            "a saved key wins"
        );
        let both = Situation {
            apple_language_model: true,
            ..local
        };
        assert_eq!(
            pick(&secrets(false, false), both),
            Some(AiProvider::Local),
            "the built-in model before Apple's"
        );
        let apple = Situation {
            apple_language_model: true,
            ..Situation::default()
        };
        assert_eq!(pick(&secrets(false, false), apple), Some(AiProvider::Apple));
        assert_eq!(pick(&secrets(false, false), Situation::default()), None);
        let choice = s
            .resolve_ai_with(&secrets(false, false), local)
            .unwrap()
            .unwrap();
        assert_eq!(choice.model, defaults::LOCAL_AI);
        assert_eq!(choice.destination(), Destination::Local);
    }

    #[test]
    fn instructions_can_go_to_the_cloud() {
        let apple = Situation {
            apple_speech: true,
            apple_language_model: true,
            ..Situation::default()
        };
        let mut s = Settings::default();
        s.ai.provider = AiProvider::Apple;
        assert_eq!(
            s.resolve_instruction_ai_with(&secrets(false, true), apple),
            None,
            "off by default"
        );

        s.ai.cloud_for_instructions = true;
        let cloud = s
            .resolve_instruction_ai_with(&secrets(false, true), apple)
            .unwrap();
        assert_eq!(cloud.provider, AiProvider::Openrouter);
        assert_eq!(cloud.model, defaults::OPENROUTER_AI);
        assert_eq!(
            s.resolve_instruction_ai_with(&secrets(false, false), apple),
            None,
            "no key"
        );

        s.general.local_only = true;
        assert_eq!(
            s.resolve_instruction_ai_with(&secrets(false, true), apple),
            None,
            "local only"
        );

        let mut cloud_main = Settings::default();
        cloud_main.ai.cloud_for_instructions = true;
        cloud_main.ai.provider = AiProvider::Openrouter;
        assert_eq!(
            cloud_main.resolve_instruction_ai_with(&secrets(false, true), apple),
            None,
            "cleanup already in the cloud"
        );
    }

    #[test]
    fn destinations() {
        let speech = |provider| SpeechChoice {
            provider,
            model: String::new(),
        };
        assert_eq!(
            speech(SpeechProvider::Apple).destination(),
            Destination::Local
        );
        assert_eq!(
            speech(SpeechProvider::WhisperLocal).destination(),
            Destination::Local
        );
        assert_eq!(
            speech(SpeechProvider::Openrouter).destination(),
            Destination::Cloud
        );

        let ai = |provider, base_url: Option<&str>| AiChoice {
            provider,
            model: String::new(),
            base_url: base_url.map(Into::into),
            fallback_models: Vec::new(),
        };
        assert_eq!(
            ai(AiProvider::Apple, None).destination(),
            Destination::Local
        );
        assert_eq!(
            ai(AiProvider::Openai, None).destination(),
            Destination::Cloud
        );
        assert_eq!(
            ai(AiProvider::Custom, Some("http://localhost:11434/v1")).destination(),
            Destination::Local
        );
        assert_eq!(
            ai(AiProvider::Custom, Some("https://api.groq.com/openai/v1")).destination(),
            Destination::Cloud
        );
    }

    #[test]
    fn over_budget_falls_back_to_this_computer() {
        let keys = secrets(true, true);
        let s = Settings::default();
        let over = |sit: Situation| Situation {
            over_budget: true,
            ..sit
        };

        // Apple's models where there are any.
        let apple = Situation {
            apple_speech: true,
            apple_language_model: true,
            ..Situation::default()
        };
        assert_eq!(
            s.resolve_speech_with(&keys, over(apple)).unwrap().provider,
            SpeechProvider::Apple
        );
        let ai = s.resolve_ai_with(&keys, over(apple)).unwrap().unwrap();
        assert_eq!(
            (ai.provider, ai.model.as_str()),
            (AiProvider::Apple, "system")
        );

        // Then local Whisper and the built-in model.
        let local = Situation {
            local_speech: true,
            local_model: true,
            ..Situation::default()
        };
        assert_eq!(
            s.resolve_speech_with(&keys, over(local)).unwrap().provider,
            SpeechProvider::WhisperLocal
        );
        assert_eq!(
            s.resolve_ai_with(&keys, over(local))
                .unwrap()
                .unwrap()
                .provider,
            AiProvider::Local
        );

        // Nothing on this computer: no AI pass, and speech can't run.
        let none = over(Situation::default());
        assert_eq!(s.resolve_ai_with(&keys, none).unwrap(), None);
        assert_eq!(
            s.resolve_speech_with(&keys, none),
            Err(ResolveError::OverBudget)
        );

        // An explicit cloud choice and its model give way too.
        let mut explicit = Settings::default();
        explicit.ai.provider = AiProvider::Openai;
        explicit.ai.model = Some("gpt-4.1".into());
        assert_eq!(
            explicit
                .resolve_ai_with(&keys, over(local))
                .unwrap()
                .unwrap()
                .model,
            defaults::LOCAL_AI
        );
        // Under the limit, nothing changes.
        assert_eq!(
            s.resolve_ai_with(&keys, local).unwrap().unwrap().provider,
            AiProvider::Openai
        );
    }

    #[test]
    fn history_is_off_unless_turned_on() {
        let mut g = GeneralSettings::default();
        assert!(!g.keep_history());
        g.keep_history = Some(true);
        assert!(g.keep_history());
    }

    #[test]
    fn agents_reach_history_only_with_its_own_switch() {
        assert!(!NotesSettings::default().agents_history);
        // Allowing notes doesn't allow history.
        let notes: NotesSettings =
            toml::from_str("agents_read = true\nagents_write = true\nagents_meetings = true")
                .unwrap();
        assert!(notes.agents_read && !notes.agents_history);
        let notes: NotesSettings = toml::from_str("agents_history = true").unwrap();
        assert!(notes.agents_history && !notes.agents_read);
    }

    #[test]
    fn update_checks_are_on_unless_turned_off() {
        let mut g = GeneralSettings::default();
        assert!(g.check_for_updates());
        g.check_for_updates = Some(false);
        assert!(!g.check_for_updates());
        let parsed: GeneralSettings = toml::from_str("check_for_updates = false").unwrap();
        assert!(!parsed.check_for_updates());
        // Installing by themselves is off unless chosen, and needs checking.
        assert!(!GeneralSettings::default().install_updates());
        let auto: GeneralSettings = toml::from_str("install_updates = true").unwrap();
        assert!(auto.install_updates());
        let off: GeneralSettings =
            toml::from_str("install_updates = true\ncheck_for_updates = false").unwrap();
        assert!(!off.install_updates());
    }

    #[test]
    fn local_urls() {
        for u in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:8080",
            "http://lmstudio.localhost/v1",
            "http://me:secret@localhost:11434/v1",
            "http://localhost?x=1",
        ] {
            assert!(is_local_url(u), "{u}");
        }
        for u in [
            "https://api.groq.com/openai/v1",
            "http://192.168.1.5:11434/v1",
            "http://localhost.evil.com/v1",
            // A name before the @ that only looks like this computer.
            "http://localhost:11434@evil.example/v1",
            "http://localhost@evil.example",
            "http://evil.example\\@localhost/v1",
            "http://evil.example#@localhost",
        ] {
            assert!(!is_local_url(u), "{u}");
        }
    }

    #[test]
    fn an_unreadable_file_is_kept_and_a_save_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("splashdown-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let typo = "[general]\nvocabulary = [\"Splashdown\"\n";
        std::fs::write(&path, typo).unwrap();
        assert!(matches!(Settings::load(&path), Err(LoadError::Parse(_))));
        let kept = Settings::keep_unreadable(&path).unwrap();
        assert_eq!(kept, dir.join("config.toml.bad"));
        // Defaults saved over it: what the user had is still there.
        Settings::default().save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), typo);
        assert!(Settings::load(&path).is_ok());
        assert!(!dir.join("config.toml.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_draft_letter_another_letter_uses_is_refused() {
        let mut s = Settings::default();
        assert_eq!(s.hotkey_problem(), None);
        s.hotkeys.draft = Some('n');
        assert_eq!(
            s.hotkey_problem(),
            Some("The draft letter must differ from the other letters.")
        );
        // The default moves out of the way of a letter chosen for another.
        s.hotkeys.draft = None;
        s.hotkeys.translate = Some('d');
        assert_eq!(s.hotkey_problem(), None);
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut s = Settings::default();
        s.speech.provider = SpeechProvider::WhisperLocal;
        s.ai.model = Some("x/y".into());
        s.hotkeys.dictation = Some(PhysicalKey::RightMeta);
        s.general.vocabulary = vec!["Tauri".into()];
        let dir = std::env::temp_dir().join(format!("splashdown-settings-{}", std::process::id()));
        let path = dir.join("config.toml");
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("dictation = \"right-meta\""), "{text}");
        assert_eq!(Settings::load(&path).unwrap(), s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_saved_preset_list_gets_word_for_word_once() {
        use crate::presets::WORD_FOR_WORD_ID;
        let saved = "[[presets]]\nid = \"default\"\nname = \"Default\"\n\n[[presets]]\nid = \"email\"\nname = \"Email\"\n";
        let s = Settings::from_toml(saved).unwrap();
        let presets = s.presets();
        let ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["default", WORD_FOR_WORD_ID, "email"]);
        assert!(!presets[1].ai);
        // Deleted afterwards: it stays deleted.
        let mut s = s;
        s.presets
            .as_mut()
            .unwrap()
            .retain(|p| p.id != WORD_FOR_WORD_ID);
        let again = Settings::from_toml(&s.to_toml().unwrap()).unwrap();
        assert!(again.presets().iter().all(|p| p.id != WORD_FOR_WORD_ID));
    }

    #[test]
    fn exported_text_reads_back_whole() {
        let mut s = Settings::default();
        s.general.vocabulary = vec!["Priya".into()];
        s.general.noise_reduction = Some(true);
        s.general.history_days = Some(7);
        s.snippets = vec![Snippet {
            trigger: "my address".into(),
            text: "1 Main St".into(),
        }];
        s.presets = Some(builtin_presets());
        // As saved by this version: the later built-ins already given.
        s.presets_given = crate::presets::LATER_PRESETS
            .iter()
            .map(|id| id.to_string())
            .collect();
        let text = s.to_toml().unwrap();
        assert_eq!(Settings::from_toml(&text).unwrap(), s);
        assert!(Settings::from_toml("[general]\nvocabulary = 3").is_err());
        assert!(Settings::from_toml("not toml at all ===").is_err());
    }

    #[test]
    fn a_learned_word_removed_from_the_vocabulary_is_forgotten() {
        let mut g = GeneralSettings {
            vocabulary: vec!["Tauri".into()],
            learned_words: vec!["Priya".into(), "tauri".into()],
            ..GeneralSettings::default()
        };
        g.settle_learned();
        assert_eq!(g.learned_words, vec!["tauri".to_string()]);
        assert_eq!(g.forgotten_words, vec!["Priya".to_string()]);
        g.settle_learned();
        assert_eq!(g.forgotten_words.len(), 1, "not added twice");
    }

    #[test]
    fn retention_and_noise_reduction_default_off() {
        let mut g = GeneralSettings::default();
        assert!(!g.noise_reduction());
        assert_eq!(g.history_days(), None);
        g.history_days = Some(0);
        assert_eq!(g.history_days(), None, "zero means keep");
        g.history_days = Some(30);
        assert_eq!(g.history_days(), Some(30));
    }

    #[test]
    fn other_sounds_are_turned_down_unless_chosen() {
        assert_eq!(GeneralSettings::default().other_sounds(), OtherAudio::Lower);
        assert!(GeneralSettings::default().share_recent());
        let s = Settings::from_toml("[general]\nother_sounds = \"leave\"").unwrap();
        assert_eq!(s.general.other_sounds(), OtherAudio::Leave, "chosen now");
        // The old name: "leave" was the old default, the rest were chosen.
        let s = Settings::from_toml("[general]\nother_audio = \"leave\"").unwrap();
        assert_eq!(s.general.other_sounds(), OtherAudio::Lower);
        let s = Settings::from_toml("[general]\nother_audio = \"mute\"").unwrap();
        assert_eq!(s.general.other_sounds(), OtherAudio::Mute);
        let p = Settings::from_toml("[general]\nother_sounds = \"pause\"").unwrap();
        assert_eq!(p.general.other_sounds(), OtherAudio::Pause);
        // Saved again, only the new name is written.
        let text = s.to_toml().unwrap();
        assert!(text.contains("other_sounds = \"mute\"") && !text.contains("other_audio"));
    }

    #[test]
    fn your_actions_survive_a_save() {
        let mut s = Settings::default();
        s.actions.mine = vec![
            MyAction {
                name: "Issue".into(),
                about: "opens a Linear issue".into(),
                does: MyActionKind::Link {
                    url: "https://linear.app/acme/issue/{text}".into(),
                },
                trusted: false,
            },
            MyAction {
                name: "Deploy staging".into(),
                about: String::new(),
                does: MyActionKind::Program {
                    path: "/usr/local/bin/deploy".into(),
                    args: vec!["--to".into(), "staging".into()],
                },
                trusted: true,
            },
            MyAction {
                name: "Next slide".into(),
                about: String::new(),
                does: MyActionKind::Keys {
                    keys: "right".into(),
                },
                trusted: false,
            },
        ];
        let text = toml::to_string_pretty(&s).unwrap();
        let back = Settings::from_toml(&text).unwrap();
        assert_eq!(back.actions.mine, s.actions.mine);
        assert!(s.actions.mine[0].takes_text() && !s.actions.mine[1].takes_text());
        assert!(back.actions.enabled_ids().contains(&"my_action"));
        assert!(
            !Settings::default()
                .actions
                .enabled_ids()
                .contains(&"my_action")
        );
    }

    #[test]
    fn parses_toml() {
        let s: Settings = toml::from_str(
            r#"
            [speech]
            provider = "whisper-local"
            model = "small.en"
            [ai]
            provider = "off"
            [general]
            vocabulary = ["Tauri"]
            "#,
        )
        .unwrap();
        assert_eq!(s.speech.provider, SpeechProvider::WhisperLocal);
        assert_eq!(s.ai.provider, AiProvider::Off);
        assert_eq!(s.general.vocabulary, vec!["Tauri"]);
    }
}
