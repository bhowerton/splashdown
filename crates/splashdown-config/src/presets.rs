//! Presets: per-app formatting, chosen automatically from the focused app,
//! window title or browser URL.

use serde::{Deserialize, Serialize};

use crate::AiProvider;
pub use splashdown_core::CleanupLevel;

pub const DEFAULT_PRESET_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    /// Stable identifier (e.g. `email`); `default` is the fallback.
    pub id: String,
    pub name: String,
    /// Extra instructions added to the AI prompt.
    pub instructions: String,
    /// Run the AI pass. Off means the transcript is inserted as heard.
    pub ai: bool,
    pub insert: InsertMode,
    /// The preset applies when any rule matches.
    pub rules: Vec<AppRule>,
    /// Send the text around the cursor to the AI pass for reference
    /// (spelling of names, tone, continuing a sentence). Off by default:
    /// it can include other people's words.
    pub field_context: bool,
    /// Send the clipboard's text to the AI pass for reference (spelling of
    /// names and code), and for instructions like "summarize what I
    /// copied". Off by default.
    pub clipboard_context: bool,
    /// Translate dictation into this language (e.g. "Spanish"); needs the
    /// AI pass. `None` keeps the language spoken.
    pub translate_to: Option<String>,
    /// How the AI pass should sound. `None` is neutral: no tone added.
    pub tone: Option<Tone>,
    /// How much the AI may reword plain dictation (with `ai` on). `None`
    /// is medium.
    pub cleanup: Option<CleanupLevel>,
    /// Turn file names said aloud into `@path` mentions of files in the
    /// project of the focused terminal or editor (for coding agents).
    pub file_tags: bool,
    /// Press Return after inserting (Return during the dictation still
    /// toggles it, as with the global auto-send).
    pub auto_send: bool,
    /// The language spoken in this preset (ISO 639-1). `None` uses
    /// General → language.
    pub language: Option<String>,
    /// A different AI model for this preset. `None` uses the one chosen
    /// under AI Models.
    pub model: Option<PresetModel>,
    /// A global shortcut that starts a hands-free dictation with this
    /// preset, e.g. `Ctrl+Alt+KeyE` (see `tauri-plugin-global-shortcut`).
    pub shortcut: Option<String>,
}

/// A preset's tone, added to the AI's instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tone {
    Formal,
    Casual,
    VeryCasual,
}

impl Tone {
    /// The instruction for this tone. Kept to wording and punctuation, so
    /// the AI still keeps what was said.
    pub fn instruction(self) -> &'static str {
        match self {
            Tone::Formal => {
                "Formal, professional tone: complete sentences, standard capitalization and punctuation, no slang. Don't add content."
            }
            Tone::Casual => {
                "Casual tone: relaxed and friendly, contractions are fine. Keep slang and emoji I say."
            }
            Tone::VeryCasual => {
                "Very casual, like texting a friend: mostly lowercase, light punctuation, no period at the very end. Keep slang and emoji I say."
            }
        }
    }
}

/// A preset's own AI model: a provider (as under AI Models) and,
/// optionally, a model name (`None`: the provider's default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetModel {
    pub provider: AiProvider,
    #[serde(default)]
    pub model: Option<String>,
}

impl Preset {
    /// What this preset adds to the AI's instructions: its tone, then its
    /// own instructions. `None` if neither is set.
    pub fn ai_instructions(&self) -> Option<String> {
        let parts: Vec<&str> = self
            .tone
            .map(Tone::instruction)
            .into_iter()
            .chain(Some(self.instructions.trim()).filter(|i| !i.is_empty()))
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// The instruction that turns plain dictation into a translation, for
    /// a preset with `translate_to` set.
    pub fn standing_instruction(&self) -> Option<String> {
        self.translate_to
            .as_deref()
            .filter(|l| !l.trim().is_empty())
            .map(|language| {
                format!("Translate this into {language}, keeping the meaning and tone.")
            })
    }
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            instructions: String::new(),
            ai: true,
            insert: InsertMode::Paste,
            rules: Vec::new(),
            field_context: false,
            clipboard_context: false,
            translate_to: None,
            tone: None,
            cleanup: None,
            file_tags: false,
            auto_send: false,
            language: None,
            model: None,
            shortcut: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InsertMode {
    #[default]
    Paste,
    Type,
}

/// Every field that's set must match (case-insensitive). A rule with no
/// fields set matches nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppRule {
    /// Bundle ID (macOS), executable name (Windows) or app name. A trailing
    /// `*` matches by prefix, e.g. `com.jetbrains.*`.
    pub app: Option<String>,
    /// Window title contains this.
    pub title: Option<String>,
    /// Browser address contains this, e.g. `mail.google.com`.
    pub url: Option<String>,
}

/// The focused app, as far as presets care.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppContext<'a> {
    pub app_id: &'a str,
    pub app_name: &'a str,
    pub title: Option<&'a str>,
    pub url: Option<&'a str>,
}

impl AppRule {
    pub fn matches(&self, ctx: &AppContext<'_>) -> bool {
        let fields = [&self.app, &self.title, &self.url];
        if fields
            .iter()
            .all(|f| f.as_deref().is_none_or(str::is_empty))
        {
            return false;
        }
        let app_ok = self
            .app
            .as_deref()
            .filter(|a| !a.is_empty())
            .is_none_or(|want| {
                [ctx.app_id, ctx.app_name]
                    .iter()
                    .any(|have| match want.strip_suffix('*') {
                        Some(prefix) => lower(have).starts_with(&lower(prefix)),
                        None => lower(have) == lower(want),
                    })
            });
        let contains = |want: &Option<String>, have: Option<&str>| {
            want.as_deref()
                .filter(|w| !w.is_empty())
                .is_none_or(|w| have.is_some_and(|h| lower(h).contains(&lower(w))))
        };
        app_ok && contains(&self.title, ctx.title) && contains(&self.url, ctx.url)
    }
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Which kind of app a style is kept for, by the built-in presets' app and
/// site lists (email, work chat, texts), whatever the user's own presets.
pub fn style_kind(ctx: &AppContext<'_>) -> splashdown_core::style::Kind {
    use splashdown_core::style::Kind;
    let builtin = builtin_presets();
    let is = |id: &str| {
        builtin
            .iter()
            .any(|p| p.id == id && p.rules.iter().any(|r| r.matches(ctx)))
    };
    if is("email") {
        Kind::Email
    } else if is("chat") {
        Kind::Chat
    } else if is("texting") {
        Kind::Texts
    } else {
        Kind::Other
    }
}

/// The first preset (other than the fallback) with a matching rule, else
/// the `default` preset, else the first one.
pub fn match_preset<'a>(presets: &'a [Preset], ctx: &AppContext<'_>) -> Option<&'a Preset> {
    presets
        .iter()
        .find(|p| p.id != DEFAULT_PRESET_ID && p.rules.iter().any(|r| r.matches(ctx)))
        .or_else(|| presets.iter().find(|p| p.id == DEFAULT_PRESET_ID))
        .or_else(|| presets.first())
}

fn app(id: &str) -> AppRule {
    AppRule {
        app: Some(id.into()),
        ..AppRule::default()
    }
}

fn url(part: &str) -> AppRule {
    AppRule {
        url: Some(part.into()),
        ..AppRule::default()
    }
}

/// The built-in preset that types exactly what was said (no AI pass).
pub const WORD_FOR_WORD_ID: &str = "word-for-word";

/// Built-in presets added after a release, given once to settings that
/// already have their own preset list (`Settings::upgrade`).
pub const LATER_PRESETS: &[&str] = &[WORD_FOR_WORD_ID];

/// The presets a fresh install starts with.
pub fn builtin_presets() -> Vec<Preset> {
    vec![
        Preset {
            id: DEFAULT_PRESET_ID.into(),
            name: "Default".into(),
            ..Preset::default()
        },
        // Second, so it's the 2 key while holding the dictation key: what
        // was said, exactly, with no AI pass. Never picked by app.
        Preset {
            id: WORD_FOR_WORD_ID.into(),
            name: "Word for word".into(),
            ai: false,
            ..Preset::default()
        },
        Preset {
            id: "email".into(),
            name: "Email".into(),
            instructions: "Use short paragraphs. Only add a greeting or sign-off if I say one or ask for it. Keep my wording.".into(),
            rules: vec![
                app("com.apple.mail"),
                app("com.microsoft.Outlook"),
                app("OUTLOOK.EXE"),
                app("olk.exe"),
                app("com.readdle.smartemail-Mac"),
                app("com.superhuman.electron"),
                url("mail.google.com"),
                url("outlook.live.com"),
                url("outlook.office.com"),
            ],
            ..Preset::default()
        },
        // Personal messages and work chat want different styles: texts are
        // short and casual.
        Preset {
            id: "texting".into(),
            name: "Texting".into(),
            instructions: "No greeting or sign-off. Keep it as short as I said it. Keep slang and emoji I say.".into(),
            tone: Some(Tone::Casual),
            rules: vec![
                app("com.apple.MobileSMS"),
                app("net.whatsapp.WhatsApp"),
                app("WhatsApp.exe"),
                app("ru.keepcoder.Telegram"),
                app("Telegram.exe"),
                app("org.whispersystems.signal-desktop"),
                app("Signal.exe"),
                url("web.whatsapp.com"),
                url("messages.google.com"),
                url("messenger.com"),
                url("web.telegram.org"),
            ],
            ..Preset::default()
        },
        Preset {
            id: "chat".into(),
            name: "Work chat".into(),
            instructions: "No greeting or sign-off. Short paragraphs.".into(),
            tone: Some(Tone::Casual),
            rules: vec![
                app("com.tinyspeck.slackmacgap"),
                app("slack.exe"),
                app("com.microsoft.teams2"),
                app("ms-teams.exe"),
                app("com.hnc.Discord"),
                app("discord.exe"),
                url("app.slack.com"),
                url("discord.com"),
                url("teams.microsoft.com"),
                url("chat.google.com"),
            ],
            ..Preset::default()
        },
        Preset {
            id: "code".into(),
            name: "Code & Terminal".into(),
            instructions: String::new(),
            ai: false,
            file_tags: true,
            rules: vec![
                app("com.apple.Terminal"),
                app("com.googlecode.iterm2"),
                app("dev.warp.Warp-Stable"),
                app("com.mitchellh.ghostty"),
                app("net.kovidgoyal.kitty"),
                app("io.alacritty"),
                app("com.github.wez.wezterm"),
                app("WindowsTerminal.exe"),
                app("cmd.exe"),
                app("powershell.exe"),
                app("pwsh.exe"),
                app("com.microsoft.VSCode"),
                app("Code.exe"),
                app("com.todesktop.230313mzl4w4u92"),
                app("Cursor.exe"),
                app("com.jetbrains.*"),
                app("dev.zed.Zed"),
            ],
            ..Preset::default()
        },
        Preset {
            id: "notes".into(),
            name: "Notes".into(),
            instructions: "Use Markdown lists and headings when I describe structure. Keep my wording.".into(),
            rules: vec![
                app("md.obsidian"),
                app("Obsidian.exe"),
                app("notion.id"),
                app("Notion.exe"),
                app("net.shinyfrog.bear"),
                app("com.apple.Notes"),
                url("notion.so"),
            ],
            ..Preset::default()
        },
    ]
}

/// A preset on its own, as a file to share (`[preset]` in TOML).
#[derive(Serialize, Deserialize)]
struct PresetFile {
    preset: Preset,
}

/// `preset` as a file someone else can import. Its id and shortcut stay
/// behind: they're this computer's.
pub fn preset_to_file(preset: &Preset) -> Result<String, String> {
    let preset = Preset {
        id: String::new(),
        shortcut: None,
        ..preset.clone()
    };
    let text = toml::to_string_pretty(&PresetFile { preset }).map_err(|e| e.to_string())?;
    Ok(format!(
        "# A Splashdown preset. To use it: Settings → Presets → New preset → Import a preset file.\n\n{text}"
    ))
}

/// A preset from a shared file, without an id (the caller gives it one).
pub fn preset_from_file(text: &str) -> Result<Preset, String> {
    let file: PresetFile =
        toml::from_str(text).map_err(|e| format!("That isn't a Splashdown preset file: {e}"))?;
    Ok(Preset {
        id: String::new(),
        shortcut: None,
        ..file.preset
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_are_kept_by_kind_of_app() {
        use splashdown_core::style::Kind;
        assert_eq!(style_kind(&ctx("com.apple.mail", None)), Kind::Email);
        assert_eq!(style_kind(&ctx("slack.exe", None)), Kind::Chat);
        assert_eq!(style_kind(&ctx("com.apple.MobileSMS", None)), Kind::Texts);
        assert_eq!(style_kind(&ctx("com.apple.Notes", None)), Kind::Other);
    }

    #[test]
    fn presets_go_to_files_and_back() {
        let mut email = builtin_presets()
            .into_iter()
            .find(|p| p.id != DEFAULT_PRESET_ID)
            .unwrap();
        email.shortcut = Some("Ctrl+Alt+KeyE".into());
        email.translate_to = Some("Spanish".into());
        let file = preset_to_file(&email).unwrap();
        assert!(file.starts_with("# A Splashdown preset."));
        assert!(file.contains("[preset]") && !file.contains("KeyE"));
        let back = preset_from_file(&file).unwrap();
        assert_eq!(back.name, email.name);
        assert_eq!(back.rules, email.rules);
        assert_eq!(back.translate_to.as_deref(), Some("Spanish"));
        assert_eq!((back.id.as_str(), back.shortcut), ("", None));
        assert!(preset_from_file("name = \"x\"").is_err());
        assert!(preset_from_file("[general]\nlanguage = \"en\"").is_err());
    }

    fn ctx<'a>(id: &'a str, url: Option<&'a str>) -> AppContext<'a> {
        AppContext {
            app_id: id,
            app_name: "",
            title: None,
            url,
        }
    }

    #[test]
    fn matches_app_url_and_falls_back() {
        let p = builtin_presets();
        assert_eq!(
            match_preset(&p, &ctx("com.tinyspeck.slackmacgap", None))
                .unwrap()
                .id,
            "chat"
        );
        assert_eq!(
            match_preset(&p, &ctx("slack.exe", None)).unwrap().id,
            "chat",
            "case-insensitive exe"
        );
        assert_eq!(
            match_preset(
                &p,
                &ctx(
                    "com.google.Chrome",
                    Some("https://mail.google.com/mail/u/0/#inbox")
                )
            )
            .unwrap()
            .id,
            "email"
        );
        assert_eq!(
            match_preset(&p, &ctx("com.google.Chrome", Some("https://example.com")))
                .unwrap()
                .id,
            "default"
        );
        assert_eq!(
            match_preset(&p, &ctx("com.jetbrains.intellij", None))
                .unwrap()
                .id,
            "code",
            "prefix wildcard"
        );
        assert!(
            !match_preset(&p, &ctx("com.apple.Terminal", None))
                .unwrap()
                .ai
        );
    }

    #[test]
    fn texts_and_work_chat_are_separate() {
        let p = builtin_presets();
        assert_eq!(
            match_preset(&p, &ctx("com.apple.MobileSMS", None))
                .unwrap()
                .id,
            "texting"
        );
        assert_eq!(
            match_preset(&p, &ctx("net.whatsapp.WhatsApp", None))
                .unwrap()
                .id,
            "texting"
        );
        assert_eq!(
            match_preset(
                &p,
                &ctx("com.google.Chrome", Some("https://messages.google.com/web"))
            )
            .unwrap()
            .id,
            "texting"
        );
        assert_eq!(
            match_preset(&p, &ctx("com.tinyspeck.slackmacgap", None))
                .unwrap()
                .id,
            "chat"
        );
    }

    #[test]
    fn rule_fields_combine_with_and() {
        let rule = AppRule {
            app: Some("com.google.Chrome".into()),
            title: Some("Gmail".into()),
            url: None,
        };
        let mut c = ctx("com.google.Chrome", None);
        assert!(!rule.matches(&c));
        c.title = Some("Inbox - Gmail");
        assert!(rule.matches(&c));
        assert!(
            !AppRule::default().matches(&c),
            "empty rule matches nothing"
        );
    }

    #[test]
    fn tone_and_instructions_combine() {
        let mut p = Preset::default();
        assert_eq!(p.ai_instructions(), None);
        p.instructions = "  No sign-off.  ".into();
        assert_eq!(p.ai_instructions().as_deref(), Some("No sign-off."));
        p.tone = Some(Tone::Formal);
        let both = p.ai_instructions().unwrap();
        assert!(both.starts_with("Formal, professional tone"));
        assert!(both.ends_with("\nNo sign-off."));
        assert_eq!(p.standing_instruction(), None);
        p.translate_to = Some("Spanish".into());
        assert!(p.standing_instruction().unwrap().contains("into Spanish"));
    }

    #[test]
    fn older_presets_load_with_defaults() {
        let p: Preset = toml::from_str("id = \"x\"\nname = \"X\"\ninstructions = \"\"").unwrap();
        assert_eq!(
            (p.tone, p.auto_send, p.language, p.model, p.shortcut),
            (None, false, None, None, None)
        );
        let p: Preset = toml::from_str(
            "id = \"x\"\nname = \"X\"\ntone = \"very-casual\"\nmodel = { provider = \"local\" }",
        )
        .unwrap();
        assert_eq!(p.tone, Some(Tone::VeryCasual));
        assert_eq!(p.model.unwrap().provider, AiProvider::Local);
    }

    #[test]
    fn round_trips_through_toml() {
        #[derive(Serialize, Deserialize)]
        struct W {
            presets: Vec<Preset>,
        }
        let text = toml::to_string(&W {
            presets: builtin_presets(),
        })
        .unwrap();
        let back: W = toml::from_str(&text).unwrap();
        assert_eq!(back.presets, builtin_presets());
    }
}
