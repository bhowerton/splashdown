//! `config.toml`: which providers and models to use.
//!
//! ```toml
//! [speech]
//! provider = "auto"          # auto | openai | openrouter | whisper-local | apple
//! model = "gpt-4o-mini-transcribe"
//!
//! [ai]
//! provider = "auto"          # auto | openai | openrouter | custom | apple | off
//! model = "gpt-4.1-nano"
//! base_url = "http://localhost:11434/v1"   # custom only
//!
//! [hotkeys]
//! dictation = "right-option"   # see splashdown_core::gesture::PhysicalKey
//! instruction = "right-shift"
//!
//! [general]
//! user_name = "Alex"
//! language = "en"
//! vocabulary = ["Tauri", "OpenRouter"]
//! ```

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
    /// Saved text inserted when its trigger phrase is spoken.
    pub snippets: Vec<Snippet>,
    /// Find-and-replace rules applied to the final text.
    pub replacements: Vec<Replacement>,
    pub notes: NotesSettings,
    /// Saved instructions run over a note or selected text.
    /// Unset means the built-in ones; once edited, the full list is saved.
    pub transforms: Option<Vec<crate::Transform>>,
}

/// Notes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotesSettings {
    /// Where notes are kept, one Markdown file each. Unset means
    /// `Documents/Splashdown Notes`.
    pub folder: Option<std::path::PathBuf>,
    /// Where a captured dictation (dictation key + N) is added.
    pub capture_to: CaptureTo,
    /// Daily notes' folder inside the notes folder. Unset means "Daily".
    pub daily_folder: Option<String>,
    /// Daily notes' file name, with YYYY, MM and DD. Unset means
    /// YYYY-MM-DD (Obsidian's default).
    pub daily_format: Option<String>,
    /// Keep a captured dictation's recording next to the note.
    pub keep_audio: bool,
    /// Let coding agents search and read notes (through `splashdown-mcp`).
    pub agents_read: bool,
    /// …and add to them.
    pub agents_write: bool,
    /// Meeting notes' folder inside the notes folder. Unset means
    /// "Meetings".
    pub meeting_folder: Option<String>,
    /// Whether a meeting is summarized when it ends. Unset means always
    /// with an AI on this computer, and asking first with a cloud one.
    pub meeting_summary: Option<MeetingSummary>,
    /// Keep a meeting's recording with its note.
    pub keep_meeting_audio: bool,
    /// Stop a meeting after 10 minutes with nobody speaking. Unset means on.
    pub stop_when_quiet: Option<bool>,
    /// Show a small bar on screen while a meeting records. Unset means on
    /// on Windows, where the tray can't show the time, and off on macOS,
    /// where the menu bar does.
    pub meeting_pill: Option<bool>,
    /// Offer to record a meeting when a call app starts using the
    /// microphone. Unset means on.
    pub offer_on_calls: Option<bool>,
    /// Questions asked with the ask key may use excerpts of matching notes.
    /// Unset means on.
    pub ask_uses_notes: Option<bool>,
    /// The first meeting's reminder about telling people has been shown.
    pub meeting_reminded: bool,
    /// Keep the Notes window above other apps' windows.
    pub keep_on_top: bool,
}

/// Whether to summarize a meeting when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MeetingSummary {
    Always,
    Ask,
    Never,
}

impl NotesSettings {
    pub fn meeting_folder(&self) -> &str {
        self.meeting_folder
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .unwrap_or("Meetings")
    }

    pub fn stop_when_quiet(&self) -> bool {
        self.stop_when_quiet.unwrap_or(true)
    }

    pub fn meeting_pill(&self) -> bool {
        self.meeting_pill.unwrap_or(cfg!(windows))
    }

    pub fn offer_on_calls(&self) -> bool {
        self.offer_on_calls.unwrap_or(true)
    }

    pub fn ask_uses_notes(&self) -> bool {
        self.ask_uses_notes.unwrap_or(true)
    }

    pub fn daily_folder(&self) -> &str {
        self.daily_folder
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .unwrap_or("Daily")
    }

    pub fn daily_format(&self) -> &str {
        self.daily_format
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .unwrap_or("YYYY-MM-DD")
    }
}

/// Where a captured dictation goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureTo {
    /// `Inbox.md` at the top of the notes folder.
    #[default]
    Inbox,
    /// Today's daily note.
    Daily,
    /// The note last opened in Notes.
    Last,
}

/// A find-and-replace rule. Plain rules match whole words, ignoring case;
/// regex rules are used as written (`$1` refers to groups).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Replacement {
    pub find: String,
    pub replace: String,
    pub regex: bool,
}

impl Replacement {
    /// The rule as a pattern: a plain rule matches its text ignoring case,
    /// as whole words where it starts or ends with a letter or digit; a
    /// regex rule is used as written.
    pub fn compile(&self) -> Result<regex::Regex, regex::Error> {
        if self.find.is_empty() {
            return Err(regex::Error::Syntax("nothing to find".into()));
        }
        if self.regex {
            return regex::Regex::new(&self.find);
        }
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        let pattern = format!(
            "{}{}{}",
            if word(self.find.chars().next()) {
                r"\b"
            } else {
                ""
            },
            regex::escape(&self.find),
            if word(self.find.chars().last()) {
                r"\b"
            } else {
                ""
            },
        );
        regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
    }
}

/// Why one of these replacement rules can't be used: `None` if they're
/// all fine. Settings and the CLI refuse to save them otherwise.
pub fn replacement_problem(rules: &[Replacement]) -> Option<String> {
    rules.iter().enumerate().find_map(|(i, r)| {
        r.compile().err().map(|e| {
            format!(
                "Replacement {} (“{}”) isn't a valid pattern: {e}",
                i + 1,
                r.find
            )
        })
    })
}

/// Unset keys use the platform defaults (Right Option / Right Ctrl, and
/// Right Shift for instructions).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySettings {
    pub dictation: Option<PhysicalKey>,
    pub instruction: Option<PhysicalKey>,
    /// The letter that, pressed while holding the dictation key, carries
    /// on hands-free. Unset means H.
    pub hands_free: Option<char>,
    /// The letter that, pressed while holding the dictation key, sends the
    /// dictation into a note instead of the field. Unset means N, or M if
    /// hands-free already uses N (settings from before notes).
    pub note: Option<char>,
    /// The letter that, pressed while holding the dictation key, makes the
    /// dictation a question answered in the Ask panel. Unset means A (K if
    /// hands-free or the note letter is A).
    pub ask: Option<char>,
    /// Double-tap the dictation key for hands-free too. Unset means off:
    /// Claude and others use a double-tap of Option.
    pub double_tap: Option<bool>,
}

impl HotkeySettings {
    pub fn dictation_key(&self) -> PhysicalKey {
        self.dictation
            .unwrap_or_else(PhysicalKey::default_dictation)
    }

    pub fn instruction_key(&self) -> PhysicalKey {
        self.instruction
            .unwrap_or_else(PhysicalKey::default_instruction)
    }

    pub fn hands_free_key(&self) -> PhysicalKey {
        let letter = self
            .hands_free
            .map(|c| c.to_ascii_lowercase())
            .filter(char::is_ascii_lowercase)
            .unwrap_or('h');
        PhysicalKey::Letter(letter)
    }

    pub fn double_tap(&self) -> bool {
        self.double_tap.unwrap_or(false)
    }

    pub fn note_key(&self) -> PhysicalKey {
        let fallback = if self.hands_free_key() == PhysicalKey::Letter('n') {
            'm'
        } else {
            'n'
        };
        let letter = self
            .note
            .map(|c| c.to_ascii_lowercase())
            .filter(char::is_ascii_lowercase)
            .unwrap_or(fallback);
        PhysicalKey::Letter(letter)
    }

    pub fn ask_key(&self) -> PhysicalKey {
        let taken = [self.hands_free_key(), self.note_key()];
        let fallback = ['a', 'k', 'j', 'q']
            .into_iter()
            .find(|&c| !taken.contains(&PhysicalKey::Letter(c)))
            .unwrap_or('a');
        let letter = self
            .ask
            .map(|c| c.to_ascii_lowercase())
            .filter(char::is_ascii_lowercase)
            .unwrap_or(fallback);
        PhysicalKey::Letter(letter)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeechSettings {
    pub provider: SpeechProvider,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: AiProvider,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// OpenRouter only: models to try, in order, if the main one errors
    /// (rate limits, outages). Unset means the default fallback.
    pub fallback_models: Option<Vec<String>>,
    /// When cleanup runs on this computer (Apple, a local server), send
    /// spoken instructions and edits of selected text to a cloud model
    /// instead, if a key is saved. Plain dictation stays local.
    pub cloud_for_instructions: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub user_name: Option<String>,
    /// ISO-639-1 code; unset means auto-detect.
    pub language: Option<String>,
    pub vocabulary: Vec<String>,
    /// Show words in the overlay while speaking (needs a local Whisper
    /// model). Unset means on.
    pub live_preview: Option<bool>,
    /// Type the AI's text into the app as it's written, rather than all at
    /// once when it's done. Unset means on.
    pub stream_typing: Option<bool>,
    /// Keep a local history of dictations. Unset means off: it's a record
    /// of everything said, so it's opt-in.
    pub keep_history: Option<bool>,
    /// Show the overlay next to the text cursor (where the app reports it)
    /// rather than at the bottom of the screen. Unset means on.
    pub overlay_near_cursor: Option<bool>,
    /// With history on, also keep each recording so it can be transcribed
    /// again. Unset means off.
    pub keep_audio: Option<bool>,
    /// The first-run setup guide has been completed.
    pub onboarded: bool,
    /// Suggest vocabulary from words corrected right after a dictation.
    /// Unset means on.
    pub learn_vocabulary: Option<bool>,
    /// Never use a cloud provider: speech stays on this computer and the AI
    /// pass only runs on a local server (or is off).
    pub local_only: bool,
    /// Tell the AI which app the text is going into. Unset means on.
    pub share_app_name: Option<bool>,
    /// Send what was dictated into the same app in the last few minutes
    /// along with the next dictation, for reference. Unset means on.
    pub share_recent: Option<bool>,
    /// Quiet sounds when recording starts and stops. Unset means on.
    pub sounds: Option<bool>,
    /// Sound volume, 0.0–1.0. Unset means the default (quiet).
    pub sound_volume: Option<f32>,
    /// Microphone name. Unset means automatic: the system default, unless
    /// that's a Bluetooth headset and a built-in mic exists.
    pub microphone: Option<String>,
    /// The microphone to use when `microphone` isn't connected, before
    /// falling back to automatic.
    pub microphone_backup: Option<String>,
    /// Limits on what cloud providers may cost.
    pub budget: Budget,
    /// Look for a new release on launch and once a day. Unset means on.
    /// Updates are only ever installed when the user clicks.
    pub check_for_updates: Option<bool>,
    /// Whisper mode: raise quiet speech before transcribing it. Unset
    /// means off.
    pub whisper_mode: Option<bool>,
    /// Keep usage numbers (words, speaking time, apps) for Settings →
    /// Insights. Unset means on; never the text itself.
    pub insights: Option<bool>,
    /// Languages mixed in speech, possibly mid-sentence (ISO codes), used
    /// while `language` is auto-detect.
    pub languages: Vec<String>,
    /// Keep a small button on screen that starts and finishes a hands-free
    /// dictation when clicked. Unset means off.
    pub mini_bar: Option<bool>,
    /// Remove steady background noise (fans, traffic, chatter) from the
    /// recording before speech-to-text. Unset means off.
    pub noise_reduction: Option<bool>,
    /// Delete history entries (and their recordings) older than this many
    /// days. Unset means keep them (up to the last 300).
    pub history_days: Option<u32>,
    /// What happens to other sounds (music, videos) while recording.
    pub other_audio: OtherAudio,
}

/// Other sounds while recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OtherAudio {
    #[default]
    Leave,
    Lower,
    Mute,
}

/// Quiet enough to sit under speech without being missed.
pub const DEFAULT_SOUND_VOLUME: f32 = 0.12;

impl GeneralSettings {
    pub fn live_preview(&self) -> bool {
        self.live_preview.unwrap_or(true)
    }

    pub fn keep_history(&self) -> bool {
        self.keep_history.unwrap_or(false)
    }

    pub fn overlay_near_cursor(&self) -> bool {
        self.overlay_near_cursor.unwrap_or(true)
    }

    pub fn whisper_mode(&self) -> bool {
        self.whisper_mode.unwrap_or(false)
    }

    pub fn mini_bar(&self) -> bool {
        self.mini_bar.unwrap_or(false)
    }

    pub fn noise_reduction(&self) -> bool {
        self.noise_reduction.unwrap_or(false)
    }

    /// How long history entries are kept, in days (`None`: until the
    /// 300-entry limit pushes them out). Zero counts as unset.
    pub fn history_days(&self) -> Option<u32> {
        self.history_days.filter(|d| *d > 0)
    }

    pub fn keep_insights(&self) -> bool {
        self.insights.unwrap_or(true)
    }

    /// Recordings are kept only alongside history.
    pub fn keep_audio(&self) -> bool {
        self.keep_history() && self.keep_audio.unwrap_or(false)
    }

    pub fn learn_vocabulary(&self) -> bool {
        self.learn_vocabulary.unwrap_or(true)
    }

    pub fn share_app_name(&self) -> bool {
        self.share_app_name.unwrap_or(true)
    }

    pub fn share_recent(&self) -> bool {
        self.share_recent.unwrap_or(true)
    }

    pub fn stream_typing(&self) -> bool {
        self.stream_typing.unwrap_or(true)
    }

    pub fn sounds(&self) -> bool {
        self.sounds.unwrap_or(true)
    }

    pub fn check_for_updates(&self) -> bool {
        self.check_for_updates.unwrap_or(true)
    }

    /// The microphones to try, in order, before the automatic choice.
    pub fn microphones(&self) -> Vec<String> {
        self.microphone
            .iter()
            .chain(
                self.microphone_backup
                    .iter()
                    .filter(|_| self.microphone.is_some()),
            )
            .cloned()
            .collect()
    }

    pub fn sound_volume(&self) -> f32 {
        self.sound_volume
            .unwrap_or(DEFAULT_SOUND_VOLUME)
            .clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechProvider {
    #[default]
    Auto,
    Openai,
    Openrouter,
    /// On this computer, with transcribe.cpp: Nemotron, Parakeet or
    /// Whisper. Named for Whisper, its first model, so existing settings
    /// files keep working.
    WhisperLocal,
    Apple,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiProvider {
    #[default]
    Auto,
    Openai,
    Openrouter,
    /// Claude (Anthropic's API). Cleanup only.
    Anthropic,
    Custom,
    Apple,
    /// The built-in llama.cpp model (a GGUF file in the models folder).
    Local,
    Off,
}

impl std::str::FromStr for SpeechProvider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "auto" => Self::Auto,
            "openai" => Self::Openai,
            "openrouter" => Self::Openrouter,
            "whisper-local" | "whisper" | "local" => Self::WhisperLocal,
            "apple" => Self::Apple,
            _ => {
                return Err(format!(
                    "unknown speech provider \"{s}\" (auto, openai, openrouter, whisper-local, apple)"
                ));
            }
        })
    }
}

impl std::str::FromStr for AiProvider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "auto" => Self::Auto,
            "openai" => Self::Openai,
            "openrouter" => Self::Openrouter,
            "anthropic" | "claude" => Self::Anthropic,
            "custom" => Self::Custom,
            "apple" => Self::Apple,
            "local" => Self::Local,
            "off" | "none" => Self::Off,
            _ => {
                return Err(format!(
                    "unknown AI provider \"{s}\" (auto, openai, openrouter, anthropic, custom, apple, local, off)"
                ));
            }
        })
    }
}

/// Default models, chosen from the M1 eval (see `eval/README.md`).
pub mod defaults {
    pub const OPENAI_STT: &str = "gpt-4o-mini-transcribe";
    /// Gemini 2.5 Flash Lite was rate-limited on most requests; Voxtral
    /// followed spoken instructions instead of transcribing them.
    pub const OPENROUTER_STT: &str = "google/gemini-3.1-flash-lite";
    /// Streams as the user speaks, so the text is ready about 0.03 s after
    /// release even on a CPU (Windows has no GPU build), with the best
    /// English of the local models and 32 languages (eval/README.md,
    /// 2026-11-01).
    pub const LOCAL_SPEECH: &str = "nemotron";
    /// 95% on the cleanup eval (via OpenRouter); gpt-4.1-nano scored 84% and
    /// the GPT-5 reasoning models took 3–7 s per cleanup.
    pub const OPENAI_AI: &str = "gpt-4.1-mini";
    pub const OPENROUTER_AI: &str = "google/gemini-2.5-flash-lite";
    /// Tied on the cleanup eval with no rate limiting, but weaker against
    /// injected instructions, so it's the fallback rather than the default.
    pub const OPENROUTER_AI_FALLBACK: &str = "mistralai/mistral-small-3.2-24b-instruct";
    pub const CUSTOM_BASE_URL: &str = "http://localhost:11434/v1";
    /// Anthropic's cheapest current model, with thinking off (see
    /// `eval/README.md` for how it scores).
    pub const ANTHROPIC_AI: &str = "claude-haiku-5-5";
    /// The built-in local model (`AiProvider::Local`).
    pub const LOCAL_AI: &str = "qwen3-4b";
    /// Model name for providers where the OS picks the model (Apple).
    pub const SYSTEM_MODEL: &str = "system";
}

/// Where a provider sends the user's words (the Privacy tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Destination {
    /// Stays on this computer.
    Local,
    /// Sent to a cloud provider.
    Cloud,
    /// Nothing is sent: the step is off or can't run.
    Off,
}

/// A concrete speech-to-text choice after resolving `auto` and defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechChoice {
    pub provider: SpeechProvider,
    pub model: String,
}

impl SpeechChoice {
    pub fn destination(&self) -> Destination {
        match self.provider {
            SpeechProvider::WhisperLocal | SpeechProvider::Apple => Destination::Local,
            _ => Destination::Cloud,
        }
    }
}

/// A concrete text-model choice; `None` means the AI pass is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiChoice {
    pub provider: AiProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub fallback_models: Vec<String>,
}

impl AiChoice {
    pub fn destination(&self) -> Destination {
        match self.provider {
            AiProvider::Apple | AiProvider::Local => Destination::Local,
            AiProvider::Custom if self.base_url.as_deref().is_some_and(is_local_url) => {
                Destination::Local
            }
            _ => Destination::Cloud,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("{0} needs an API key (set {1})")]
    MissingKey(&'static str, &'static str),
    #[error("the custom AI provider needs a model name")]
    CustomNeedsModel,
    #[error("local-only mode is on, so {0} can't be used")]
    LocalOnly(&'static str),
    #[error(
        "the spending limit is reached and there's no speech model on this computer — raise the limit in Settings, or download a Whisper model"
    )]
    OverBudget,
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
        } else {
            None
        }
    }

    /// Loads settings from `path`; a missing file means all defaults.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
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
        Ok(toml::from_str(text)?)
    }

    /// Writes settings to `path`, creating its folder if needed.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Resolves `auto` providers and default models given which keys exist
    /// (assuming no on-device Apple models).
    pub fn resolve_speech(&self, secrets: &Secrets) -> Result<SpeechChoice, ResolveError> {
        self.resolve_speech_with(secrets, Situation::default())
    }

    /// Like `resolve_speech`, also considering Apple's on-device models.
    /// A saved key wins (the user chose that provider); otherwise Apple is
    /// preferred over local Whisper.
    /// Over the spending limit, cloud speech-to-text gives way to Apple's
    /// model or local Whisper.
    pub fn resolve_speech_with(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<SpeechChoice, ResolveError> {
        let choice = self.choose_speech(secrets, on_device)?;
        if !(on_device.over_budget && choice.destination() == Destination::Cloud) {
            return Ok(choice);
        }
        let provider = if on_device.apple_speech {
            SpeechProvider::Apple
        } else if on_device.local_speech {
            SpeechProvider::WhisperLocal
        } else {
            return Err(ResolveError::OverBudget);
        };
        let mut local = self.clone();
        local.speech = SpeechSettings {
            provider,
            model: None,
        };
        local.choose_speech(secrets, on_device)
    }

    fn choose_speech(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<SpeechChoice, ResolveError> {
        let local_only = self.general.local_only;
        let provider = match self.speech.provider {
            SpeechProvider::Auto if local_only && on_device.apple_speech => SpeechProvider::Apple,
            SpeechProvider::Auto if local_only => SpeechProvider::WhisperLocal,
            SpeechProvider::Openai | SpeechProvider::Openrouter if local_only => {
                return Err(ResolveError::LocalOnly("cloud speech-to-text"));
            }
            SpeechProvider::Auto if secrets.openai.is_some() => SpeechProvider::Openai,
            SpeechProvider::Auto if secrets.openrouter.is_some() => SpeechProvider::Openrouter,
            SpeechProvider::Auto if on_device.apple_speech => SpeechProvider::Apple,
            SpeechProvider::Auto => SpeechProvider::WhisperLocal,
            p => p,
        };
        require_key(provider_key_need_speech(provider), secrets)?;
        let model = self.speech.model.clone().unwrap_or_else(|| {
            match provider {
                SpeechProvider::Openai => defaults::OPENAI_STT,
                SpeechProvider::Openrouter => defaults::OPENROUTER_STT,
                SpeechProvider::WhisperLocal => defaults::LOCAL_SPEECH,
                SpeechProvider::Apple | SpeechProvider::Auto => defaults::SYSTEM_MODEL,
            }
            .to_string()
        });
        Ok(SpeechChoice { provider, model })
    }

    pub fn resolve_ai(&self, secrets: &Secrets) -> Result<Option<AiChoice>, ResolveError> {
        self.resolve_ai_with(secrets, Situation::default())
    }

    /// Like `resolve_ai`, also considering on-device models. Over the
    /// spending limit, a cloud provider gives way to Apple's model, the
    /// built-in local model, or no AI pass.
    pub fn resolve_ai_with(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<Option<AiChoice>, ResolveError> {
        let choice = self.choose_ai(secrets, on_device)?;
        let paid = choice
            .as_ref()
            .is_some_and(|c| c.destination() == Destination::Cloud);
        if !(on_device.over_budget && paid) {
            return Ok(choice);
        }
        let provider = if on_device.apple_language_model {
            AiProvider::Apple
        } else if on_device.local_model {
            AiProvider::Local
        } else {
            return Ok(None);
        };
        let mut local = self.clone();
        local.ai = AiSettings {
            provider,
            ..AiSettings::default()
        };
        local.choose_ai(secrets, on_device)
    }

    fn choose_ai(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Result<Option<AiChoice>, ResolveError> {
        let local_only = self.general.local_only;
        let provider = match self.ai.provider {
            AiProvider::Auto if local_only && on_device.apple_language_model => AiProvider::Apple,
            AiProvider::Auto if local_only && on_device.local_model => AiProvider::Local,
            AiProvider::Auto if local_only => return Ok(None),
            AiProvider::Openai | AiProvider::Openrouter | AiProvider::Anthropic if local_only => {
                return Err(ResolveError::LocalOnly("a cloud AI provider"));
            }
            AiProvider::Custom
                if local_only
                    && !is_local_url(
                        self.ai
                            .base_url
                            .as_deref()
                            .unwrap_or(defaults::CUSTOM_BASE_URL),
                    ) =>
            {
                return Err(ResolveError::LocalOnly("a server on another computer"));
            }
            AiProvider::Auto if secrets.openai.is_some() => AiProvider::Openai,
            AiProvider::Auto if secrets.openrouter.is_some() => AiProvider::Openrouter,
            AiProvider::Auto if secrets.anthropic.is_some() => AiProvider::Anthropic,
            AiProvider::Auto if on_device.apple_language_model => AiProvider::Apple,
            AiProvider::Auto if on_device.local_model => AiProvider::Local,
            AiProvider::Auto | AiProvider::Off => return Ok(None),
            p => p,
        };
        require_key(provider_key_need_ai(provider), secrets)?;
        let model = match (provider, &self.ai.model) {
            (_, Some(m)) => m.clone(),
            (AiProvider::Openai, None) => defaults::OPENAI_AI.into(),
            (AiProvider::Openrouter, None) => defaults::OPENROUTER_AI.into(),
            (AiProvider::Anthropic, None) => defaults::ANTHROPIC_AI.into(),
            (AiProvider::Local, None) => defaults::LOCAL_AI.into(),
            (AiProvider::Custom, None) => return Err(ResolveError::CustomNeedsModel),
            (_, None) => defaults::SYSTEM_MODEL.into(),
        };
        let base_url = (provider == AiProvider::Custom).then(|| {
            self.ai
                .base_url
                .clone()
                .unwrap_or_else(|| defaults::CUSTOM_BASE_URL.into())
        });
        let fallback_models = match (&self.ai.fallback_models, provider) {
            (Some(list), AiProvider::Openrouter) => list.clone(),
            (None, AiProvider::Openrouter) => vec![defaults::OPENROUTER_AI_FALLBACK.into()],
            _ => Vec::new(),
        };
        Ok(Some(AiChoice {
            provider,
            fallback_models: fallback_models
                .into_iter()
                .filter(|m| *m != model)
                .collect(),
            model,
            base_url,
        }))
    }

    /// The cloud model that handles instructions when
    /// `ai.cloud_for_instructions` is on and cleanup itself runs locally.
    /// `None` when it doesn't apply: the setting is off, local-only mode is
    /// on, cleanup already uses the cloud, or there's no key.
    pub fn resolve_instruction_ai_with(
        &self,
        secrets: &Secrets,
        on_device: Situation,
    ) -> Option<AiChoice> {
        if !self.ai.cloud_for_instructions || self.general.local_only || on_device.over_budget {
            return None;
        }
        let main = self.resolve_ai_with(secrets, on_device).ok()??;
        if main.destination() != Destination::Local {
            return None;
        }
        let provider = if secrets.openai.is_some() {
            AiProvider::Openai
        } else if secrets.openrouter.is_some() {
            AiProvider::Openrouter
        } else if secrets.anthropic.is_some() {
            AiProvider::Anthropic
        } else {
            return None;
        };
        let mut cloud = self.clone();
        cloud.ai = AiSettings {
            provider,
            ..AiSettings::default()
        };
        cloud.resolve_ai_with(secrets, on_device).ok().flatten()
    }
}

/// What's true right now that provider resolution depends on, beyond the
/// settings and keys: which on-device models can run, and whether the
/// spending limit is reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Situation {
    /// Apple's speech model is installed (Apple Silicon, macOS 26+).
    pub apple_speech: bool,
    /// Apple Intelligence is on.
    pub apple_language_model: bool,
    /// The built-in local text model is downloaded and can run.
    pub local_model: bool,
    /// The default local Whisper model is downloaded.
    pub local_speech: bool,
    /// The spending limit is reached: cloud providers aren't used.
    pub over_budget: bool,
}

/// Spending limits for cloud providers, in US dollars. Unset means none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub monthly_usd: Option<f64>,
    pub daily_usd: Option<f64>,
}

/// Whether an OpenAI-compatible base URL points at this computer.
pub fn is_local_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        // IPv6 literal, e.g. [::1]:8080
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    ) || host.ends_with(".localhost")
}

enum KeyNeed {
    None,
    OpenAi,
    OpenRouter,
    Anthropic,
}

fn provider_key_need_speech(p: SpeechProvider) -> KeyNeed {
    match p {
        SpeechProvider::Openai => KeyNeed::OpenAi,
        SpeechProvider::Openrouter => KeyNeed::OpenRouter,
        _ => KeyNeed::None,
    }
}

fn provider_key_need_ai(p: AiProvider) -> KeyNeed {
    match p {
        AiProvider::Openai => KeyNeed::OpenAi,
        AiProvider::Openrouter => KeyNeed::OpenRouter,
        AiProvider::Anthropic => KeyNeed::Anthropic,
        _ => KeyNeed::None,
    }
}

fn require_key(need: KeyNeed, secrets: &Secrets) -> Result<(), ResolveError> {
    match need {
        KeyNeed::OpenAi if secrets.openai.is_none() => {
            Err(ResolveError::MissingKey("OpenAI", "OPENAI_API_KEY"))
        }
        KeyNeed::OpenRouter if secrets.openrouter.is_none() => {
            Err(ResolveError::MissingKey("OpenRouter", "OPENROUTER_API_KEY"))
        }
        KeyNeed::Anthropic if secrets.anthropic.is_none() => {
            Err(ResolveError::MissingKey("Anthropic", "ANTHROPIC_API_KEY"))
        }
        _ => Ok(()),
    }
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
    fn auto_uses_the_local_model_without_keys_or_apple() {
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
        assert_eq!(pick(&secrets(false, false), both), Some(AiProvider::Apple));
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
    fn update_checks_are_on_unless_turned_off() {
        let mut g = GeneralSettings::default();
        assert!(g.check_for_updates());
        g.check_for_updates = Some(false);
        assert!(!g.check_for_updates());
        let parsed: GeneralSettings = toml::from_str("check_for_updates = false").unwrap();
        assert!(!parsed.check_for_updates());
    }

    #[test]
    fn local_urls() {
        for u in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:8080",
            "http://lmstudio.localhost/v1",
        ] {
            assert!(is_local_url(u), "{u}");
        }
        for u in [
            "https://api.groq.com/openai/v1",
            "http://192.168.1.5:11434/v1",
            "http://localhost.evil.com/v1",
        ] {
            assert!(!is_local_url(u), "{u}");
        }
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
        let text = s.to_toml().unwrap();
        assert_eq!(Settings::from_toml(&text).unwrap(), s);
        assert!(Settings::from_toml("[general]\nvocabulary = 3").is_err());
        assert!(Settings::from_toml("not toml at all ===").is_err());
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
    fn other_audio_is_left_alone_unless_chosen() {
        assert_eq!(GeneralSettings::default().other_audio, OtherAudio::Leave);
        assert!(GeneralSettings::default().share_recent());
        let s: Settings = toml::from_str("[general]\nother_audio = \"mute\"").unwrap();
        assert_eq!(s.general.other_audio, OtherAudio::Mute);
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
