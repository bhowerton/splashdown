//! Settings → Actions: which voice actions are on, the user's own actions,
//! and the search engine.

use super::*;

/// Voice actions: spoken commands that do things beyond the text field
/// ("open Spotify", "remind me at 3…"), planned by
/// `splashdown_core::action`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionSettings {
    /// Spoken commands may do things on this computer. Off until chosen.
    pub enabled: bool,
    /// Built-in actions turned off (catalog ids, `open_app`…).
    pub off: Vec<String>,
    /// Where a web search goes.
    pub search_engine: SearchEngine,
    /// Ask before adding a calendar event or reminder, rather than adding
    /// it with Undo.
    pub confirm_events: bool,
    /// Shortcuts that run without asking first ("Always allow").
    pub trusted_shortcuts: Vec<String>,
    /// Actions the user set up: a link, a script or program, keys to press.
    pub mine: Vec<MyAction>,
}

impl ActionSettings {
    /// The catalog ids that are on.
    pub fn enabled_ids(&self) -> Vec<&'static str> {
        splashdown_core::action::CATALOG
            .iter()
            .map(|s| s.id)
            .filter(|id| !self.off.iter().any(|o| o == id))
            .filter(|id| cfg!(target_os = "macos") || *id != "run_shortcut")
            .filter(|id| *id != "my_action" || !self.mine.is_empty())
            .collect()
    }
}

/// One of the user's own voice actions ("Deploy staging", "Next slide").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyAction {
    /// What the user says, more or less ("Deploy staging").
    pub name: String,
    /// What it does, for the AI that picks it (optional).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    #[serde(flatten)]
    pub does: MyActionKind,
    /// Runs without asking first ("Always allow").
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trusted: bool,
}

/// What one of the user's actions does. `{text}` in a link or an argument
/// is filled with what was said for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MyActionKind {
    /// Opens a link (`https://linear.app/acme/issue/{text}`).
    Link { url: String },
    /// Runs a script or program, with these arguments, never through a
    /// shell.
    Program {
        path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    /// Presses keys in the app in front ("cmd+shift+4", "right").
    Keys { keys: String },
}

/// Where text said for an action goes.
pub const TEXT_SLOT: &str = "{text}";

impl MyAction {
    /// It has a place for text.
    pub fn takes_text(&self) -> bool {
        match &self.does {
            MyActionKind::Link { url } => url.contains(TEXT_SLOT),
            MyActionKind::Program { args, .. } => args.iter().any(|a| a.contains(TEXT_SLOT)),
            MyActionKind::Keys { .. } => false,
        }
    }
}

/// A web search engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchEngine {
    #[default]
    Google,
    Duckduckgo,
    Bing,
}
