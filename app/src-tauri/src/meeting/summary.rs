//! The summary at the top of a meeting note: which AI writes it, asking
//! first when it's a cloud one, and writing it.

use super::*;

/// The AI that runs transforms (it writes the summary): a name for the
/// question, and whether it's on this computer. `None` without one.
pub(super) fn summary_ai(settings: &Settings) -> Option<(String, bool)> {
    let secrets = Secrets::load();
    let on_device = crate::engine::on_device(settings);
    let choice = settings
        .resolve_instruction_ai_with(&secrets, on_device)
        .or_else(|| settings.resolve_ai_with(&secrets, on_device).ok().flatten())?;
    let local = choice.destination() == Destination::Local;
    let name = match choice.provider {
        splashdown_config::AiProvider::Openai => "OpenAI",
        splashdown_config::AiProvider::Openrouter => "OpenRouter",
        splashdown_config::AiProvider::Anthropic => "Anthropic",
        _ if local => "the AI on this computer",
        _ => "your AI server",
    };
    Some((name.to_string(), local))
}

/// Summarizes a meeting (with the Meeting summary transform) or an app's
/// audio (`one_sided`, with Summary).
pub(super) fn summarize(
    app: &AppHandle,
    store: &Store,
    note: &str,
    settings: &Settings,
    one_sided: bool,
) {
    let Some((ai, local)) = summary_ai(settings) else {
        return;
    };
    // Nobody said anything: nothing to summarize, or to ask about.
    let Ok(body) = store.read(note).map(|n| n.body) else {
        return;
    };
    if crate::notes::store::transcript_start(&body).is_none() {
        return;
    }
    let run = match settings.notes.meeting_summary {
        Some(MeetingSummary::Always) => true,
        Some(MeetingSummary::Never) => false,
        None if local => true,
        Some(MeetingSummary::Ask) | None => ask_to_summarize(app, &ai, one_sided),
    };
    if !run {
        return;
    }
    // Re-read: My notes may have changed while the question was up.
    let Ok(body) = store.read(note).map(|n| n.body) else {
        return;
    };
    let Some(at) = crate::notes::store::transcript_start(&body) else {
        return;
    };
    let transcript = body[at..].trim();
    // What the user wrote down shows what mattered to them.
    let input = match crate::notes::store::section(&body[..at], MY_NOTES) {
        Some(mine) if !mine.is_empty() => {
            format!("My notes, written during the meeting:\n{mine}\n\nTranscript:\n{transcript}")
        }
        _ => transcript.to_string(),
    };
    let state = app.state::<AppState>();
    let transform = if one_sided {
        "summary"
    } else {
        "meeting-summary"
    };
    match state.session.transform(transform.into(), input) {
        Ok(out) if out.unchanged.is_none() => {
            if let Err(e) = store.insert_summary(note, &out.text) {
                tracing::warn!("couldn't add the summary: {e}");
            }
        }
        Ok(out) => tracing::info!("no meeting summary: {}", out.unchanged.unwrap_or_default()),
        Err(e) => tell(app, format!("The meeting summary didn't work: {e}"), true),
    }
}

/// Asks before the meeting's text goes to a cloud AI. "Always" is kept.
pub(super) fn ask_to_summarize(app: &AppHandle, ai: &str, one_sided: bool) -> bool {
    let what = if one_sided { "recording" } else { "meeting" };
    let title = format!("Summarize this {what} with {ai}?");
    let text = if one_sided {
        format!("The transcript goes to {ai} to write a summary of its main points.")
    } else {
        format!(
            "The transcript goes to {ai} to write a summary of decisions, action items and open questions."
        )
    };
    let buttons = ["Summarize", "Always summarize", "Not now"];
    asking(
        app,
        Some(crate::prompt::Prompt {
            title: title.clone(),
            text: text.clone(),
            buttons: buttons.map(String::from).to_vec(),
        }),
    );
    let answer = crate::prompt::ask(app, &title, &text, &buttons);
    asking(app, None);
    if answer == Some(1) {
        let state = app.state::<AppState>();
        if let Err(e) = state.update(app, |s| {
            s.notes.meeting_summary = Some(MeetingSummary::Always);
        }) {
            tracing::warn!("couldn't save always summarizing: {e}");
        }
        return true;
    }
    answer == Some(0)
}

/// Shows the meeting waiting on `question` (or no longer), in Notes.
pub(super) fn asking(app: &AppHandle, question: Option<crate::prompt::Prompt>) {
    let status = {
        let mut slot = running();
        let Some(run) = slot.as_mut() else {
            return;
        };
        run.status.question = question;
        run.status.clone()
    };
    show(app, Some(&status));
}
