//! A hook that asks the AI: the user's own instruction ("draft a follow-up
//! email to the people in this meeting") run on the text of what happened,
//! and the result put where they chose: a section of the note, another
//! note, a file, the clipboard, a web address, their Reminders or an email
//! draft.
//!
//! The AI only writes. It has no tools, so the text it reads (a meeting is
//! mostly other people's words) can at worst change what it writes, never
//! where that goes or what else happens. Reminders and an email draft keep
//! to that: Splashdown does one fixed thing with the text (a reminder for
//! each to-do line; a draft that is never sent, to the addresses typed in
//! Settings or to no one), and nothing in the text chooses another. The
//! text goes to the AI that
//! follows instructions (Settings → AI Models): the one on this computer,
//! or a cloud one, as Settings says where the hook is added. A text too
//! long for the model is read in parts (`splashdown_core::instruct`).

use std::path::Path;

use chrono::NaiveDate;
use splashdown_config::PutIn;
use splashdown_core::instruct::{self, About};

use super::*;

/// What the AI is asked, and where its answer goes.
pub(super) struct Asked<'a> {
    pub name: &'a str,
    pub instruction: &'a str,
    pub put: PutIn,
    pub to: Option<&'a str>,
}

impl<'a> Asked<'a> {
    pub(super) fn of(hook: &'a Hook) -> Option<Self> {
        match &hook.run {
            HookTarget::Ask {
                name,
                instruction,
                put,
                to,
            } => Some(Asked {
                name: name.trim(),
                instruction,
                put: *put,
                to: to.as_deref().map(str::trim),
            }),
            _ => None,
        }
    }
}

/// Asks, and puts the answer where it goes: what was done, in a few words,
/// with what was written.
pub(super) fn run(app: &AppHandle, asked: &Asked<'_>, happened: &Happened) -> Result<Did, String> {
    // Before the AI is asked: there would be nowhere to put its answer.
    if asked.put == PutIn::Reminders && !cfg!(target_os = "macos") {
        return Err(NO_REMINDERS.into());
    }
    // The same for a list that isn't there: said now, not after the AI
    // has spent its time writing.
    if asked.put == PutIn::Reminders
        && let Some(list) = asked.to.map(str::trim).filter(|l| !l.is_empty())
    {
        reminders_list(list)?;
    }
    let Some(text) = write(app, asked.instruction, happened)? else {
        return Ok(Did::only("Nothing to do"));
    };
    let said = put(app, asked, happened, &text)?;
    Ok(Did {
        said,
        wrote: Some(text),
    })
}

/// The note what was written went into, by its place in the notes folder,
/// when it went into one: to open it from the notice.
pub(super) fn note_of(asked: &Asked<'_>, happened: &Happened) -> Option<String> {
    match asked.put {
        PutIn::Note => happened
            .note
            .as_ref()
            .map(|n| n.rel.clone())
            .filter(|rel| !rel.is_empty()),
        PutIn::OtherNote => Some(other_note(asked.to.unwrap_or_default()).0),
        _ => None,
    }
}

/// What the AI wrote for `instruction` about what happened; `None` when it
/// found nothing to do.
pub(super) fn write(
    app: &AppHandle,
    instruction: &str,
    happened: &Happened,
) -> Result<Option<String>, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or("Splashdown is still starting.")?;
    let setup = state
        .session
        .answer_model()?
        .ok_or("Asking the AI needs an AI model: choose one in Settings → AI Models.")?;
    let about = About {
        what: what(happened),
        title: happened.note.as_ref().map(|n| n.title.clone()),
        user_name: setup.user_name,
        today: splashdown_core::prompt::today(Local::now().date_naive()),
    };
    let model = setup.model;
    let result = tauri::async_runtime::block_on(instruct::run(
        model.as_ref(),
        instruction,
        &happened.text,
        &about,
    ));
    let usage = match &result {
        Ok(done) => done.usage,
        Err(failed) => failed.usage,
    };
    crate::spend::shared().record(&[(model.name(), usage)], Local::now());
    match result {
        Ok(done) => {
            tracing::info!(
                "a hook asked {} ({} part(s) read)",
                model.name(),
                done.parts
            );
            Ok(Some(done.text).filter(|t| !t.trim().is_empty()))
        }
        Err(failed) => Err(crate::errors::pipeline(
            &splashdown_core::PipelineError::Llm(failed.error),
            None,
            Some(&model.name()),
        )),
    }
}

/// What the text is, for the prompt.
fn what(happened: &Happened) -> String {
    instruct::what(happened.event, happened.period().as_deref())
}

fn put(
    app: &AppHandle,
    asked: &Asked<'_>,
    happened: &Happened,
    text: &str,
) -> Result<String, String> {
    let to = asked.to.unwrap_or_default();
    match asked.put {
        PutIn::Note => {
            let note = happened
                .note
                .as_ref()
                .filter(|n| !n.rel.is_empty())
                .ok_or("there's no note to put it in")?;
            notes(app)?
                .set_result(&note.rel, asked.name, text)
                .map_err(|e| format!("couldn't add it to the note: {e}"))?;
            Ok("Added to the note".into())
        }
        PutIn::OtherNote => {
            let (rel, title) = other_note(to);
            notes(app)?
                .add_under(
                    &rel,
                    &title,
                    heading(happened).as_deref(),
                    text,
                    // A schedule's heading is the days it covers: run again
                    // for the same days, it says the same thing better, not
                    // a second thing. A meeting's is its title and the day,
                    // which two meetings can share, so those are both kept.
                    happened.event == HookEvent::Schedule.name(),
                )
                .map_err(|e| format!("couldn't add it to {title}: {e}"))?;
            Ok(format!("Added to {title}"))
        }
        PutIn::File => {
            add_to_file(Path::new(to), heading(happened).as_deref(), text)?;
            let name = Path::new(to)
                .file_name()
                .map_or_else(|| to.to_string(), |n| n.to_string_lossy().into_owned());
            Ok(format!("Added to {name}"))
        }
        PutIn::Clipboard => {
            splashdown_platform::copy_to_clipboard(text)
                .map_err(|e| format!("couldn't copy it: {e}"))?;
            Ok("Copied to the clipboard".into())
        }
        PutIn::Web => {
            send(to, happened.event, sent(asked.name, happened, text)?)?;
            Ok(format!("Sent to {}", host(to)))
        }
        PutIn::Reminders => remind(app, asked, happened, text),
        PutIn::EmailDraft => {
            let draft = draft(asked, happened, text, DRAFT_LINK_MOST)?;
            if !draft.whole {
                splashdown_platform::copy_to_clipboard(&letter(text)).map_err(|e| {
                    format!("too long for an email draft, and couldn't copy it: {e}")
                })?;
            }
            splashdown_platform::open_url(&draft.link)
                .map_err(|e| format!("couldn't open an email draft: {e}"))?;
            Ok(if draft.whole {
                "Opened as an email draft".into()
            } else {
                "Too long for an email draft: copied to the clipboard, to paste into the draft that opened".into()
            })
        }
    }
}

// ---------------------------------------------------------------- reminders

/// Why no reminders are added off the Mac.
const NO_REMINDERS: &str = "Splashdown can only add reminders on a Mac";
/// The most reminders one run adds: a result gone wrong shouldn't fill the
/// list.
const TODOS_MOST: usize = 25;
/// The longest a reminder's title is, in characters.
const TODO_LONGEST: usize = 200;
/// How many to-dos the question before adding them names.
const TODOS_ASKED: usize = 6;

/// A to-do in what the AI wrote, to add as a reminder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Todo {
    pub text: String,
    /// The day it's due (`2026-11-12`), with the time if the line has one
    /// (`2026-11-12T15:00`). `None` unless the line says so plainly.
    pub due: Option<String>,
}

/// The to-dos in what the AI wrote: each open to-do line (`- [ ] …`), or,
/// when it wrote none of those at all, each plain list line. Anything else
/// (headings, sentences, to-dos ticked off) is left out.
pub(super) fn todos(text: &str, today: NaiveDate) -> Vec<Todo> {
    let lines: Vec<(Option<bool>, String)> = text
        .lines()
        .filter_map(list_line)
        .map(|(open, line)| (open, plain(line)))
        .filter(|(_, line)| line.chars().any(char::is_alphanumeric))
        .collect();
    let boxes = lines.iter().any(|(open, _)| open.is_some());
    lines
        .into_iter()
        .filter(|(open, _)| !boxes || *open == Some(true))
        .map(|(_, line)| Todo {
            due: due(&line, today),
            text: shortened(&line, TODO_LONGEST),
        })
        .collect()
}

/// A list line's text, and whether it has a box: open (`[ ]`), ticked
/// (`[x]`), or none. `None` for a line that isn't in a list.
fn list_line(line: &str) -> Option<(Option<bool>, &str)> {
    let line = line.trim();
    let numbered = || {
        let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
        (rest.len() < line.len())
            .then(|| rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")))
            .flatten()
    };
    let item = ["- ", "* ", "+ ", "• "]
        .iter()
        .find_map(|mark| line.strip_prefix(mark))
        .or_else(numbered);
    // A box with no list mark before it is a to-do all the same.
    let rest = item.unwrap_or(line).trim_start();
    for (mark, open) in [("[ ]", true), ("[]", true), ("[x]", false), ("[X]", false)] {
        if let Some(text) = rest.strip_prefix(mark) {
            return Some((Some(open), text.trim()));
        }
    }
    item.map(|text| (None, text.trim()))
}

/// A line without Markdown's marks for bold and code, on one line.
fn plain(line: &str) -> String {
    line.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text`, cut to `most` characters with an ellipsis if it's longer.
fn shortened(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_string();
    }
    let cut: String = text.chars().take(most - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The day a to-do is due, if its line says so plainly: one date written
/// as `2026-11-12`, today or later, with the time if one follows it
/// (`2026-11-12 15:00`). A day said in words ("by Wednesday") stays in the
/// reminder's text and sets no date: working one out could get it wrong.
fn due(line: &str, today: NaiveDate) -> Option<String> {
    let bytes = line.as_bytes();
    let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
    let dates: Vec<usize> = (0..bytes.len())
        .filter(|&i| {
            (0..10).all(|k| match k {
                4 | 7 => bytes.get(i + k) == Some(&b'-'),
                _ => digit(i + k),
            }) && !(i > 0 && digit(i - 1))
                && !digit(i + 10)
        })
        .collect();
    // Two dates on a line: which one it's due isn't plain.
    let [at] = dates[..] else {
        return None;
    };
    let day = NaiveDate::parse_from_str(&line[at..at + 10], "%Y-%m-%d")
        .ok()
        .filter(|day| *day >= today)?;
    Some(match time_after(&line[at + 10..]) {
        Some((hour, minute)) => format!("{day}T{hour:02}:{minute:02}"),
        None => day.to_string(),
    })
}

/// A time on the 24-hour clock right after a date: "T15:00", " 15:00",
/// " at 9:30". Not one with "am" or "pm", which is left in the text.
fn time_after(rest: &str) -> Option<(u32, u32)> {
    let rest = ["T", " at ", ", ", " "]
        .iter()
        .find_map(|lead| rest.strip_prefix(lead))?;
    let (hour, after) = rest.split_once(':')?;
    let minute = after.get(..2)?;
    let digits = |s: &str| !s.is_empty() && s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit());
    let after = after[2..].trim_start().to_ascii_lowercase();
    if !digits(hour)
        || !digits(minute)
        || after.starts_with(|c: char| c.is_ascii_digit() || c == ':')
        || ["am", "pm", "a.m", "p.m"]
            .iter()
            .any(|half| after.starts_with(half))
    {
        return None;
    }
    let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// What a reminder's notes say: the meeting or note it came from and the
/// day, and the hook that added it.
fn came_from(name: &str, happened: &Happened) -> String {
    match heading(happened) {
        Some(heading) => format!("{heading} · {name} (Splashdown)"),
        None => format!("{name} (Splashdown)"),
    }
}

/// "3 reminders", "1 reminder".
fn reminders(count: usize) -> String {
    if count == 1 {
        "1 reminder".into()
    } else {
        format!("{count} reminders")
    }
}

/// Adds a reminder for each to-do in what the AI wrote, asking first when
/// the user has chosen to be asked before events and reminders are added
/// (Settings → Actions → Choices), as a spoken "remind me to…" does.
fn remind(
    app: &AppHandle,
    asked: &Asked<'_>,
    happened: &Happened,
    text: &str,
) -> Result<String, String> {
    let found = todos(text, Local::now().date_naive());
    if found.is_empty() {
        return Ok("No to-dos in what it wrote: no reminders added".into());
    }
    let adding = &found[..found.len().min(TODOS_MOST)];
    let settings = app
        .try_state::<AppState>()
        .map(|s| s.settings())
        .ok_or("Splashdown is still starting.")?;
    if settings.actions.confirm_events {
        let title = match adding.len() {
            1 => "Add this reminder?".to_string(),
            count => format!("Add these {count} reminders?"),
        };
        let answer = crate::prompt::ask(
            app,
            &title,
            &question(asked.name, happened, adding),
            &["Add", "Cancel"],
        );
        if answer != Some(0) {
            return Ok("Not added".into());
        }
    }
    let notes = came_from(asked.name, happened);
    // The list the user named in Settings, or their default one.
    let list = asked.to.map(str::trim).filter(|l| !l.is_empty());
    for (done, todo) in adding.iter().enumerate() {
        add_reminder(todo, &notes, list).map_err(|e| match done {
            0 => e,
            done => format!("added {} of {}, then: {e}", done, adding.len()),
        })?;
    }
    let to = list.map_or_else(String::new, |list| format!(" to {list}"));
    Ok(if adding.len() < found.len() {
        format!(
            "Added the first {} of {} to-dos as reminders{to}",
            adding.len(),
            found.len()
        )
    } else {
        format!("Added {}{to}", reminders(adding.len()))
    })
}

/// What the question before adding reminders says: which hook, what
/// about, and the first few to-dos.
fn question(name: &str, happened: &Happened, adding: &[Todo]) -> String {
    let shown: Vec<String> = adding
        .iter()
        .take(TODOS_ASKED)
        .map(|todo| format!("“{}”", shortened(&todo.text, 80)))
        .collect();
    let more = match adding.len().saturating_sub(TODOS_ASKED) {
        0 => String::new(),
        more => format!(", and {more} more"),
    };
    let about = heading(happened).map_or_else(String::new, |h| format!(" for {h}"));
    format!("{name} wrote{about}: {}{more}.", shown.join(", "))
}

/// Checks that Reminders has the list a hook adds to.
#[cfg(target_os = "macos")]
fn reminders_list(list: &str) -> Result<(), String> {
    match tauri::async_runtime::block_on(splashdown_apple_shim::reminders_list_exists(list)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(no_such_list(list)),
        Err(e) => Err(crate::voice_actions::calendar_error(
            e,
            "reminders",
            "Reminders",
        )),
    }
}

#[cfg(not(target_os = "macos"))]
fn reminders_list(_: &str) -> Result<(), String> {
    Ok(())
}

/// What's said when a hook's list isn't in Reminders.
fn no_such_list(list: &str) -> String {
    format!(
        "Reminders has no list called “{list}”: make it there, or leave List empty for your default list"
    )
}

#[cfg(target_os = "macos")]
fn add_reminder(todo: &Todo, notes: &str, list: Option<&str>) -> Result<(), String> {
    tauri::async_runtime::block_on(splashdown_apple_shim::add_reminder(
        &todo.text,
        todo.due.as_deref(),
        Some(notes),
        list,
    ))
    .map(|_| ())
    .map_err(|e| crate::voice_actions::calendar_error(e, "reminders", "Reminders"))
}

#[cfg(not(target_os = "macos"))]
fn add_reminder(_: &Todo, _: &str, _: Option<&str>) -> Result<(), String> {
    Err(NO_REMINDERS.into())
}

// ---------------------------------------------------------------- an email draft

/// The longest link a draft is opened with. A draft opens through a
/// `mailto:` link, and what takes it cuts a long one short or refuses it:
/// about 2,000 characters on Windows, more on a Mac (a mail app in the
/// browser takes about 8,000). Past that the text goes on the clipboard.
const DRAFT_LINK_MOST: usize = if cfg!(windows) { 2_000 } else { 8_000 };
/// The longest a draft's subject is, in characters.
const SUBJECT_LONGEST: usize = 150;
/// What a draft says when the text was too long to open in it.
const PASTE_HERE: &str =
    "(What was written is too long to open in a draft. It's on your clipboard: paste it here.)";

/// The link that opens a draft, and whether it holds the whole text.
#[derive(Debug)]
struct Draft {
    link: String,
    /// False when the text was too long for a link: the draft then says to
    /// paste it.
    whole: bool,
}

/// The link that opens what the AI wrote as a draft in the mail app. It's
/// to the addresses the user typed in Settings (`to`), or to no one: never
/// to an address from the text, which is only ever the body. The subject
/// is the hook's name and what it's about.
fn draft(asked: &Asked<'_>, happened: &Happened, text: &str, most: usize) -> Result<Draft, String> {
    let to = PutIn::addresses(asked.to.unwrap_or_default())
        .ok_or("the draft's addresses aren't plain email addresses")?
        .join(",");
    let subject = subject(asked.name, happened);
    let link = |body: &str| crate::voice_actions::mailto(Some(&to), Some(&subject), body);
    // Mail wants its lines to end the long way.
    let body = letter(text).replace('\n', "\r\n");
    let whole = link(&body)?;
    if whole.len() <= most {
        return Ok(Draft {
            link: whole,
            whole: true,
        });
    }
    Ok(Draft {
        link: link(PASTE_HERE)?,
        whole: false,
    })
}

/// A draft's subject: the hook's name, and the note's or meeting's title
/// or the days a scheduled hook covers.
fn subject(name: &str, happened: &Happened) -> String {
    let about = happened
        .note
        .as_ref()
        .map(|note| note.title.clone())
        .or_else(|| happened.meeting.as_ref().map(|m| m.title.clone()))
        .or_else(|| happened.period())
        .map(|about| plain(&about))
        .filter(|about| !about.is_empty());
    let subject = match about {
        Some(about) => format!("{name}: {about}"),
        None => name.to_string(),
    };
    shortened(&plain(&subject), SUBJECT_LONGEST)
}

/// What the AI wrote as an email shows it: without Markdown's marks for
/// headings, bold and to-do boxes, which a mail app would show as they
/// are.
fn letter(text: &str) -> String {
    text.trim()
        .lines()
        .map(|line| {
            let heading = line.trim_start_matches('#');
            let line = match heading.strip_prefix(' ') {
                Some(heading) if heading.len() + 1 < line.len() => heading,
                _ => line,
            };
            let line = line.trim_end().replace("**", "").replace("__", "");
            match line.split_once("- [ ] ") {
                Some((indent, todo)) if indent.trim().is_empty() => format!("{indent}- {todo}"),
                _ => line,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------- Test

/// What would be done with `text` where it's more than putting it
/// somewhere whole, for Test: which lines would become reminders, or how
/// a draft would open. Nothing is added or opened.
pub(super) fn would(asked: &Asked<'_>, happened: &Happened, text: &str) -> Option<String> {
    match asked.put {
        PutIn::Reminders => {
            let found = todos(text, Local::now().date_naive());
            if found.is_empty() {
                return Some(
                    "None of these lines is a to-do, so no reminder would be added.".into(),
                );
            }
            let adding = &found[..found.len().min(TODOS_MOST)];
            let lines: Vec<String> = adding
                .iter()
                .map(|todo| match &todo.due {
                    Some(due) => format!("• {} (due {})", todo.text, due.replace('T', " ")),
                    None => format!("• {}", todo.text),
                })
                .collect();
            let only = if adding.len() < found.len() {
                format!(" (the first of {} to-dos)", found.len())
            } else {
                String::new()
            };
            Some(format!(
                "This would add {}{only}:\n{}",
                reminders(adding.len()),
                lines.join("\n")
            ))
        }
        PutIn::EmailDraft => {
            let to = PutIn::addresses(asked.to.unwrap_or_default())?;
            let to = if to.is_empty() {
                "no one".to_string()
            } else {
                to.join(", ")
            };
            let long = match draft(asked, happened, text, DRAFT_LINK_MOST) {
                Ok(draft) if !draft.whole => {
                    " It's too long to open in a draft, so it would go on the clipboard to paste in."
                }
                _ => "",
            };
            Some(format!(
                "This would open as an email draft to {to}, with the subject “{}”.{long}",
                subject(asked.name, happened)
            ))
        }
        _ => None,
    }
}

/// The notes folder.
fn notes(app: &AppHandle) -> Result<std::sync::Arc<crate::notes::store::Store>, String> {
    let settings = app
        .try_state::<AppState>()
        .map(|s| s.settings())
        .ok_or("Splashdown is still starting.")?;
    Ok(crate::notes::store_for(settings.notes.folder.clone())?.0)
}

/// The note a result is added to, as the user named it ("Commitments",
/// "Work/Decisions.md"): its path in the notes folder, and its title.
pub(super) fn other_note(to: &str) -> (String, String) {
    let named = to.trim().replace('\\', "/");
    let named = named.trim_matches('/');
    let stem = if named.to_ascii_lowercase().ends_with(".md") {
        &named[..named.len() - 3]
    } else {
        named
    };
    let title = stem.rsplit('/').next().unwrap_or(stem).trim().to_string();
    (format!("{stem}.md"), title)
}

/// The heading a result goes under in a note or file that collects them:
/// the meeting's or transcript's title and the day, the days a scheduled
/// hook's notes are from, or the title of the meeting about to start and
/// its day. None for a dictation or a note from the phone, whose results
/// are added as they are.
fn heading(happened: &Happened) -> Option<String> {
    if let Some(days) = happened.period() {
        return Some(days);
    }
    if let Some(meeting) = &happened.meeting {
        let day = meeting.starts.get(..10).unwrap_or(&meeting.starts);
        return Some(format!("{} · {day}", meeting.title.trim()));
    }
    let note = happened.note.as_ref()?;
    let day = happened.at.get(..10).unwrap_or(&happened.at);
    Some(format!("{} · {day}", note.title.trim()))
}

/// Adds the result to the end of a file, making it if it isn't there.
fn add_to_file(path: &Path, heading: Option<&str>, text: &str) -> Result<(), String> {
    use std::io::Write;
    let empty = std::fs::metadata(path).map_or(true, |m| m.len() == 0);
    let gap = if empty { "" } else { "\n" };
    let block = match heading {
        Some(heading) => format!("{gap}## {heading}\n\n{}\n", text.trim()),
        None => format!("{gap}{}\n", text.trim()),
    };
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(block.as_bytes()))
        .map_err(|e| format!("couldn't add it to {}: {e}", path.display()))
}

/// What a web address gets: as any webhook, with `text` holding what the
/// AI wrote rather than what it was written from, and the hook's name.
fn sent(name: &str, happened: &Happened, text: &str) -> Result<String, String> {
    let written = Happened {
        text: text.to_string(),
        ..happened.clone()
    };
    let mut json = serde_json::to_value(&written).map_err(|e| e.to_string())?;
    if let Some(fields) = json.as_object_mut() {
        fields.insert("ask".into(), name.into());
    }
    Ok(json.to_string())
}

/// A meeting to try an instruction on, for Test.
const SAMPLE_MEETING: &str = "# Pricing review\n\nWith Dana Lee.\n\n## Summary\n\nThe new pricing page goes live on Friday, once Dana has the final numbers.\n\n## Transcript\n\n**[0:00] You:** Thanks for joining. Where are we on the pricing page?\n\n**[0:06] Dana Lee:** The design is done. I still need the final numbers from you.\n\n**[0:13] You:** I'll send you the numbers by Wednesday.\n\n**[0:18] Dana Lee:** Then I can have it live on Friday. Should we tell support first?\n\n**[0:25] You:** Yes. I'll write to support tomorrow.\n\n**[0:29] Dana Lee:** Great. Let's check in again next Tuesday.\n";

const SAMPLE_SAID: &str = "Remind me to call the dentist tomorrow morning. And an idea: a weekly digest of what I agreed to in meetings.";

/// What Test asks about: enough of a meeting, or of something said, for
/// an instruction to have something to do.
pub(super) fn sample(event: HookEvent) -> Happened {
    let mut happened = super::sample(event);
    match happened.note.as_mut() {
        Some(note) => {
            note.title = "Pricing review".into();
            note.summary = crate::notes::markdown::section(SAMPLE_MEETING, "Summary")
                .filter(|s| !s.is_empty());
            happened.text = SAMPLE_MEETING.into();
        }
        // These come with notes of their own to try it on.
        None if matches!(event, HookEvent::Schedule | HookEvent::BeforeMeeting) => {}
        None => happened.text = SAMPLE_SAID.into(),
    }
    happened
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_goes_to_the_note_the_user_named() {
        assert_eq!(
            other_note("Commitments"),
            ("Commitments.md".into(), "Commitments".into())
        );
        assert_eq!(
            other_note(" Work/Decisions.MD "),
            ("Work/Decisions.md".into(), "Decisions".into())
        );
        assert_eq!(
            other_note(r"Work\To-dos"),
            ("Work/To-dos.md".into(), "To-dos".into())
        );
    }

    #[test]
    fn results_collect_under_the_meeting_and_day() {
        let meeting = sample(HookEvent::Meeting);
        let heading = heading(&meeting).unwrap();
        assert!(heading.starts_with("Pricing review · 20"), "{heading}");
        assert_eq!(
            heading.chars().count(),
            "Pricing review · 2026-11-09".chars().count()
        );
        assert!(super::heading(&sample(HookEvent::PhoneNote)).is_none());
        // A brief goes under the meeting to come and its day.
        let coming = calendar::Event {
            id: "A1".into(),
            starts: DateTime::parse_from_rfc3339("2026-11-17T14:00:00+00:00")
                .unwrap()
                .with_timezone(&Local),
            all_day: false,
            declined: false,
            title: " Pricing review ".into(),
            people: vec!["Dana Lee".into()],
        };
        let brief = Happened::coming(String::new(), &coming, Local::now());
        let day = coming.starts.format("%Y-%m-%d");
        assert_eq!(
            super::heading(&brief).unwrap(),
            format!("Pricing review · {day}")
        );
        assert!(what(&brief).contains("about to start"));
        assert!(
            sample(HookEvent::BeforeMeeting)
                .text
                .contains("[An earlier meeting, ")
        );
        assert!(meeting.text.contains("**[0:13] You:**"));
        assert!(sample(HookEvent::Dictation).text.contains("dentist"));

        let dir = std::env::temp_dir().join(format!("sd-hooks-ask-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("log.md");
        add_to_file(
            &file,
            Some("Weekly sync · 2026-11-09"),
            "- [ ] Send the deck\n",
        )
        .unwrap();
        add_to_file(&file, None, "Call the dentist").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "## Weekly sync · 2026-11-09\n\n- [ ] Send the deck\n\nCall the dentist\n"
        );
        assert!(add_to_file(&dir.join("no/such/folder/x.md"), None, "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_web_address_gets_what_the_ai_wrote() {
        let meeting = sample(HookEvent::Meeting);
        let body = sent("Follow-up email", &meeting, "Hi Dana,").unwrap();
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["event"], "meeting");
        assert_eq!(json["ask"], "Follow-up email");
        assert_eq!(json["text"], "Hi Dana,");
        assert_eq!(json["note"]["title"], "Pricing review");
        // Not the transcript it was written from.
        assert!(!body.contains("final numbers from you"));
        assert!(json.get("rel").is_none() && json["note"].get("rel").is_none());
    }

    fn asked<'a>(put: PutIn, to: Option<&'a str>) -> Asked<'a> {
        Asked {
            name: "Follow-up email",
            instruction: "Draft a follow-up email.",
            put,
            to,
        }
    }

    fn day(day: &str) -> NaiveDate {
        NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn each_to_do_line_becomes_a_reminder() {
        let today = day("2026-11-10");
        let texts = |text: &str| -> Vec<String> {
            todos(text, today).into_iter().map(|t| t.text).collect()
        };
        // Open to-dos only: not the heading, the sentence, the decision or
        // the one ticked off.
        let review = "## For me to do\n\nHere is what you took on:\n\n- [ ] Send Dana the **final numbers** by Wednesday\n* [ ] Write to `support` tomorrow\n  - [ ] Check   the EU prices\n1. [ ] Book the room\n[ ] Call Sam\n- [x] Share the deck\n\n## Decided\n\n- The pricing page goes live on Friday\n";
        assert_eq!(
            texts(review),
            [
                "Send Dana the final numbers by Wednesday",
                "Write to support tomorrow",
                "Check the EU prices",
                "Book the room",
                "Call Sam",
            ]
        );
        // With no to-do lines at all, each plain list line is one.
        assert_eq!(
            texts("To do:\n- Send the numbers\n2) Write to support\n• Call Sam\n\nThanks!"),
            ["Send the numbers", "Write to support", "Call Sam"]
        );
        // Everything ticked off, or nothing in a list: none.
        assert!(texts("- [x] Send the numbers\n- [X] Call Sam").is_empty());
        assert!(texts("Hi Dana,\n\nThanks for today.\n\n---\n- \n- [ ] \n- - -").is_empty());
        assert!(texts("").is_empty());
        // A long line is cut, not left out.
        let long = format!("- [ ] {}", "word ".repeat(100));
        let cut = &todos(&long, today)[0].text;
        assert_eq!(cut.chars().count(), TODO_LONGEST);
        assert!(cut.ends_with('…'));
        // The words of a line can't add anything but that line.
        let odd = todos("- [ ] Pay Sam\\n- [ ] and another\tone", today);
        assert_eq!(odd.len(), 1);
        assert_eq!(odd[0].text, "Pay Sam\\n- [ ] and another one");
    }

    #[test]
    fn a_reminder_is_due_only_when_its_line_says_so_plainly() {
        let today = day("2026-11-10");
        let due = |line: &str| todos(line, today).pop().unwrap().due;
        // Days in words stay words.
        assert_eq!(due("- [ ] Send Dana the numbers by Wednesday"), None);
        assert_eq!(due("- [ ] Write to support tomorrow at 9"), None);
        assert_eq!(due("- [ ] Call Sam on 12 November"), None);
        assert_eq!(due("- [ ] Call Sam on 11/12/2026"), None);
        // A whole date, today or later.
        assert_eq!(
            due("- [ ] Send the numbers by 2026-11-12").as_deref(),
            Some("2026-11-12")
        );
        assert_eq!(
            due("- [ ] File it (due 2026-11-10)").as_deref(),
            Some("2026-11-10")
        );
        // With its time, on the 24-hour clock.
        for timed in [
            "- [ ] Call Sam 2026-11-12 15:00",
            "- [ ] Call Sam 2026-11-12T15:00",
            "- [ ] Call Sam on 2026-11-12 at 15:00.",
        ] {
            assert_eq!(due(timed).as_deref(), Some("2026-11-12T15:00"), "{timed}");
        }
        assert_eq!(
            due("- [ ] Call Sam 2026-11-12 at 9:30").as_deref(),
            Some("2026-11-12T09:30")
        );
        // A time that could be read two ways is left in the words.
        for unsure in [
            "- [ ] Call Sam 2026-11-12 3:00 pm",
            "- [ ] Call Sam 2026-11-12 25:00",
            "- [ ] Call Sam 2026-11-12 with Dana: the 15:00 slot",
            "- [ ] Call Sam 2026-11-12 15:00:30",
        ] {
            assert_eq!(due(unsure).as_deref(), Some("2026-11-12"), "{unsure}");
        }
        // Not a day gone by, a day that doesn't exist, or one of two.
        assert_eq!(due("- [ ] Follow up on the 2026-11-03 outage"), None);
        assert_eq!(due("- [ ] Send it by 2026-13-40"), None);
        assert_eq!(due("- [ ] Move it from 2026-11-12 to 2026-11-19"), None);
        assert_eq!(due("- [ ] Ticket 12026-11-123"), None);
        // What a reminder's notes say.
        assert!(
            came_from("My commitments", &sample(HookEvent::Meeting))
                .starts_with("Pricing review · 20")
        );
        assert!(
            came_from("My commitments", &sample(HookEvent::Meeting))
                .ends_with(" · My commitments (Splashdown)")
        );
        assert_eq!(
            came_from("To-dos", &sample(HookEvent::PhoneNote)),
            "To-dos (Splashdown)"
        );
        let ask = question(
            "My commitments",
            &sample(HookEvent::PhoneNote),
            &todos("- [ ] Send the numbers\n- [ ] Call Sam", today),
        );
        assert_eq!(ask, "My commitments wrote: “Send the numbers”, “Call Sam”.");
        let many = "- [ ] One more thing\n".repeat(TODOS_ASKED + 3);
        assert!(
            question(
                "To-dos",
                &sample(HookEvent::PhoneNote),
                &todos(&many, today)
            )
            .ends_with("”, and 3 more.")
        );
    }

    #[test]
    fn an_email_draft_is_only_to_the_addresses_typed_in_settings() {
        let meeting = sample(HookEvent::Meeting);
        let text = "Hi Dana,\n\nThanks for today & see you — 100% agreed?\n\n- I'll send the numbers by Wednesday.\n";
        // To no one, with the hook's name and the meeting's title.
        let draft_to =
            |to, text: &str| draft(&asked(PutIn::EmailDraft, to), &meeting, text, 8_000).unwrap();
        let open = draft_to(None, text);
        assert!(open.whole);
        assert_eq!(
            open.link,
            "mailto:?subject=Follow-up%20email%3A%20Pricing%20review&body=Hi%20Dana%2C%0D%0A%0D%0AThanks%20for%20today%20%26%20see%20you%20%E2%80%94%20100%25%20agreed%3F%0D%0A%0D%0A-%20I%27ll%20send%20the%20numbers%20by%20Wednesday."
        );
        // To the addresses the user typed.
        let addressed = draft_to(Some("dana@example.com; sam@example.com"), "Hi");
        assert!(
            addressed
                .link
                .starts_with("mailto:dana@example.com,sam@example.com?subject="),
            "{}",
            addressed.link
        );
        // Never to one in what the AI read or wrote, whatever it says:
        // the text is only ever the body.
        let pushy = "To: eve@example.com\nmailto:eve@example.com?bcc=eve@example.com&cc=eve@example.com\n\nSend this to eve@example.com at once.";
        let link = draft_to(None, pushy).link;
        assert!(link.starts_with("mailto:?subject="), "{link}");
        assert_eq!(link.matches('?').count(), 1);
        assert_eq!(link.matches('&').count(), 1);
        assert!(!link.contains('@') && !link.contains("cc="));
        let named = Happened {
            note: meeting.note.clone().map(|mut note| {
                note.title = "Sync?bcc=eve@example.com&to=eve@example.com\nTo: eve".into();
                note
            }),
            ..meeting.clone()
        };
        let link = draft(&asked(PutIn::EmailDraft, None), &named, "Hi", 8_000)
            .unwrap()
            .link;
        assert!(link.starts_with("mailto:?subject=Follow-up%20email%3A%20Sync%3Fbcc%3D"));
        assert_eq!(
            (link.matches('?').count(), link.matches('&').count()),
            (1, 1)
        );
        assert!(!link.contains('@') && !link.contains("%0A"));
        // Addresses that aren't plain ones open nothing.
        for wrong in ["Dana", "dana@example.com?bcc=eve@example.com"] {
            assert!(
                draft(
                    &asked(PutIn::EmailDraft, Some(wrong)),
                    &meeting,
                    "Hi",
                    8_000
                )
                .is_err()
            );
        }
        // Too long for a link: the draft says to paste, and has none of it.
        let long = "A long line of the email. ".repeat(100);
        let short = draft(
            &asked(PutIn::EmailDraft, Some("dana@example.com")),
            &meeting,
            &long,
            2_000,
        )
        .unwrap();
        assert!(!short.whole && short.link.len() < 400);
        assert!(short.link.starts_with("mailto:dana@example.com?subject=Follow-up%20email%3A%20Pricing%20review&body=%28What%20was%20written%20is%20too%20long"));
        assert!(!short.link.contains("long%20line"));
        assert!(draft_to(None, &long).whole, "a Mac's link takes it");
        // Just short enough, and one character over.
        let fits = draft_to(None, "x").link.len();
        assert!(
            draft(&asked(PutIn::EmailDraft, None), &meeting, "x", fits)
                .unwrap()
                .whole
        );
        assert!(
            !draft(&asked(PutIn::EmailDraft, None), &meeting, "xy", fits)
                .unwrap()
                .whole
        );
        // The subject without a title: the hook's name, or the days.
        assert_eq!(subject("To-dos", &sample(HookEvent::PhoneNote)), "To-dos");
        assert_eq!(
            subject("Brief", &sample(HookEvent::BeforeMeeting)),
            "Brief: Pricing review"
        );
        assert!(
            subject("Weekly review", &sample(HookEvent::Schedule)).starts_with("Weekly review: 20")
        );
        assert_eq!(
            subject("Notes", &named).chars().count(),
            "Notes: Sync?bcc=eve@example.com&to=eve@example.com To: eve"
                .chars()
                .count()
        );
        // Headings, bold and to-do boxes are for notes, not for mail.
        assert_eq!(
            letter(
                "## Decided\n\n**Dana:** goes live on __Friday__  \n- [ ] #1 priority\n  - [ ] and a [ ] box\n#hashtag\n"
            ),
            "Decided\n\nDana: goes live on Friday\n- #1 priority\n  - and a [ ] box\n#hashtag"
        );
    }

    #[test]
    fn a_hook_says_what_it_does_with_reminders_and_drafts() {
        let hook = |put, to: Option<&str>| Hook {
            when: HookEvent::Meeting,
            run: HookTarget::Ask {
                name: "My commitments".into(),
                instruction: "List what I said I'd do.".into(),
                put,
                to: to.map(str::to_string),
            },
            preset: None,
            title_has: None,
            on: true,
            trusted: false,
            notify: false,
            schedule: None,
            before: None,
        };
        assert_eq!(
            describe(&hook(PutIn::Reminders, None)),
            "Gives the text to the AI with “List what I said I'd do.” and adds a reminder to your Reminders for each to-do it writes"
        );
        assert_eq!(
            describe(&hook(PutIn::Reminders, Some(" Work "))),
            "Gives the text to the AI with “List what I said I'd do.” and adds a reminder to your Reminders list Work for each to-do it writes"
        );
        assert_eq!(
            describe(&hook(
                PutIn::EmailDraft,
                Some("dana@example.com, sam@example.com")
            )),
            "Gives the text to the AI with “List what I said I'd do.” and opens what it writes as an email draft to dana@example.com, sam@example.com, which is not sent"
        );
        for no_one in [None, Some(" "), Some("not an address")] {
            assert_eq!(
                describe(&hook(PutIn::EmailDraft, no_one)),
                "Gives the text to the AI with “List what I said I'd do.” and opens what it writes as an email draft to no one, which is not sent"
            );
        }
        // Test says what would be done, and does none of it.
        let today = Local::now().date_naive();
        let meeting = sample(HookEvent::Meeting);
        let wrote = format!(
            "- [ ] Send Dana the numbers by {today}\n- [ ] Write to support\n- Decided: Friday"
        );
        assert_eq!(
            would(&asked(PutIn::Reminders, None), &meeting, &wrote).unwrap(),
            format!(
                "This would add 2 reminders:\n• Send Dana the numbers by {today} (due {today})\n• Write to support"
            )
        );
        assert_eq!(
            would(&asked(PutIn::Reminders, None), &meeting, "Hi Dana,").unwrap(),
            "None of these lines is a to-do, so no reminder would be added."
        );
        assert_eq!(
            would(
                &asked(PutIn::EmailDraft, Some("dana@example.com")),
                &meeting,
                "Hi Dana,"
            )
            .unwrap(),
            "This would open as an email draft to dana@example.com, with the subject “Follow-up email: Pricing review”."
        );
        let long = "A long line of the email. ".repeat(1_000);
        assert!(
            would(&asked(PutIn::EmailDraft, None), &meeting, &long)
                .unwrap()
                .ends_with("to no one, with the subject “Follow-up email: Pricing review”. It's too long to open in a draft, so it would go on the clipboard to paste in.")
        );
        assert!(would(&asked(PutIn::Clipboard, None), &meeting, "Hi").is_none());
        let many = "- [ ] One more thing\n".repeat(TODOS_MOST + 5);
        assert!(
            would(&asked(PutIn::Reminders, None), &meeting, &many)
                .unwrap()
                .starts_with("This would add 25 reminders (the first of 30 to-dos):\n")
        );
    }
}
