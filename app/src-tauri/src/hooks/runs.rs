//! What the hooks did lately (Settings → Actions → When something happens
//! → What ran): when, which hook, about what, and how it went. Never the
//! text itself: that's wherever the hook put it.

use serde::Deserialize;
use tauri::Emitter;

use super::*;

/// How many runs are kept.
const KEPT: usize = 50;
const FILE: &str = "hook-runs.json";
/// Sent to Settings when a hook has run, to show it.
const RAN: &str = "hooks-ran";

static SAVING: Mutex<()> = Mutex::new(());

/// One run of one hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// When it finished, as `2026-11-09T14:03:22-05:00`.
    pub at: String,
    /// The event, as hooks name it: `meeting`, `dictation`…
    pub event: String,
    /// The hook, as the list names it.
    pub hook: String,
    /// The meeting's or transcript's title, or the app dictated into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    pub ok: bool,
    /// What it did ("Added to the note"), or what went wrong.
    pub said: String,
    /// How long it took, in seconds.
    pub took: f32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Runs {
    #[serde(default)]
    runs: Vec<Run>,
}

/// `runs` with `run` added, the oldest dropped past `KEPT`.
fn with(mut runs: Vec<Run>, run: Run) -> Vec<Run> {
    runs.push(run);
    let extra = runs.len().saturating_sub(KEPT);
    runs.drain(..extra);
    runs
}

/// Keeps `run`, and tells Settings.
pub(super) fn record(app: &AppHandle, run: Run) {
    let saved = crate::data::path(FILE).and_then(|path| {
        let _saving = SAVING.locked();
        let kept: Runs = crate::data::read(&path);
        crate::data::write(
            &path,
            &Runs {
                runs: with(kept.runs, run),
            },
        )
    });
    if let Err(e) = saved {
        tracing::warn!("couldn't keep what a hook did: {e}");
    }
    let _ = app.emit(RAN, ());
}

/// What the hooks did lately, newest first.
#[tauri::command]
pub fn hooks_runs() -> Vec<Run> {
    let Ok(path) = crate::data::path(FILE) else {
        return Vec::new();
    };
    let mut kept: Runs = crate::data::read(&path);
    kept.runs.reverse();
    kept.runs
}

/// Forgets what the hooks did.
#[tauri::command]
pub fn hooks_runs_clear() -> Result<(), String> {
    let path = crate::data::path(FILE)?;
    let _saving = SAVING.locked();
    crate::data::write(&path, &Runs::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_latest_runs_are_kept() {
        let run = |n: usize| Run {
            at: format!("2026-11-09T14:{:02}:00-05:00", n % 60),
            event: "meeting".into(),
            hook: format!("hook {n}"),
            about: Some("Weekly sync".into()),
            ok: true,
            said: "Added to the note".into(),
            took: 1.5,
        };
        let mut runs = Vec::new();
        for n in 0..KEPT + 5 {
            runs = with(runs, run(n));
        }
        assert_eq!(runs.len(), KEPT);
        assert_eq!(runs[0].hook, "hook 5");
        assert_eq!(runs[KEPT - 1].hook, format!("hook {}", KEPT + 4));
        // As the file has them, and back.
        let json = serde_json::to_string(&Runs { runs: runs.clone() }).unwrap();
        assert!(!json.contains("text"));
        assert_eq!(serde_json::from_str::<Runs>(&json).unwrap().runs, runs);
    }
}
