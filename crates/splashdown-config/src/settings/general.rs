//! Settings → General and Privacy: vocabulary and learned words, history,
//! sounds, the overlay, what the AI is told, and Compose's choices.

use super::*;

/// Languages to translate into, at most (`translate_targets`). Settings
/// has its own copy (`app/ui/settings/general.js`).
pub const MOST_TRANSLATE_TARGETS: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub user_name: Option<String>,
    /// ISO-639-1 code; unset means auto-detect.
    pub language: Option<String>,
    pub vocabulary: Vec<String>,
    /// Words in `vocabulary` learned from corrections (shown as learned,
    /// to forget with one click).
    pub learned_words: Vec<String>,
    /// Learned words the user removed: never learned again.
    pub forgotten_words: Vec<String>,
    /// How speech recognition wrote vocabulary words before the user
    /// corrected them ("TRUX" = ["trucks"]), a hint for the AI pass.
    pub misheard: std::collections::BTreeMap<String, Vec<String>>,
    /// Show words in the overlay while speaking (needs a speech model on
    /// this computer). Unset means on.
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
    /// Learn names and terms corrected right after a dictation into the
    /// vocabulary. Unset means on.
    pub learn_vocabulary: Option<bool>,
    /// Never use a cloud provider: speech stays on this computer and the AI
    /// pass only runs on a local server (or is off).
    pub local_only: bool,
    /// Tell the AI which app the text is going into. Unset means on.
    pub share_app_name: Option<bool>,
    /// Send what was dictated into the same app in the last few minutes
    /// along with the next dictation, for reference. Unset means on.
    pub share_recent: Option<bool>,
    /// Compose reads the text around the field in its window (the
    /// conversation being answered) when nothing is selected. Unset means
    /// on.
    pub compose_reads_window: Option<bool>,
    /// Names and terms in the window around the field (the email or chat
    /// on screen) join the vocabulary for each dictation. Unset means on.
    pub screen_words: Option<bool>,
    /// Compose writes the way the user does in their own earlier messages
    /// on screen. Unset means on.
    pub compose_style: Option<bool>,
    /// Keep the drafts Compose writes and what the user sends instead (on
    /// this computer), to learn their style. Unset means on.
    pub learn_style: Option<bool>,
    /// Compose shows its draft in a panel to change and Insert, instead of
    /// writing it straight into the field. Unset means off.
    pub compose_panel: Option<bool>,
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
    pub check_for_updates: Option<bool>,
    /// Install a release that's found without being asked, once it has
    /// been out a couple of days and Splashdown is idle. Unset means off:
    /// then updates are only ever installed when the user clicks.
    pub install_updates: Option<bool>,
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
    /// Unset means turned down, as Siri does.
    pub other_sounds: Option<OtherAudio>,
    /// The same setting's earlier name, read once and never written: it was
    /// always saved, as "leave" when nobody had chosen it, so only "lower"
    /// and "mute" carry over (`Settings::load`).
    #[serde(rename = "other_audio", skip_serializing)]
    pub legacy_other_audio: Option<OtherAudio>,
    /// The voice that reads text aloud (the system's id for it). Unset
    /// means the system's own voice for the text's language.
    pub read_voice: Option<String>,
    /// How fast text is read aloud, 1.0 being the voice's usual pace.
    /// Unset means 1.0.
    pub read_speed: Option<f32>,
    /// The natural voices were offered (and downloaded, or not) the first
    /// time something was read without them.
    pub natural_voice_offered: bool,
    /// Languages to translate into (Right Option + T, ⌃⌥T), by English
    /// name ("Spanish"), as the menu bar lists them. Up to
    /// `MOST_TRANSLATE_TARGETS`.
    pub translate_targets: Vec<String>,
    /// The one translating goes into now (picked in the menu bar). Unset,
    /// or no longer in the list, means the first.
    pub translate_to: Option<String>,
    /// In email and chat apps, write a dictation in the language of the
    /// conversation on screen when it's clearly another one than spoken.
    pub reply_in_their_language: bool,
}

/// Other sounds while recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OtherAudio {
    Leave,
    /// To a quarter of their volume (the default).
    Lower,
    Mute,
    /// Players that are playing are paused, and played again afterwards.
    Pause,
}

/// Quiet enough to sit under speech without being missed.
pub const DEFAULT_SOUND_VOLUME: f32 = 0.12;

impl GeneralSettings {
    /// The language ⌃⌥T translates into, if any is set up.
    pub fn translate_target(&self) -> Option<&str> {
        let targets: Vec<&str> = self
            .translate_targets
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        self.translate_to
            .as_deref()
            .map(str::trim)
            .filter(|t| targets.contains(t))
            .or_else(|| targets.first().copied())
    }

    pub fn live_preview(&self) -> bool {
        self.live_preview.unwrap_or(true)
    }

    pub fn keep_history(&self) -> bool {
        self.keep_history.unwrap_or(false)
    }

    pub fn overlay_near_cursor(&self) -> bool {
        self.overlay_near_cursor.unwrap_or(true)
    }

    /// What happens to other sounds while recording.
    pub fn other_sounds(&self) -> OtherAudio {
        self.other_sounds.unwrap_or(OtherAudio::Lower)
    }

    /// How fast text is read aloud, from half to twice the usual pace.
    pub fn read_speed(&self) -> f32 {
        self.read_speed.unwrap_or(1.0).clamp(0.5, 2.0)
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

    /// Learned words no longer in the vocabulary (removed in Settings)
    /// become forgotten, so they aren't learned again.
    pub fn settle_learned(&mut self) {
        let vocabulary = &self.vocabulary;
        self.misheard
            .retain(|word, _| vocabulary.iter().any(|v| v.eq_ignore_ascii_case(word)));
        let (kept, removed): (Vec<String>, Vec<String>) = std::mem::take(&mut self.learned_words)
            .into_iter()
            .partition(|w| vocabulary.iter().any(|v| v.eq_ignore_ascii_case(w)));
        self.learned_words = kept;
        for word in removed {
            if !self
                .forgotten_words
                .iter()
                .any(|f| f.eq_ignore_ascii_case(&word))
            {
                self.forgotten_words.push(word);
            }
        }
    }

    pub fn learn_vocabulary(&self) -> bool {
        self.learn_vocabulary.unwrap_or(true)
    }

    pub fn share_app_name(&self) -> bool {
        self.share_app_name.unwrap_or(true)
    }

    pub fn compose_reads_window(&self) -> bool {
        self.compose_reads_window.unwrap_or(true)
    }

    pub fn screen_words(&self) -> bool {
        self.screen_words.unwrap_or(true)
    }

    pub fn compose_style(&self) -> bool {
        self.compose_style.unwrap_or(true)
    }

    pub fn learn_style(&self) -> bool {
        self.learn_style.unwrap_or(true)
    }

    pub fn compose_panel(&self) -> bool {
        self.compose_panel.unwrap_or(false)
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

    /// Whether updates install by themselves: only while checking is on.
    pub fn install_updates(&self) -> bool {
        self.check_for_updates() && self.install_updates.unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_translate_target_is_the_one_picked_or_the_first() {
        let mut g = GeneralSettings::default();
        assert_eq!(g.translate_target(), None);
        g.translate_targets = vec!["Spanish".into(), " French ".into()];
        assert_eq!(g.translate_target(), Some("Spanish"));
        g.translate_to = Some("French".into());
        assert_eq!(g.translate_target(), Some("French"));
        // No longer in the list.
        g.translate_to = Some("German".into());
        assert_eq!(g.translate_target(), Some("Spanish"));
    }
}
