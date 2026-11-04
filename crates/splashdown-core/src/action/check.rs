//! Checking a planned action's parameters before anything runs: types,
//! times, and names matched to the user's own (`best_name`), with the text
//! folding and spelling distance that takes.

use super::*;

/// Lower case, without accents, for comparing names.
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Whether `text` names `value`: all of it, or one of its words of three
/// or more letters (any word in scripts written without spaces).
pub(super) fn mentions(text: &str, value: &str) -> bool {
    text.contains(value)
        || value
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 3 || !w.is_ascii())
            .any(|w| text.contains(w))
}

/// Which of `names` (installed apps) `query` means: the same name, all of
/// its words ("word" → "Microsoft Word", "vs code" → "Visual Studio
/// Code"), the start of a name, or a name one or two letters off (a
/// misheard "Spotfy"). Ties go to the shorter name.
pub fn best_name(query: &str, names: &[String]) -> Option<usize> {
    let words = |s: &str| -> Vec<String> {
        fold(s.trim_end_matches(".app"))
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let q = words(query);
    if q.is_empty() {
        return None;
    }
    let q_joined = q.join(" ");
    names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let n = words(name);
            let n_joined = n.join(" ");
            let initials: String = n.iter().filter_map(|w| w.chars().next()).collect();
            let word_matches = |qw: &String| {
                n.iter()
                    .any(|nw| nw == qw || (qw.len() >= 3 && nw.starts_with(qw.as_str())))
                    || (qw.len() >= 2 && initials.starts_with(qw.as_str()))
            };
            let score = if n_joined == q_joined {
                100
            } else if q.iter().all(word_matches) {
                80
            } else if q_joined.chars().count() >= 3 && n_joined.starts_with(&q_joined) {
                70
            } else {
                let distance = edit_distance(&n_joined, &q_joined);
                let allowed = if q_joined.chars().count() >= 8 { 2 } else { 1 };
                if q_joined.chars().count() >= 4 && distance <= allowed {
                    60
                } else {
                    return None;
                }
            };
            Some((i, score, n_joined.chars().count()))
        })
        .max_by_key(|&(_, score, len)| (score, std::cmp::Reverse(len)))
        .map(|(i, ..)| i)
}

pub(super) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// A parameter's value, checked against its kind. `None` when it's
/// missing, null or empty.
pub(super) enum Checked {
    Text(String),
    Number(i64),
}

pub(super) fn check_param(param: &Param, value: Option<&Value>) -> Result<Option<Checked>, String> {
    let value = match value {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(v) => Some(v),
    };
    let Some(value) = value else {
        return if param.required {
            Err(format!("{} is missing", param.name))
        } else {
            Ok(None)
        };
    };
    let bad = || Err(format!("{} isn't valid: {value}", param.name));
    match param.kind {
        Kind::Text => match value.as_str() {
            Some(s) => Ok(Some(Checked::Text(s.trim().to_string()))),
            None => bad(),
        },
        Kind::OneOf(words) => match value.as_str().map(str::trim) {
            Some(s) if words.contains(&s) => Ok(Some(Checked::Text(s.to_string()))),
            _ => bad(),
        },
        Kind::Time => match value.as_str().map(str::trim).and_then(local_time) {
            Some(t) => Ok(Some(Checked::Text(t))),
            None => bad(),
        },
        Kind::Number(min, max) => {
            let n = value
                .as_i64()
                .or_else(|| value.as_f64().map(|f| f.round() as i64))
                .or_else(|| {
                    value
                        .as_str()
                        .and_then(|s| s.trim().trim_end_matches('%').parse().ok())
                });
            match n {
                Some(n) if (min..=max).contains(&n) => Ok(Some(Checked::Number(n))),
                _ => bad(),
            }
        }
    }
}

/// `YYYY-MM-DDTHH:MM` (seconds and a zone are dropped), if it's a real
/// date and time.
pub(super) fn local_time(s: &str) -> Option<String> {
    let s = s.replace(' ', "T");
    let date = s.get(..10)?;
    let time = s.get(11..16)?;
    if s.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let num = |t: &str| t.parse::<u32>().ok();
    let (y, m, d) = (
        num(date.get(..4)?)?,
        num(date.get(5..7)?)?,
        num(date.get(8..10)?)?,
    );
    let (h, min) = (num(time.get(..2)?)?, num(time.get(3..5)?)?);
    if date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' || time.as_bytes()[2] != b':' {
        return None;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    ((1..=days).contains(&d) && h < 24 && min < 60 && y >= 2000).then(|| format!("{date}T{time}"))
}

pub(super) fn check(spec: &Spec, obj: &Map<String, Value>) -> Result<Action, String> {
    let mut texts = std::collections::HashMap::new();
    let mut numbers = std::collections::HashMap::new();
    for param in spec.params {
        match check_param(param, obj.get(param.name))? {
            Some(Checked::Text(t)) => {
                texts.insert(param.name, t);
            }
            Some(Checked::Number(n)) => {
                numbers.insert(param.name, n);
            }
            None => {}
        }
    }
    let mut take = |name: &str| texts.remove(name);
    // Required parameters were checked above.
    let need = |v: Option<String>| v.unwrap_or_default();
    let action = match spec.id {
        "open_app" => Action::OpenApp {
            app: need(take("app")),
        },
        "switch_to_app" => Action::SwitchToApp {
            app: need(take("app")),
        },
        "search_web" => Action::SearchWeb {
            query: need(take("query")),
            site: take("site").unwrap_or_else(|| "web".into()),
        },
        "open_website" => Action::OpenWebsite {
            site: need(take("site")),
        },
        "splashdown" => {
            let what = need(take("what"));
            let preset = take("preset");
            if what == "switch_preset" && preset.is_none() {
                return Err("switch_preset needs a preset".into());
            }
            Action::Splashdown {
                preset: (what == "switch_preset").then_some(preset).flatten(),
                what,
            }
        }
        "add_todo" => Action::AddTodo {
            text: need(take("text")),
        },
        "media" => {
            let what = need(take("what"));
            let level = numbers.get("level").map(|&n| n as u8);
            if what == "set_volume" && level.is_none() {
                return Err("set_volume needs a level".into());
            }
            Action::Media {
                level: (what == "set_volume").then_some(level).flatten(),
                what,
            }
        }
        "create_event" => {
            let start = need(take("start"));
            // An end that isn't after the start is a slip: the calendar's
            // default length is used instead.
            let end = take("end").filter(|e| *e > start);
            Action::CreateEvent {
                title: need(take("title")),
                start,
                end,
                location: take("location"),
            }
        }
        "create_reminder" => Action::CreateReminder {
            text: need(take("text")),
            due: take("due"),
        },
        "draft_email" => Action::DraftEmail {
            to: take("to"),
            subject: take("subject"),
            body: need(take("body")),
        },
        "run_shortcut" => Action::RunShortcut {
            name: need(take("name")),
            input: take("input"),
        },
        "my_action" => Action::MyAction {
            name: need(take("name")),
            text: take("text"),
        },
        other => return Err(format!("{other:?} has no handler")),
    };
    Ok(action)
}
