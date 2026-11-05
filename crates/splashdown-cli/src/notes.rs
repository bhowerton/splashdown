//! `splashdown-cli notes list|search|read`: the notes tools coding agents
//! get through `splashdown-mcp`, from a terminal or a script. They go to
//! the running app, so the same Settings → Privacy switches apply (agents
//! run commands too).

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::Value as Json;
use splashdown_control::{NoteFilter, ReadPart, Request};

#[derive(Subcommand)]
pub enum NotesCommand {
    /// Notes newest first, narrowed by date, kind, folder or tag:
    /// `notes list --when this_week --kind meeting --summary`.
    List {
        #[command(flatten)]
        filter: FilterArgs,
        /// Each meeting's summary too (at most 25 notes then).
        #[arg(long)]
        summary: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// From a previous page's next cursor.
        #[arg(long)]
        cursor: Option<String>,
        /// Print the app's JSON as it is.
        #[arg(long)]
        json: bool,
    },
    /// Notes containing these words (titles and text, transcripts
    /// included), title matches first, then newest.
    Search {
        query: Vec<String>,
        #[command(flatten)]
        filter: FilterArgs,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Print a note, or one part of a meeting or transcript, by its path
    /// in the notes folder: `notes read "Meetings/Weekly sync.md"
    /// --section transcript --speaker Dana`.
    Read {
        path: String,
        /// all, summary, my_notes or transcript.
        #[arg(long)]
        section: Option<String>,
        /// Transcript only: from this time (1:05, 1:02:30 or seconds).
        #[arg(long)]
        start: Option<String>,
        /// Transcript only: up to this time.
        #[arg(long)]
        end: Option<String>,
        /// Transcript only: what this speaker said (You is you).
        #[arg(long)]
        speaker: Option<String>,
        /// One page of at most this many characters, instead of all of it.
        #[arg(long)]
        max_chars: Option<usize>,
        /// One page, from a previous page's next cursor.
        #[arg(long)]
        cursor: Option<String>,
        /// Print the app's JSON (one page) as it is.
        #[arg(long)]
        json: bool,
    },
}

/// The filters `list` and `search` share.
#[derive(Args)]
pub struct FilterArgs {
    /// today, yesterday, this_week, last_week, this_month, last_month,
    /// last_7_days or last_30_days.
    #[arg(long)]
    when: Option<String>,
    /// From a date (2026-11-02), a time, today, yesterday or a while ago
    /// (7d, 36h).
    #[arg(long)]
    since: Option<String>,
    /// Before a date or time, in the same forms.
    #[arg(long)]
    until: Option<String>,
    /// meeting, transcript, capture or note.
    #[arg(long)]
    kind: Option<String>,
    /// A folder, with its subfolders.
    #[arg(long)]
    folder: Option<String>,
    /// A tag, with the tags nested under it.
    #[arg(long)]
    tag: Option<String>,
}

impl From<&FilterArgs> for NoteFilter {
    fn from(f: &FilterArgs) -> Self {
        NoteFilter {
            when: f.when.clone(),
            since: f.since.clone(),
            until: f.until.clone(),
            kind: f.kind.clone(),
            folder: f.folder.clone(),
            tag: f.tag.clone(),
        }
    }
}

fn ask(request: &Request) -> Result<Json> {
    let text = splashdown_control::send(request)?.unwrap_or_default();
    serde_json::from_str(&text).context("the app's answer wasn't JSON")
}

pub fn run(command: &NotesCommand) -> Result<()> {
    match command {
        NotesCommand::List {
            filter,
            summary,
            limit,
            cursor,
            json,
        } => {
            let answer = ask(&Request::ListNotes {
                filter: filter.into(),
                with_summary: *summary,
                limit: Some(*limit),
                cursor: cursor.clone(),
            })?;
            if *json {
                println!("{answer}");
                return Ok(());
            }
            let notes = answer["notes"].as_array().cloned().unwrap_or_default();
            for note in &notes {
                println!("{}", line(note));
                if let Some(summary) = note["summary"].as_str() {
                    for l in summary.lines() {
                        if l.trim().is_empty() {
                            println!();
                        } else {
                            println!("    {l}");
                        }
                    }
                    println!();
                }
            }
            if notes.is_empty() {
                eprintln!("{}", answer["hint"].as_str().unwrap_or("No notes match."));
            }
            if let Some(next) = answer["next_cursor"].as_str() {
                eprintln!(
                    "{} of {} shown. More: --cursor {next}",
                    notes.len(),
                    answer["total"]
                );
            }
        }
        NotesCommand::Search {
            query,
            filter,
            limit,
            json,
        } => {
            let answer = ask(&Request::SearchNotes {
                query: query.join(" "),
                limit: Some(*limit),
                filter: filter.into(),
            })?;
            if *json {
                println!("{answer}");
                return Ok(());
            }
            let hits = answer.as_array().cloned().unwrap_or_default();
            if hits.is_empty() {
                eprintln!("Nothing matches.");
            }
            for hit in &hits {
                println!("{}", line(hit));
                if let Some(said) = hit["said"].as_array() {
                    let at: Vec<String> = said
                        .iter()
                        .map(|s| match s["speaker"].as_str() {
                            Some(who) => format!("{who} at {}", text(&s["at"])),
                            None => format!("at {}", text(&s["at"])),
                        })
                        .collect();
                    println!("    said by {}", at.join(", "));
                }
                println!("    {}", text(&hit["excerpt"]));
            }
        }
        NotesCommand::Read {
            path,
            section,
            start,
            end,
            speaker,
            max_chars,
            cursor,
            json,
        } => {
            let mut part = ReadPart {
                section: section.clone(),
                start: start.clone(),
                end: end.clone(),
                speaker: speaker.clone(),
                max_chars: *max_chars,
                cursor: cursor.clone(),
            };
            // One page when asked for one; otherwise all of it.
            let one_page = *json || max_chars.is_some() || cursor.is_some();
            loop {
                let answer = ask(&Request::ReadNote {
                    path: path.clone(),
                    part: part.clone(),
                })?;
                if *json {
                    println!("{answer}");
                    return Ok(());
                }
                if let Some(missing) = answer["missing"].as_str() {
                    eprintln!("{missing}");
                }
                print!("{}", text(&answer["text"]));
                match answer["next_cursor"].as_str() {
                    Some(next) if !one_page => part.cursor = Some(next.to_string()),
                    Some(next) => {
                        println!();
                        eprintln!("More: --cursor {next}");
                        break;
                    }
                    None => break,
                }
            }
            println!();
        }
    }
    Ok(())
}

fn text(v: &Json) -> &str {
    v.as_str().unwrap_or_default()
}

/// `2026-11-02 10:00  meeting  Planning offsite · Nov 2  2:00:04, You, Dana Lee  Meetings/Planning offsite.md`
fn line(note: &Json) -> String {
    let when = note["created"]
        .as_str()
        .or(note["modified"].as_str())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "                ".into());
    let mut extra: Vec<String> = Vec::new();
    if let Some(d) = note["duration"].as_str() {
        extra.push(d.to_string());
    }
    if let Some(who) = note["speakers"].as_array().filter(|w| !w.is_empty()) {
        extra.push(
            who.iter()
                .filter_map(Json::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if let Some(tags) = note["tags"].as_array().filter(|t| !t.is_empty()) {
        extra.push(
            tags.iter()
                .filter_map(Json::as_str)
                .map(|t| format!("#{t}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!("  {}", extra.join(" · "))
    };
    format!(
        "{when}  {:<10} {}{extra}  ({})",
        text(&note["kind"]),
        text(&note["title"]),
        text(&note["path"])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_show_what_a_note_is() {
        let meeting = serde_json::json!({
            "created": "2026-11-02T10:00:00-05:00",
            "kind": "meeting",
            "title": "Planning offsite",
            "path": "Meetings/Planning offsite.md",
            "duration": "2:00:04",
            "speakers": ["You", "Dana Lee"],
            "tags": ["launch"],
        });
        let line = line(&meeting);
        assert!(line.contains("meeting    Planning offsite  2:00:04 · You, Dana Lee · #launch"));
        assert!(line.ends_with("(Meetings/Planning offsite.md)"));
        let plain = line_of_plain();
        assert!(plain.contains("note       Plan  (Plan.md)"), "{plain}");
    }

    fn line_of_plain() -> String {
        line(
            &serde_json::json!({ "kind": "note", "title": "Plan", "path": "Plan.md", "speakers": [], "tags": [] }),
        )
    }
}
