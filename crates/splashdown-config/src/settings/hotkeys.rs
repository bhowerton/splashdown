//! The dictation and instruction keys and the letters pressed while
//! holding the dictation key, with their defaults.

use super::*;

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
    /// The letter that, pressed while holding the dictation key, makes the
    /// dictation a request for a draft. Unset means none: the instruction
    /// key asks for drafts too ("reply saying I'm in").
    pub compose: Option<char>,
    /// The letter that, pressed while holding the dictation key, translates
    /// the dictation (or the selected text) into the language picked under
    /// Translate into. Unset means T (L if another letter is T).
    pub translate: Option<char>,
    /// The letter that, pressed while holding the dictation key, opens the
    /// dictation in the draft panel instead of typing it, to add to and
    /// change by voice before inserting it. Unset means D (G or B if
    /// another letter is D).
    pub draft: Option<char>,
    /// Double-tap the dictation key for hands-free too. Unset means off:
    /// other apps may use a double-tap of Option for their own shortcuts.
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

    pub fn translate_key(&self) -> PhysicalKey {
        let taken = [
            Some(self.hands_free_key()),
            Some(self.note_key()),
            Some(self.ask_key()),
            self.compose_key(),
        ];
        let fallback = ['t', 'l', 'y']
            .into_iter()
            .find(|&c| !taken.contains(&Some(PhysicalKey::Letter(c))))
            .unwrap_or('t');
        let letter = self
            .translate
            .map(|c| c.to_ascii_lowercase())
            .filter(char::is_ascii_lowercase)
            .unwrap_or(fallback);
        PhysicalKey::Letter(letter)
    }

    /// The draft letter: after the others, so a letter someone already
    /// chose for one of them keeps its meaning.
    pub fn draft_key(&self) -> PhysicalKey {
        let taken = [
            Some(self.hands_free_key()),
            Some(self.note_key()),
            Some(self.ask_key()),
            self.compose_key(),
            Some(self.translate_key()),
        ];
        let fallback = ['d', 'g', 'b']
            .into_iter()
            .find(|&c| !taken.contains(&Some(PhysicalKey::Letter(c))))
            .unwrap_or('d');
        let letter = self
            .draft
            .map(|c| c.to_ascii_lowercase())
            .filter(char::is_ascii_lowercase)
            .unwrap_or(fallback);
        PhysicalKey::Letter(letter)
    }

    /// The compose letter, if one is set.
    pub fn compose_key(&self) -> Option<PhysicalKey> {
        self.compose
            .map(|c| c.to_ascii_lowercase())
            .filter(char::is_ascii_lowercase)
            .map(PhysicalKey::Letter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_draft_letter_is_d_unless_another_letter_is() {
        let mut h = HotkeySettings::default();
        assert_eq!(h.draft_key(), PhysicalKey::Letter('d'));
        h.compose = Some('d');
        assert_eq!(h.draft_key(), PhysicalKey::Letter('g'));
        h.translate = Some('g');
        assert_eq!(h.draft_key(), PhysicalKey::Letter('b'));
        h.draft = Some('X');
        assert_eq!(h.draft_key(), PhysicalKey::Letter('x'));
    }
}
