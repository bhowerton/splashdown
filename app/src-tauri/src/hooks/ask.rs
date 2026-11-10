//! A hook that asks the AI: the user's own instruction ("draft a follow-up
//! email to the people in this meeting") run on the text of what happened,
//! and the result put where they chose: a section of the note, another
//! note, a file, the clipboard or a web address.
//!
//! The AI only writes. It has no tools, so the text it reads (a meeting is
//! mostly other people's words) can at worst change what it writes, never
//! where that goes or what else happens. The text goes to the AI that
//! follows instructions (Settings → AI Models): the one on this computer,
//! or a cloud one, as Settings says where the hook is added. A text too
//! long for the model is read in parts (`splashdown_core::instruct`).

use std::path::Path;

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

/// Asks, and puts the answer where it goes: what was done, in a few words.
pub(super) fn run(
    app: &AppHandle,
    asked: &Asked<'_>,
    happened: &Happened,
) -> Result<String, String> {
    let Some(text) = write(app, asked.instruction, happened)? else {
        return Ok("Nothing to do".into());
    };
    put(app, asked, happened, &text)
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
        what: what(happened.event).to_string(),
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
fn what(event: &str) -> &'static str {
    match event {
        "meeting" => {
            "the note of a meeting they were in, with its transcript (other people are named, or \"Them\")"
        }
        "transcript" => "the transcript of a recording",
        "phone-note" => "a note they said into their phone",
        _ => "something they dictated",
    }
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
                .add_under(&rel, &title, heading(happened).as_deref(), text)
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
fn other_note(to: &str) -> (String, String) {
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
/// the meeting's or transcript's title and the day. None for a dictation
/// or a note from the phone, whose results are added as they are.
fn heading(happened: &Happened) -> Option<String> {
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
}
