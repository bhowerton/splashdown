//! Computer use (an exploration): reading the actions a model asks for
//! through Anthropic's computer toolset (`computer_toolset_20260801`),
//! checking them, and building the results that go back. Platform-free;
//! `splashdown-cli computer` drives the screen with `splashdown-platform`.
//!
//! Coordinates are in the pixel space of the screenshots sent to the model;
//! the caller maps them to the screen.

use serde_json::{Value, json};

/// The toolset's name on every call and result.
pub const TOOLSET: &str = "computer";

/// Our own tool next to the toolset: Splashdown types the user's content
/// itself, exactly, where the cursor is, so the model never retypes (and
/// can't change) it.
pub const INSERT_CONTENT: &str = "insert_content";

/// Modifier keys held for a click, scroll or key press.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command on a Mac, the Windows key elsewhere ("super", "cmd").
    pub cmd: bool,
}

impl Mods {
    pub fn any(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.cmd
    }
}

/// A key, by what it is rather than where it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// A named key, normalized: "return", "tab", "escape", "backspace",
    /// "delete", "space", "left", "right", "up", "down", "home", "end",
    /// "pageup", "pagedown", "f1"…"f12".
    Named(&'static str),
    /// A printable character, lower case for letters.
    Char(char),
}

/// A key with its modifiers ("ctrl+s", "cmd+shift+t", "Return").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    pub mods: Mods,
    pub key: Key,
}

const NAMED: &[(&str, &str)] = &[
    ("return", "return"),
    ("enter", "return"),
    ("kp_enter", "return"),
    ("tab", "tab"),
    ("escape", "escape"),
    ("esc", "escape"),
    ("backspace", "backspace"),
    ("delete", "delete"),
    ("del", "delete"),
    ("space", "space"),
    ("left", "left"),
    ("right", "right"),
    ("up", "up"),
    ("down", "down"),
    ("home", "home"),
    ("end", "end"),
    ("page_up", "pageup"),
    ("pageup", "pageup"),
    ("prior", "pageup"),
    ("page_down", "pagedown"),
    ("pagedown", "pagedown"),
    ("next", "pagedown"),
    ("f1", "f1"),
    ("f2", "f2"),
    ("f3", "f3"),
    ("f4", "f4"),
    ("f5", "f5"),
    ("f6", "f6"),
    ("f7", "f7"),
    ("f8", "f8"),
    ("f9", "f9"),
    ("f10", "f10"),
    ("f11", "f11"),
    ("f12", "f12"),
];

/// Characters xdotool names spell out ("minus", "slash").
const CHAR_NAMES: &[(&str, char)] = &[
    ("minus", '-'),
    ("plus", '+'),
    ("equal", '='),
    ("comma", ','),
    ("period", '.'),
    ("slash", '/'),
    ("backslash", '\\'),
    ("semicolon", ';'),
    ("apostrophe", '\''),
    ("grave", '`'),
    ("bracketleft", '['),
    ("bracketright", ']'),
];

/// Reads modifier names ("ctrl", "shift", "super", "cmd"), `+`-joined.
pub fn parse_mods(text: &str) -> Result<Mods, String> {
    let mut mods = Mods::default();
    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        if !set_mod(&mut mods, part) {
            return Err(format!("{part:?} isn't a modifier"));
        }
    }
    Ok(mods)
}

fn set_mod(mods: &mut Mods, name: &str) -> bool {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => mods.ctrl = true,
        "alt" | "option" | "opt" => mods.alt = true,
        "shift" => mods.shift = true,
        "super" | "cmd" | "command" | "meta" | "win" => mods.cmd = true,
        _ => return false,
    }
    true
}

/// Reads a key or `+`-joined combo, xdotool-style ("Return", "ctrl+s",
/// "alt+Tab", "cmd+shift+t").
pub fn parse_combo(text: &str) -> Result<Combo, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    // "ctrl++" means Ctrl and the plus key.
    let (mod_parts, key_part) = match parts.as_slice() {
        [rest @ .., "", ""] => (rest, "+"),
        [rest @ .., last] => (rest, *last),
        [] => return Err("no key".into()),
    };
    let mut mods = Mods::default();
    for part in mod_parts {
        if !set_mod(&mut mods, part) {
            return Err(format!("{part:?} isn't a modifier"));
        }
    }
    let lower = key_part.to_ascii_lowercase();
    let key = if let Some((_, name)) = NAMED.iter().find(|(n, _)| *n == lower) {
        Key::Named(name)
    } else if let Some((_, c)) = CHAR_NAMES.iter().find(|(n, _)| *n == lower) {
        Key::Char(*c)
    } else {
        let mut chars = key_part.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Key::Char(c.to_ascii_lowercase()),
            _ => return Err(format!("{key_part:?} isn't a key")),
        }
    };
    Ok(Combo { mods, key })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// One action the model asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Screenshot,
    /// A region `[x0, y0, x1, y1]` at full resolution.
    Zoom([f64; 4]),
    Click {
        button: Button,
        count: u8,
        /// At the cursor when not given.
        at: Option<(f64, f64)>,
        mods: Mods,
    },
    Drag {
        from: (f64, f64),
        to: (f64, f64),
        mods: Mods,
    },
    MouseMove((f64, f64)),
    MouseDown,
    MouseUp,
    CursorPosition,
    Scroll {
        direction: Direction,
        amount: u32,
        at: Option<(f64, f64)>,
        mods: Mods,
    },
    Type(String),
    Key {
        combo: Combo,
        repeat: u32,
    },
    HoldKey {
        combo: Combo,
        seconds: f64,
    },
    Wait(f64),
    /// Our own tool: type the user's content at the cursor.
    InsertContent,
}

impl Action {
    /// Whether it changes anything (a click, a key, text); looking and
    /// waiting don't.
    pub fn acts(&self) -> bool {
        !matches!(
            self,
            Action::Screenshot | Action::Zoom(_) | Action::CursorPosition | Action::Wait(_)
        )
    }
}

fn point(v: Option<&Value>) -> Result<Option<(f64, f64)>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    match v.as_array().map(Vec::as_slice) {
        Some([x, y]) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => Ok(Some((x, y))),
            _ => Err(format!("not a point: {v}")),
        },
        _ => Err(format!("not a point: {v}")),
    }
}

fn need_point(input: &Value, field: &str) -> Result<(f64, f64), String> {
    point(input.get(field))?.ok_or_else(|| format!("{field} is missing"))
}

fn text<'a>(input: &'a Value, field: &str) -> Option<&'a str> {
    input.get(field).and_then(Value::as_str)
}

fn seconds(input: &Value) -> Result<f64, String> {
    let s = input
        .get("duration")
        .and_then(Value::as_f64)
        .ok_or("duration is missing")?;
    if (0.0..=300.0).contains(&s) {
        Ok(s)
    } else {
        Err(format!("duration {s} is out of range"))
    }
}

/// Reads a `tool_use` block's `name` and `input` into an action.
pub fn parse(name: &str, input: &Value) -> Result<Action, String> {
    let mods = || text(input, "text").map_or(Ok(Mods::default()), parse_mods);
    let click = |button, count| -> Result<Action, String> {
        Ok(Action::Click {
            button,
            count,
            at: point(input.get("coordinate"))?,
            mods: mods()?,
        })
    };
    match name {
        "screenshot" => Ok(Action::Screenshot),
        "zoom" => {
            let region: Vec<f64> = input
                .get("region")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            match region.as_slice() {
                [x0, y0, x1, y1] if x1 > x0 && y1 > y0 && *x0 >= 0.0 && *y0 >= 0.0 => {
                    Ok(Action::Zoom([*x0, *y0, *x1, *y1]))
                }
                _ => Err("region must be [x0, y0, x1, y1] with x1 > x0 and y1 > y0".into()),
            }
        }
        "left_click" => click(Button::Left, 1),
        "right_click" => click(Button::Right, 1),
        "middle_click" => click(Button::Middle, 1),
        "double_click" => click(Button::Left, 2),
        "triple_click" => click(Button::Left, 3),
        "left_click_drag" => Ok(Action::Drag {
            from: need_point(input, "start_coordinate")?,
            to: need_point(input, "coordinate")?,
            mods: mods()?,
        }),
        "mouse_move" => Ok(Action::MouseMove(need_point(input, "coordinate")?)),
        "left_mouse_down" => Ok(Action::MouseDown),
        "left_mouse_up" => Ok(Action::MouseUp),
        "cursor_position" => Ok(Action::CursorPosition),
        "scroll" => {
            let direction = match text(input, "scroll_direction") {
                Some("up") => Direction::Up,
                Some("down") => Direction::Down,
                Some("left") => Direction::Left,
                Some("right") => Direction::Right,
                other => return Err(format!("scroll_direction {other:?}")),
            };
            let amount = input
                .get("scroll_amount")
                .and_then(Value::as_u64)
                .unwrap_or(3)
                .clamp(1, 50) as u32;
            Ok(Action::Scroll {
                direction,
                amount,
                at: point(input.get("coordinate"))?,
                mods: mods()?,
            })
        }
        "type" => text(input, "text")
            .map(|t| Action::Type(t.to_string()))
            .ok_or_else(|| "text is missing".into()),
        "key" => Ok(Action::Key {
            combo: parse_combo(text(input, "text").ok_or("text is missing")?)?,
            repeat: input
                .get("repeat")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 100) as u32,
        }),
        "hold_key" => Ok(Action::HoldKey {
            combo: parse_combo(text(input, "text").ok_or("text is missing")?)?,
            seconds: seconds(input)?,
        }),
        "wait" => Ok(Action::Wait(seconds(input)?)),
        INSERT_CONTENT => Ok(Action::InsertContent),
        other => Err(format!("{other:?} isn't an action")),
    }
}

/// Words on a button or menu item that mean the click can't be taken back
/// or reaches other people: such clicks wait for the user's OK, whatever
/// the model decided. Lower case; matched as whole words.
const CONSEQUENTIAL: &[&str] = &[
    "send",
    "send now",
    "delete",
    "remove",
    "erase",
    "empty trash",
    "buy",
    "buy now",
    "pay",
    "purchase",
    "place order",
    "checkout",
    "check out",
    "submit",
    "publish",
    "post",
    "share",
    "confirm",
    "transfer",
    "sign",
    "accept",
    "agree",
    "subscribe",
    "unsubscribe",
    "install",
    "uninstall",
    "reply all",
    "forward",
    "archive",
    "move to trash",
    "discard",
    "overwrite",
    "replace",
    "enviar",
    "eliminar",
    "borrar",
    "comprar",
    "pagar",
    "senden",
    "löschen",
    "kaufen",
    "bezahlen",
    "envoyer",
    "supprimer",
    "acheter",
    "payer",
];

/// The consequential word in a control's label, if there is one.
pub fn consequential(label: &str) -> Option<&'static str> {
    let words: Vec<String> = label
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    CONSEQUENTIAL.iter().copied().find(|phrase| {
        let phrase: Vec<&str> = phrase.split(' ').collect();
        words
            .windows(phrase.len())
            .any(|w| w.iter().zip(&phrase).all(|(a, b)| a == b))
    })
}

/// Whether a key press commits something by itself (sends a message,
/// deletes a file): Return with Command or Control, Command+Delete.
pub fn consequential_key(combo: &Combo) -> bool {
    match combo.key {
        Key::Named("return") => combo.mods.cmd || combo.mods.ctrl,
        Key::Named("backspace" | "delete") => combo.mods.cmd,
        _ => false,
    }
}

/// A `tool_result` with text.
pub fn result_text(id: &str, text: &str, error: bool) -> Value {
    let mut r = json!({
        "type": "tool_result",
        "tool_use_id": id,
        "toolset_name": TOOLSET,
        "content": [{ "type": "text", "text": text }],
    });
    if error {
        r["is_error"] = json!(true);
    }
    r
}

/// A `tool_result` with an image (`mime`, base64), and optional text
/// before it.
pub fn result_image(id: &str, mime: &str, base64: &str, note: Option<&str>) -> Value {
    let mut content = Vec::new();
    if let Some(note) = note {
        content.push(json!({ "type": "text", "text": note }));
    }
    content.push(json!({
        "type": "image",
        "source": { "type": "base64", "media_type": mime, "data": base64 },
    }));
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "toolset_name": TOOLSET,
        "content": content,
    })
}

/// The result for our own `insert_content` tool (not part of the toolset,
/// so no `toolset_name`).
pub fn result_own(id: &str, text: &str, error: bool) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": text,
        "is_error": error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_key_combos() {
        let c = parse_combo("ctrl+s").unwrap();
        assert!(c.mods.ctrl && !c.mods.cmd);
        assert_eq!(c.key, Key::Char('s'));
        assert_eq!(parse_combo("Return").unwrap().key, Key::Named("return"));
        let c = parse_combo("cmd+shift+T").unwrap();
        assert!(c.mods.cmd && c.mods.shift);
        assert_eq!(c.key, Key::Char('t'));
        assert_eq!(parse_combo("super+space").unwrap().key, Key::Named("space"));
        assert_eq!(parse_combo("ctrl++").unwrap().key, Key::Char('+'));
        assert_eq!(
            parse_combo("Page_Down").unwrap().key,
            Key::Named("pagedown")
        );
        assert_eq!(parse_combo("ctrl+minus").unwrap().key, Key::Char('-'));
        assert!(parse_combo("hyper+x").is_err());
        assert!(parse_combo("Launch_Rockets").is_err());
    }

    #[test]
    fn reads_actions() {
        assert_eq!(
            parse("left_click", &json!({"coordinate": [10, 20]})).unwrap(),
            Action::Click {
                button: Button::Left,
                count: 1,
                at: Some((10.0, 20.0)),
                mods: Mods::default()
            }
        );
        assert!(matches!(
            parse("double_click", &json!({"text": "shift"})).unwrap(),
            Action::Click {
                count: 2,
                at: None,
                mods: Mods { shift: true, .. },
                ..
            }
        ));
        assert_eq!(
            parse(
                "left_click_drag",
                &json!({"start_coordinate": [1, 2], "coordinate": [3, 4]})
            )
            .unwrap(),
            Action::Drag {
                from: (1.0, 2.0),
                to: (3.0, 4.0),
                mods: Mods::default()
            }
        );
        assert!(matches!(
            parse(
                "scroll",
                &json!({"scroll_direction": "down", "scroll_amount": 5})
            )
            .unwrap(),
            Action::Scroll {
                direction: Direction::Down,
                amount: 5,
                at: None,
                ..
            }
        ));
        assert_eq!(
            parse("zoom", &json!({"region": [0, 0, 100, 50]})).unwrap(),
            Action::Zoom([0.0, 0.0, 100.0, 50.0])
        );
        assert!(parse("zoom", &json!({"region": [100, 0, 0, 50]})).is_err());
        assert!(parse("left_click", &json!({"coordinate": [-1, 5]})).is_err());
        assert!(parse("wait", &json!({"duration": 500})).is_err());
        assert!(parse("format_disk", &json!({})).is_err());
        assert!(!parse("screenshot", &json!({})).unwrap().acts());
        assert!(parse("type", &json!({"text": "hi"})).unwrap().acts());
    }

    #[test]
    fn spots_consequential_controls() {
        assert_eq!(consequential("Send"), Some("send"));
        assert_eq!(consequential("Place Order"), Some("place order"));
        assert_eq!(consequential("Move to Trash"), Some("move to trash"));
        assert_eq!(consequential("Löschen"), Some("löschen"));
        assert_eq!(consequential("Sender"), None, "a whole word only");
        assert_eq!(consequential("New Message"), None);
        assert!(consequential_key(&parse_combo("cmd+Return").unwrap()));
        assert!(consequential_key(&parse_combo("cmd+BackSpace").unwrap()));
        assert!(!consequential_key(&parse_combo("Return").unwrap()));
    }

    #[test]
    fn results_echo_the_toolset() {
        let r = result_text("toolu_1", "OK", false);
        assert_eq!(r["toolset_name"], "computer");
        assert!(r.get("is_error").is_none());
        let r = result_image("toolu_2", "image/jpeg", "AAAA", Some("after the click"));
        assert_eq!(r["content"][1]["source"]["media_type"], "image/jpeg");
        assert!(
            result_own("toolu_3", "OK", false)
                .get("toolset_name")
                .is_none()
        );
    }
}
