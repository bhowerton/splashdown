use serde::{Deserialize, Serialize};

/// What a recording session does with the user's speech (spec §2.1, §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Transcribe, run the cleanup pass, insert.
    Dictate,
    /// Speech before the instruction-key tap is content, speech after it is
    /// an instruction applied to that content.
    DictateInstruct,
    /// Text was selected at key-down: the whole utterance is an instruction
    /// applied to the selection.
    Transform,
    /// Transcribe and insert with no AI pass.
    Raw,
}

impl Mode {
    /// Whether this mode sends text to the language model.
    pub fn uses_llm(self) -> bool {
        !matches!(self, Mode::Raw)
    }
}
