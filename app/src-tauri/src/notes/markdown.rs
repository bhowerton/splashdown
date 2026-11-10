//! What a note's Markdown holds, read and written as text: the front
//! matter (the `---` block at the top), tags, sections (`## My notes`),
//! the transcript's start, the title, and a file name from it. Nothing
//! here touches files (that's `store.rs`).

use splashdown_core::meeting::Turn;

/// The longest a file name made from a title gets.
pub(super) const NAME_CHARS: usize = 60;

/// Splits `---` front matter from the body. The front matter is returned
/// with its closing line, so writing `front + body` restores the file.
pub fn split_front_matter(text: &str) -> (&str, &str) {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return ("", text);
    };
    let mut offset = text.len() - rest.len();
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() == "---" {
            return text.split_at(offset);
        }
    }
    ("", text)
}

/// A note's tags: its front matter's, then `#tags` in its text, without
/// the `#`, each once (whatever its case), in order.
pub fn tags_of(front: &str, body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in front_tags(front).into_iter().chain(text_tags(body)) {
        if !out.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            out.push(tag);
        }
    }
    out
}

/// `tags:` (or `tag:`) in front matter: `[a, b]`, `a, b` or a list of
/// `- a` lines.
pub fn front_tags(front: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = front.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(value) = line
            .strip_prefix("tags:")
            .or_else(|| line.strip_prefix("tag:"))
        else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            while let Some(item) = lines.peek().and_then(|l| l.trim_start().strip_prefix("- ")) {
                out.push(item.to_string());
                lines.next();
            }
        } else {
            let value = value.trim_start_matches('[').trim_end_matches(']');
            out.extend(value.split(',').map(str::to_string));
        }
    }
    out.into_iter()
        .map(|t| clean_tag(&t))
        .filter(|t| !t.is_empty())
        .collect()
}

/// A tag as written in front matter: no `#` or quotes, spaces as `-`.
pub(super) fn clean_tag(tag: &str) -> String {
    tag.trim()
        .trim_matches(['"', '\''])
        .trim_start_matches('#')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

/// `#tags` in Markdown text, as Obsidian reads them: `#` at the start or
/// after a space, then letters, digits, `_`, `-` or `/` with at least one
/// that isn't a digit. Not in code, and not a heading's `# `.
pub fn text_tags(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut in_code = false;
        let mut previous = ' ';
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '`' {
                in_code = !in_code;
            } else if c == '#' && !in_code && previous.is_whitespace() {
                let tag: String = chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
                    .collect();
                let tag = tag.trim_end_matches('/');
                if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit()) {
                    out.push(tag.to_string());
                    i += 1 + tag.chars().count();
                    previous = 'x';
                    continue;
                }
            }
            previous = c;
            i += 1;
        }
    }
    out
}

/// `front` (front matter with its `---` lines, or "") with `tags:` set to
/// `tags`, or taken out when there are none.
pub(super) fn with_front_tags(front: &str, tags: &[String]) -> String {
    let tags: Vec<String> = tags
        .iter()
        .map(|t| clean_tag(t))
        .filter(|t| !t.is_empty())
        .collect();
    let line = (!tags.is_empty()).then(|| format!("tags: [{}]", tags.join(", ")));
    if front.is_empty() {
        return line.map_or_else(String::new, |l| format!("---\n{l}\n---\n"));
    }
    let mut out: Vec<String> = Vec::new();
    let mut lines = front.lines().peekable();
    let mut placed = false;
    while let Some(l) = lines.next() {
        if l.starts_with("tags:") || l.starts_with("tag:") {
            // A list below it goes too.
            while lines
                .peek()
                .is_some_and(|n| n.trim_start().starts_with("- "))
            {
                lines.next();
            }
            if let Some(line) = &line
                && !placed
            {
                out.push(line.clone());
                placed = true;
            }
            continue;
        }
        // Before the closing `---`.
        if l.trim_end() == "---" && !out.is_empty() && !placed {
            if let Some(line) = &line {
                out.push(line.clone());
            }
            placed = true;
        }
        out.push(l.to_string());
    }
    format!("{}\n", out.join("\n"))
}

/// `key: value` from front matter, unquoted.
pub(super) fn front_matter_value(front: &str, key: &str) -> Option<String> {
    front.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix(':')?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// `agents: false` (or `no`, `off`) in front matter: the note is kept
/// from coding agents.
pub(super) fn hidden_from_agents(front: &str) -> bool {
    front_matter_value(front, "agents").is_some_and(|v| {
        ["false", "no", "off"]
            .iter()
            .any(|f| v.eq_ignore_ascii_case(f))
    })
}

/// `front` (front matter with its `---` lines, or "") with `key: value`
/// set, replacing the key's line if there is one, or with the key taken
/// out when `value` is `None`. Front matter left empty is dropped.
pub(super) fn with_front_value(front: &str, key: &str, value: Option<&str>) -> String {
    let is_key = |l: &str| l.strip_prefix(key).is_some_and(|r| r.starts_with(':'));
    let line = value.map(|v| format!("{key}: {}", yaml_scalar(v)));
    if front.is_empty() {
        return line.map_or_else(String::new, |l| format!("---\n{l}\n---\n"));
    }
    let inner: Vec<&str> = front
        .lines()
        .skip(1)
        .take_while(|l| l.trim_end() != "---")
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    for l in inner {
        if is_key(l) {
            if let Some(line) = line.as_ref().filter(|_| !placed) {
                out.push(line.clone());
                placed = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if let Some(line) = line.filter(|_| !placed) {
        out.push(line);
    }
    if out.iter().all(|l| l.trim().is_empty()) {
        return String::new();
    }
    format!("---\n{}\n---\n", out.join("\n"))
}

/// Whether `line` starts a part of a meeting note: a heading of its own
/// level or a transcript turn.
pub(super) fn starts_part(line: &str) -> bool {
    let line = line.trim_end();
    Turn::parse(line).is_some()
        || line.strip_prefix("## ").is_some_and(|h| {
            ["My notes", "Summary", "Transcript"]
                .iter()
                .any(|known| h.trim().eq_ignore_ascii_case(known))
        })
}

/// Where the transcript's first turn starts in `text`, if it has one.
pub fn transcript_start(text: &str) -> Option<usize> {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if Turn::parse(line).is_some() {
            return Some(at);
        }
        at += line.len();
    }
    None
}

/// What's under `## {heading}` in `body`, if it has that section.
pub fn section(body: &str, heading: &str) -> Option<String> {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| {
        l.trim_end()
            .strip_prefix("## ")
            .is_some_and(|h| h.trim().eq_ignore_ascii_case(heading))
    })?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| starts_part(l))
        .map_or(lines.len(), |i| start + 1 + i);
    Some(lines[start + 1..end].concat().trim().to_string())
}

/// `body` with `text` under `## {heading}`, replacing what was there (up
/// to the next `## ` heading or transcript turn). A new section goes
/// before the summary or transcript, or at the end; empty `text` takes the
/// section out.
pub fn with_section(body: &str, heading: &str, text: &str) -> String {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let is_heading = |l: &str| {
        l.trim_end()
            .strip_prefix("## ")
            .is_some_and(|h| h.trim().eq_ignore_ascii_case(heading))
    };
    let text = text.trim();
    let block = if text.is_empty() {
        String::new()
    } else {
        format!("## {heading}\n\n{text}\n\n")
    };
    if let Some(start) = lines.iter().position(|l| is_heading(l)) {
        let end = lines[start + 1..]
            .iter()
            .position(|l| starts_part(l))
            .map_or(lines.len(), |i| start + 1 + i);
        let before = lines[..start].concat();
        let after = lines[end..].concat();
        let block = if after.is_empty() && !block.is_empty() {
            format!("{}\n", block.trim_end())
        } else {
            block
        };
        return format!("{before}{block}{after}");
    }
    if block.is_empty() {
        return body.to_string();
    }
    // Before the transcript, or else before a note about the recording
    // ("*Ended at 0:07.*") when nobody spoke.
    let aside = |l: &str| {
        let l = l.trim();
        l.len() > 2 && l.starts_with('*') && !l.starts_with("**") && l.ends_with('*')
    };
    match lines
        .iter()
        .position(|l| starts_part(l))
        .or_else(|| lines.iter().position(|l| aside(l)))
    {
        Some(at) => {
            let before = lines[..at].concat();
            let before = before.trim_end();
            let gap = if before.is_empty() { "" } else { "\n\n" };
            format!("{before}{gap}{block}{}", lines[at..].concat())
        }
        None => {
            let before = body.trim_end();
            let gap = if before.is_empty() { "" } else { "\n\n" };
            format!("{before}{gap}{}\n", block.trim_end())
        }
    }
}

/// `body` with `text` under `## {heading}`, for what a hook's AI wrote: a
/// section of its own above the note's known parts, replacing what an
/// earlier run put under the same heading (up to the next `## ` heading of
/// any kind). `text` is made safe to sit there first: see `as_result`.
pub fn with_result(body: &str, heading: &str, text: &str) -> String {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let is_heading = |l: &str| {
        l.trim_end()
            .strip_prefix("## ")
            .is_some_and(|h| h.trim().eq_ignore_ascii_case(heading))
    };
    let text = as_result(text);
    let Some(start) = lines.iter().position(|l| is_heading(l)) else {
        return with_section(body, heading, &text);
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## ") || starts_part(l))
        .map_or(lines.len(), |i| start + 1 + i);
    let without = format!("{}{}", lines[..start].concat(), lines[end..].concat());
    with_section(&without, heading, &text)
}

/// What an AI wrote, made safe to sit in a note as one section: its own
/// `#` and `##` headings become `###` (they'd end the section, or name
/// one of the note's parts), and a line written like a transcript turn is
/// quoted, so the transcript still starts where it does.
pub fn as_result(text: &str) -> String {
    text.trim()
        .lines()
        .map(|line| {
            let marks = line.chars().take_while(|c| *c == '#').count();
            if (1..=2).contains(&marks) && line[marks..].starts_with(' ') {
                format!("###{}", &line[marks..])
            } else if Turn::parse(line).is_some() {
                format!("> {line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn front_matter_id(text: &str) -> Option<String> {
    let (front, _) = split_front_matter(text);
    front.lines().find_map(|line| {
        let id = line.strip_prefix("splashdown-id:")?.trim();
        let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        ok.then(|| id.to_string())
    })
}

/// The day a `## 2026-11-10` heading names: what the Inbox puts above
/// each day's lines, so that they can be told apart later.
pub fn day_heading(line: &str) -> Option<chrono::NaiveDate> {
    let day = line.trim_end().strip_prefix("## ")?.trim();
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// The last day a note has a day heading for.
pub fn last_day(text: &str) -> Option<chrono::NaiveDate> {
    text.lines().rev().find_map(day_heading)
}

/// The time a dictated line starts with (`**09:15** …`), and the rest.
pub fn timed(line: &str) -> Option<(chrono::NaiveTime, &str)> {
    let (time, rest) = line.strip_prefix("**")?.split_once("**")?;
    let time = chrono::NaiveTime::parse_from_str(time, "%H:%M").ok()?;
    Some((time, rest.trim_start()))
}

/// The first line with text, without Markdown heading marks.
pub fn title_of(body: &str) -> String {
    body.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

/// A title made safe as a file name on macOS and Windows: no `\ / : * ? "
/// < > |` or control characters, no trailing dots or spaces, not a
/// reserved Windows name, at most `NAME_CHARS` characters.
pub fn file_name_part(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "\\/:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut name: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(NAME_CHARS)
        .collect();
    name = name
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_string();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&name)) {
        name.push('_');
    }
    name
}

/// A YAML value that's safe whatever the app name contains.
pub(super) fn yaml_scalar(s: &str) -> String {
    let plain = s.chars().all(|c| c.is_alphanumeric() || " .-_".contains(c)) && !s.starts_with(' ');
    if plain {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}
