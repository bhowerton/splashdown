//! "What can I say?": every kind of command on one page, in a small window.
//! Said as a command ("what can I say?", "help"), chosen from the menu bar,
//! or pointed to when a command isn't understood. Built from the settings
//! each time it's shown, so it names the user's own keys and lists only
//! the voice actions that are on and that this computer can do.

use serde::Serialize;
use splashdown_config::Settings;
use splashdown_core::gesture::PhysicalKey;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;

pub const WINDOW: &str = "help";
/// App event with a fresh `Sheet`, when the window is shown again.
const CHANGED: &str = "help-changed";

/// The page: sections of things to say or press.
#[derive(Debug, Clone, Serialize)]
pub struct Sheet {
    pub sections: Vec<Section>,
    /// Voice actions are off: the "Do things" section offers to turn them on.
    pub actions_off: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub title: String,
    /// How to give these ("Tap Right Shift first, then say it").
    pub how: String,
    pub items: Vec<Item>,
    /// The voice actions section (for the page's "Turn them on" button).
    pub actions: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub what: String,
    /// Things to say, shown in quotes.
    pub say: Vec<String>,
    /// Or a key to press.
    pub keys: Option<String>,
}

fn say(what: &str, say: &[&str]) -> Item {
    Item {
        what: what.into(),
        say: say.iter().map(|s| (*s).to_string()).collect(),
        keys: None,
    }
}

fn press(what: &str, keys: impl Into<String>) -> Item {
    Item {
        what: what.into(),
        say: Vec::new(),
        keys: Some(keys.into()),
    }
}

/// A key's name, with letters as the letter.
fn key(k: PhysicalKey) -> String {
    match k {
        PhysicalKey::Letter(c) => c.to_ascii_uppercase().to_string(),
        k => k.label().to_string(),
    }
}

/// A global shortcut with Control and Option (Alt).
fn shortcut(letter: char) -> String {
    if cfg!(target_os = "macos") {
        format!("⌃⌥{letter}")
    } else {
        format!("Ctrl+Alt+{letter}")
    }
}

/// What the sheet says for `settings`.
pub fn sheet(settings: &Settings) -> Sheet {
    let h = &settings.hotkeys;
    let dictation = key(h.dictation_key());
    let instruction = key(h.instruction_key());
    let mut sections = vec![
        Section {
            title: "Change what you just dictated".into(),
            how: format!(
                "Hold {dictation}, tap {instruction} before saying anything, then say it."
            ),
            items: vec![
                say(
                    "Rewrite it",
                    &[
                        "make it more formal",
                        "translate that to Spanish",
                        "capitalize that",
                    ],
                ),
                say(
                    "Delete",
                    &["delete that last sentence", "delete the last word"],
                ),
                say(
                    "Format",
                    &["make that bold", "italicize that", "select that"],
                ),
                say("Send it or take it back", &["send it", "undo that"]),
            ],
            actions: false,
        },
        Section {
            title: "While you dictate".into(),
            how: format!(
                "Hold {dictation} and speak, then tap {instruction} and add how it should come out."
            ),
            items: vec![say(
                "Add an instruction",
                &["…as a short email", "make it a bulleted list"],
            )],
            actions: false,
        },
        Section {
            title: "Selected text".into(),
            how: format!("Select text, then hold {dictation} and say what to do with it."),
            items: vec![say(
                "Rewrite it",
                &[
                    "make this more formal",
                    "fix the grammar",
                    "translate this to German",
                ],
            )],
            actions: false,
        },
        Section {
            title: "Ask a question".into(),
            how: format!(
                "Hold {dictation}, press {}, and ask. The answer shows in a panel; it can use your notes.",
                key(h.ask_key())
            ),
            items: vec![say(
                "Ask",
                &[
                    "what's the German word for deadline?",
                    "what did we decide about the launch?",
                ],
            )],
            actions: false,
        },
        Section {
            title: "Write it for me".into(),
            how: format!(
                "Hold {dictation}, tap {instruction}, and say what to write. It's written where \
                 you're typing, as a reply to the email or chat on screen. To change it, select \
                 it and say the change."
            ),
            items: vec![
                say(
                    "Draft",
                    &[
                        "reply saying I'm in and ask about parking",
                        "tell the team the launch moved to Thursday",
                    ],
                ),
                say(
                    "Change it",
                    &["shorter", "add that I'm free Thursday", "in German"],
                ),
            ],
            actions: false,
        },
        Section {
            title: "Hear it read aloud".into(),
            how: format!(
                "Select text, hold {dictation} and say it. With nothing selected, tap {instruction} \
                 first and the text you're writing is read. A summary is written by your AI first."
            ),
            items: vec![
                say("Read it", &["read this to me"]),
                say(
                    "Hear a summary",
                    &["read me a summary of this", "read me the action items"],
                ),
                say(
                    "Hear it in another language",
                    &["read this to me in Spanish"],
                ),
                press(
                    "Translate selected text (languages: menu bar → Translate into)",
                    format!("{dictation} + {}", key(h.translate_key())),
                ),
                press("Stop reading", "Esc"),
            ],
            actions: false,
        },
    ];

    let actions_off = !settings.actions.enabled;
    let enabled = settings.actions.enabled_ids();
    let presets = settings.presets();
    let switch_preset = presets
        .get(1)
        .map(|p| format!("switch to the {} preset", p.name));
    let mut doing = Vec::new();
    for id in &enabled {
        let item = match *id {
            "open_app" => say("Open apps", &["open Spotify"]),
            "switch_to_app" => say("Switch apps", &["go back to Slack"]),
            "search_web" => say(
                "Search",
                &[
                    "search YouTube for paella recipes",
                    "find coffee near me on the map",
                ],
            ),
            "open_website" => say("Open websites", &["go to GitHub"]),
            "media" => say(
                "Music and volume",
                &["pause the music", "next song", "set the volume to 30"],
            ),
            "add_todo" => say("To-dos", &["add buy milk to my to-dos", "undo that"]),
            "create_event" => say("Calendar", &["add dentist to my calendar on Thursday at 4"]),
            "create_reminder" => say("Reminders", &["remind me at 5 to stretch"]),
            "draft_email" => say(
                "Email drafts",
                &[
                    "email Dana that I'll be ten minutes late",
                    "email this to Sam (with text selected)",
                ],
            ),
            "run_shortcut" => say("Shortcuts", &["run my Morning routine shortcut"]),
            "my_action" => Item {
                what: "Your actions".into(),
                say: settings
                    .actions
                    .mine
                    .iter()
                    .map(|m| {
                        if m.takes_text() {
                            format!("{} …", m.name)
                        } else {
                            m.name.clone()
                        }
                    })
                    .collect(),
                keys: None,
            },
            "splashdown" => {
                let mut said = vec![
                    "start recording the meeting".to_string(),
                    "new note".to_string(),
                    "whisper mode on".to_string(),
                    "try that again".to_string(),
                ];
                said.extend(switch_preset.clone());
                Item {
                    what: "Splashdown itself".into(),
                    say: said,
                    keys: None,
                }
            }
            _ => continue,
        };
        doing.push(item);
    }
    let how = if actions_off {
        "Voice actions are off. Turned on, the same commands can open apps, search, play \
         music, and add reminders, events and to-dos."
            .to_string()
    } else {
        format!(
            "Hold {dictation} and tap {instruction} first, as for changes, then say what to \
             do. Nothing is ever sent, deleted or bought."
        )
    };
    // Right after the changes: the same key does both.
    sections.insert(
        1,
        Section {
            title: "Do things".into(),
            how,
            items: if actions_off { Vec::new() } else { doing },
            actions: true,
        },
    );

    let mut keys = vec![
        press(
            "Add it to your notes instead of typing it",
            key(h.note_key()),
        ),
        press("Ask a question", key(h.ask_key())),
        press(
            "Dictate a draft to add to and change by voice before it goes in",
            key(h.draft_key()),
        ),
        press(
            &format!("Go hands-free (tap {dictation} again to finish)"),
            key(h.hands_free_key()),
        ),
    ];
    if let Some(compose) = h.compose_key() {
        keys.push(press("Have a message written", key(compose)));
    }
    // The word-for-word preset's number, where it is in the list.
    if let Some(n) = presets
        .iter()
        .position(|p| p.id == splashdown_config::WORD_FOR_WORD_ID)
        .filter(|at| *at < 9)
    {
        keys.push(press(
            "Type exactly what you say, with no tidying",
            (n + 1).to_string(),
        ));
    }
    if presets.len() > 1 {
        keys.push(press("Use another preset", "1–9"));
    }
    keys.push(press(
        "Translate it, or the selected text, into the language in the menu bar (again: the next one)",
        key(h.translate_key()),
    ));
    keys.push(press(
        "Press Return once it's typed",
        key(PhysicalKey::Return),
    ));
    keys.push(press("Cancel", key(PhysicalKey::Escape)));
    sections.push(Section {
        title: format!("Keys while you hold {dictation}"),
        how: String::new(),
        items: keys,
        actions: false,
    });
    sections.push(Section {
        title: "Shortcuts".into(),
        how: String::new(),
        items: vec![
            press(
                "Swap the AI's version for exactly what you said",
                shortcut('Z'),
            ),
            press("Paste exactly what you said last", shortcut('V')),
            press("Open Notes", shortcut('N')),
            press("Record a meeting (again to stop)", shortcut('M')),
            press(
                "Translate without holding a key: the selection, or a dictation (again to finish)",
                shortcut('T'),
            ),
        ],
        actions: false,
    });
    Sheet {
        sections,
        actions_off,
    }
}

fn current(app: &AppHandle) -> Sheet {
    sheet(&app.state::<AppState>().settings())
}

/// Shows the window, creating it the first time.
pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = app.emit_to(WINDOW, CHANGED, current(app));
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("help.html".into()))
        .title("What you can say")
        .inner_size(520.0, 640.0)
        .min_inner_size(380.0, 300.0)
        .resizable(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .center()
        .focused(true)
        .build();
    match result {
        Ok(window) => {
            // Closing only hides it.
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = handle.hide();
                }
            });
        }
        Err(e) => tracing::warn!("couldn't open What you can say: {e}"),
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub fn help_sheet(app: AppHandle) -> Sheet {
    current(&app)
}

/// Esc: hides the window.
#[tauri::command]
pub fn help_close(app: AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.hide();
    }
}

/// "Turn on voice actions": Settings, on the Actions tab.
#[tauri::command]
pub fn help_open_actions(app: AppHandle) {
    help_close(app.clone());
    crate::tray::open_settings_tab(&app, "actions");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_said(sheet: &Sheet) -> Vec<String> {
        sheet
            .sections
            .iter()
            .flat_map(|s| s.items.iter().flat_map(|i| i.say.clone()))
            .collect()
    }

    #[test]
    fn names_the_users_keys() {
        let mut settings = Settings::default();
        settings.hotkeys.instruction = Some(PhysicalKey::LeftShift);
        settings.hotkeys.note = Some('j');
        let sheet = sheet(&settings);
        assert!(sheet.sections[0].how.contains("Left Shift"));
        let keys = sheet
            .sections
            .iter()
            .find(|s| s.title.starts_with("Keys"))
            .unwrap();
        assert_eq!(keys.items[0].keys.as_deref(), Some("J"));
    }

    #[test]
    fn lists_only_actions_that_are_on() {
        let mut settings = Settings::default();
        settings.actions.enabled = false;
        let off = sheet(&settings);
        assert!(off.actions_off);
        assert!(!all_said(&off).iter().any(|s| s == "open Spotify"));

        settings.actions.enabled = true;
        settings.actions.off = vec!["media".into()];
        let on = sheet(&settings);
        let said = all_said(&on);
        assert!(said.iter().any(|s| s == "open Spotify"));
        assert!(!said.iter().any(|s| s == "pause the music"));
        assert_eq!(
            said.iter().any(|s| s.contains("shortcut")),
            cfg!(target_os = "macos")
        );
    }
}
