//! Telling the user that a hook has written something: a small panel
//! with the hook's name, where what it wrote went, how it starts, and a
//! way to open it. For a hook with "Tell me when it's done" on, and for
//! one run by hand. A brief written ten minutes before a meeting is no
//! use unseen in a note.

use super::*;

/// How long the panel stays when nobody answers it.
const SHOWN_FOR: Duration = Duration::from_secs(5 * 60);
/// How much of what was written the panel holds: it shows a few lines and
/// scrolls.
const MOST: usize = 600;

/// Shows that `hook` wrote `wrote` for what happened, in the background.
pub(super) fn tell(app: &AppHandle, hook: &Hook, happened: &Happened, said: &str, wrote: &str) {
    let note = ask::Asked::of(hook).and_then(|asked| ask::note_of(&asked, happened));
    let title = title(&name(hook), about(happened).as_deref());
    let text = format!("{said}.\n\n{}", start_of(wrote, MOST));
    let app = app.clone();
    let started = std::thread::Builder::new()
        .name("hook-notice".into())
        .spawn(move || {
            let buttons: &[&str] = if note.is_some() {
                &["Open", "OK"]
            } else {
                &["OK"]
            };
            let answer = crate::prompt::tell(&app, &title, &text, buttons, SHOWN_FOR);
            if let (Some(0), Some(rel)) = (answer, note) {
                let opener = app.clone();
                let _ = app.run_on_main_thread(move || {
                    crate::notes::open_to(&opener, Some(&format!("note:{rel}")));
                });
            }
        });
    if let Err(e) = started {
        tracing::warn!("couldn't show what a hook wrote: {e}");
    }
}

/// What a hook's run was about: the note, the meeting, or the days.
pub(super) fn about(happened: &Happened) -> Option<String> {
    happened
        .note
        .as_ref()
        .map(|n| n.title.clone())
        .or_else(|| happened.meeting.as_ref().map(|m| m.title.clone()))
        .or_else(|| happened.app.clone())
        .or_else(|| happened.period())
}

fn title(hook: &str, about: Option<&str>) -> String {
    match about.map(str::trim).filter(|a| !a.is_empty()) {
        Some(about) => format!("{hook} · {about}"),
        None => hook.to_string(),
    }
}

/// The start of what was written, as plain lines: Markdown's marks would
/// only be noise in a panel that shows text as it is.
fn start_of(wrote: &str, most: usize) -> String {
    let mut out = String::new();
    for line in wrote.lines() {
        let line = line.trim();
        let line = line.trim_start_matches('#').trim_start();
        let line = match line
            .strip_prefix("- [ ] ")
            .or_else(|| line.strip_prefix("- [x] "))
        {
            Some(todo) => format!("☐ {todo}"),
            None => match line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
                Some(item) => format!("• {item}"),
                None => line.to_string(),
            },
        };
        let line = line.replace("**", "");
        if line.is_empty() && (out.is_empty() || out.ends_with("\n\n")) {
            continue;
        }
        if out.chars().count() + line.chars().count() > most {
            let room = most.saturating_sub(out.chars().count());
            out.extend(line.chars().take(room));
            out.push('…');
            return out;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_shows_how_what_was_written_starts() {
        let wrote = "**Decided**\n- The pricing page goes live on Friday (Pricing review)\n\n\n## For me to do\n- [ ] Send Dana the final numbers by Wednesday\n";
        assert_eq!(
            start_of(wrote, 600),
            "Decided\n• The pricing page goes live on Friday (Pricing review)\n\nFor me to do\n☐ Send Dana the final numbers by Wednesday"
        );
        let cut = start_of(wrote, 30);
        assert!(cut.starts_with("Decided\n• The pricing page") && cut.ends_with('…'));
        assert!(cut.chars().count() <= 31);
        assert_eq!(title("Brief", Some("Weekly sync")), "Brief · Weekly sync");
        assert_eq!(title("Brief", Some(" ")), "Brief");
        assert_eq!(title("Brief", None), "Brief");
    }
}
