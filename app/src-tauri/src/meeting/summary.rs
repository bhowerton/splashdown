//! The summary at the top of a meeting note: which AI writes it, asking
//! first when it's a cloud one, and writing it.

use super::*;

/// The AI that runs transforms (it writes the summary): a name for the
/// question, and whether it's on this computer. `None` without one.
pub(crate) fn summary_ai(settings: &Settings) -> Option<(String, bool)> {
    let secrets = Secrets::load();
    let on_device = crate::engine::on_device(settings);
    let choice = settings
        .resolve_instruction_ai_with(&secrets, on_device)
        .or_else(|| settings.resolve_ai_with(&secrets, on_device).ok().flatten())?;
    let local = choice.destination() == Destination::Local;
    let name = match choice.provider {
        p @ (splashdown_config::AiProvider::Openai
        | splashdown_config::AiProvider::Openrouter
        | splashdown_config::AiProvider::Anthropic) => p.label(),
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
    if crate::notes::markdown::transcript_start(&body).is_none() {
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
    let Some(at) = crate::notes::markdown::transcript_start(&body) else {
        return;
    };
    let transcript = body[at..].trim();
    // What the user wrote down shows what mattered to them.
    let input = match crate::notes::markdown::section(&body[..at], MY_NOTES) {
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
    let input = match readable(&state, settings, transform, input) {
        Ok(input) => input,
        Err(e) => return tell(app, format!("The meeting summary didn't work: {e}"), true),
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

/// Tokens of the AI's context kept for the summary's own prompt and for
/// the summary: what's left is for the meeting.
const SUMMARY_ROOM: usize = 2000;

/// `input` as the AI that writes the summary can read it: as it is, or,
/// when it's longer than that AI reads at once (the one on this computer
/// reads about fifteen minutes of talk), notes taken from each part of it
/// for the summary. That takes a request per part, so a long meeting's
/// summary takes minutes rather than seconds there.
fn readable(
    state: &AppState,
    settings: &Settings,
    transform: &str,
    input: String,
) -> Result<String, String> {
    use splashdown_core::instruct;
    let Ok(Some(setup)) = state.session.answer_model() else {
        // No AI: the transform says so.
        return Ok(input);
    };
    let model = setup.model;
    let fits = model.context_window().saturating_sub(SUMMARY_ROOM) * 3 / 4;
    if splashdown_core::prompt::estimated_tokens(&input) <= fits {
        return Ok(input);
    }
    let Some(instruction) = settings
        .transforms()
        .into_iter()
        .chain(splashdown_config::builtin_transforms())
        .find(|t| t.id == transform)
        .map(|t| t.instruction)
    else {
        return Ok(input);
    };
    let about = instruct::About {
        what: "the transcript of a meeting they were in".into(),
        title: None,
        user_name: setup.user_name,
        today: splashdown_core::prompt::today(chrono::Local::now().date_naive()),
    };
    let result = tauri::async_runtime::block_on(instruct::condense(
        model.as_ref(),
        &instruction,
        &input,
        &about,
        fits,
    ));
    let usage = match &result {
        Ok(done) => done.usage,
        Err(failed) => failed.usage,
    };
    crate::spend::shared().record(&[(model.name(), usage)], chrono::Local::now());
    match result {
        Ok(done) if done.text.trim().is_empty() => Err("there was nothing to summarize".into()),
        Ok(done) => {
            tracing::info!(
                "a long meeting was read in {} parts for its summary",
                done.parts
            );
            Ok(format!(
                "The meeting was too long to read at once, so these are notes taken from each part of it, in order:\n\n{}",
                done.text
            ))
        }
        Err(failed) => Err(crate::errors::pipeline(
            &splashdown_core::PipelineError::Llm(failed.error),
            None,
            Some(&model.name()),
        )),
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
            choices: Vec::new(),
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
