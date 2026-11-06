//! Remembering people by voice (Settings → Meetings → "Recognize people
//! you've named"). After a meeting's speakers are told apart, each one's
//! voiceprint is made and checked against the people the user named
//! before (`voices.json`); a clear match gets the name. The meeting's
//! voiceprints are kept (`speakers/<note id>.json`) so that naming a
//! speaker in Notes later saves their voice under that name.
//!
//! Voiceprints are other people's biometric data: they stay in the app's
//! data folder, never in notes, never sent, and Forget removes them.

use std::collections::BTreeMap;

use splashdown_core::voices::{self, Voice};
use splashdown_providers::SpeakerTurn;
use splashdown_providers::models::VoiceModel;

use super::*;

/// Turns shorter than this are left out of a voiceprint (a "yeah" says
/// little about a voice), in seconds.
const SHORTEST_TURN: f64 = 1.5;
/// Speech used for a voiceprint, at most, in seconds.
const MOST_SPEECH: f64 = 120.0;
const RATE: f64 = 16_000.0;

/// A meeting's voiceprints, by the label the transcript gives them now.
type Prints = BTreeMap<String, Vec<f32>>;

fn data_dir() -> Result<PathBuf, String> {
    Ok(crate::engine::paths()?.config_dir)
}

fn voices_file() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("voices.json"))
}

fn meeting_file(id: &str) -> Result<PathBuf, String> {
    // The id is read from a note the user can edit: nothing that leaves the
    // folder.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("This note's id isn't one Splashdown wrote.".into());
    }
    Ok(data_dir()?.join("speakers").join(format!("{id}.json")))
}

fn read<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// One change at a time to the saved voices (a meeting ending while a
/// speaker is renamed).
fn voices_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// After a meeting's speakers were labelled Speaker 1, 2… (`order`: the
/// diarization's speakers in that order): voiceprints for those who said
/// enough, the saved people among them named, and the prints kept.
pub(super) fn recognize(
    store: &Store,
    note: &str,
    pcm: &[f32],
    turns: &[SpeakerTurn],
    order: &[u32],
) {
    let Ok(models) = crate::engine::paths().map(|p| p.models_dir) else {
        return;
    };
    let model = VoiceModel::path_in(&models);
    if !model.is_file() {
        return;
    }
    let Some(id) = store.id(note) else {
        return;
    };
    let started = Instant::now();
    let printer = match splashdown_providers::Voiceprinter::load(&model) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("{e}");
            return;
        }
    };
    let speakers: Vec<(String, Vec<f32>)> = order
        .iter()
        .enumerate()
        .filter_map(|(i, &speaker)| {
            let speech = speech_of(pcm, turns, speaker)?;
            Some((format!("Speaker {}", i + 1), printer.print(&speech)?))
        })
        .collect();
    tracing::info!(
        "{} of {} speakers' voices measured in {:.1} s",
        speakers.len(),
        order.len(),
        started.elapsed().as_secs_f32()
    );
    if speakers.is_empty() {
        return;
    }
    let _lock = voices_lock();
    let Ok(voices_path) = voices_file() else {
        return;
    };
    let mut saved: Vec<Voice> = read(&voices_path);
    let invited = store
        .read(note)
        .map(|n| invited(&n.body))
        .unwrap_or_default();
    let found: Vec<(String, String)> = voices::recognize(&speakers, &saved, &invited)
        .into_iter()
        .map(|(label, name)| (label.to_string(), name))
        .collect();
    let mut prints: Prints = speakers.into_iter().collect();
    if !found.is_empty() {
        let renamed = store.rewrite(note, |body| {
            found.iter().fold(body.to_string(), |body, (label, name)| {
                rename(&body, label, name)
            })
        });
        match renamed {
            Ok(()) => {
                for (label, name) in &found {
                    if let Some(print) = prints.remove(label) {
                        prints.insert(name.clone(), print);
                    }
                    if let Some(voice) = saved.iter_mut().find(|v| &v.name == name) {
                        voice.meetings += 1;
                    }
                }
                tracing::info!("{} speaker(s) recognized by voice", found.len());
                if let Err(e) = write(&voices_path, &saved) {
                    tracing::warn!("couldn't save the voices: {e}");
                }
            }
            Err(e) => tracing::warn!("couldn't name the recognized speakers: {e}"),
        }
    }
    let kept = meeting_file(&id).and_then(|path| write(&path, &prints));
    if let Err(e) = kept {
        tracing::warn!("couldn't keep the meeting's voiceprints: {e}");
    }
}

/// `speaker`'s speech in `pcm`: their turns of some length that nobody
/// else spoke over, up to `MOST_SPEECH`; `None` with too little to go on.
fn speech_of(pcm: &[f32], turns: &[SpeakerTurn], speaker: u32) -> Option<Vec<f32>> {
    let mut speech = Vec::new();
    for turn in turns.iter().filter(|t| t.speaker == speaker) {
        let (start, end) = (turn.start.as_secs_f64(), turn.end.as_secs_f64());
        let overlapped = turns.iter().any(|o| {
            o.speaker != speaker && o.start.as_secs_f64() < end && o.end.as_secs_f64() > start
        });
        if end - start < SHORTEST_TURN || overlapped {
            continue;
        }
        let from = ((start * RATE) as usize).min(pcm.len());
        let to = ((end * RATE) as usize).min(pcm.len());
        speech.extend_from_slice(&pcm[from..to]);
        if speech.len() as f64 >= MOST_SPEECH * RATE {
            break;
        }
    }
    (speech.len() as f64 >= f64::from(voices::LEAST_SPEECH_SECS) * RATE).then_some(speech)
}

/// The people a meeting's note says were invited: its "With Dana, Sam."
/// line (from the calendar).
fn invited(body: &str) -> Vec<String> {
    body.lines()
        .find_map(|l| l.trim().strip_prefix("With ")?.strip_suffix('.'))
        .map(|names| {
            names
                .split(", ")
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `body` with `from`'s turns said by `to`, and "(from)" after an action
/// item too.
fn rename(body: &str, from: &str, to: &str) -> String {
    body.split('\n')
        .map(|line| match Turn::parse(line) {
            Some(turn) if turn.who == from => Turn { who: to, ..turn }.line(),
            _ => line.replace(&format!("({from})"), &format!("({to})")),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Notes: speaker `who` of meeting `path` was named `name`. Their voice
/// from that meeting is saved under the name (if Recognize is on and it
/// was measured), and the meeting's print now goes by it.
#[tauri::command]
pub fn notes_speaker_named(
    path: String,
    who: String,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let settings = state.settings();
    if !settings.notes.remember_speakers {
        return Ok(());
    }
    let (store, _) = crate::notes::store_for(settings.notes.folder.clone())?;
    let Some(id) = store.id(&path) else {
        return Ok(());
    };
    let _lock = voices_lock();
    let meeting = meeting_file(&id)?;
    let mut prints: Prints = read(&meeting);
    let Some(print) = prints.remove(&who) else {
        return Ok(());
    };
    let voices_path = voices_file()?;
    let mut saved: Vec<Voice> = read(&voices_path);
    voices::learn(&mut saved, &name, print.clone());
    prints.insert(name.clone(), print);
    write(&voices_path, &saved)?;
    write(&meeting, &prints)?;
    tracing::info!("a voice saved under a name");
    Ok(())
}

/// A saved person, for Settings.
#[derive(Debug, Serialize)]
pub struct SavedVoice {
    name: String,
    meetings: u32,
}

/// Settings: the people saved by voice.
#[tauri::command]
pub fn voices_list() -> Vec<SavedVoice> {
    let Ok(path) = voices_file() else {
        return Vec::new();
    };
    let mut saved: Vec<Voice> = read(&path);
    saved.sort_by_key(|v| v.name.to_lowercase());
    saved
        .into_iter()
        .map(|v| SavedVoice {
            name: v.name,
            meetings: v.meetings,
        })
        .collect()
}

/// Settings: forgets one person's voice, or everyone's (`None`), with every
/// meeting's voiceprints.
#[tauri::command]
pub fn voices_forget(name: Option<String>) -> Result<(), String> {
    let _lock = voices_lock();
    let path = voices_file()?;
    match name {
        Some(name) => {
            let mut saved: Vec<Voice> = read(&path);
            saved.retain(|v| v.name != name);
            write(&path, &saved)
        }
        None => {
            let _ = std::fs::remove_file(&path);
            let speakers = data_dir()?.join("speakers");
            match std::fs::remove_dir_all(&speakers) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_speaker_is_renamed_in_turns_and_action_items() {
        let body = "With Dana, Sam.\n\n## Summary\n- Send the deck (Speaker 2)\n\n## Transcript\n\n**[0:01] Speaker 2:** Hello.\n\n**[0:04] Speaker 1:** Hi.\n\n**[0:09] Speaker 2:** Speaker 2 here.";
        let out = rename(body, "Speaker 2", "Dana");
        assert!(out.contains("- Send the deck (Dana)"));
        assert!(out.contains("**[0:01] Dana:** Hello."));
        assert!(out.contains("**[0:04] Speaker 1:** Hi."));
        // What was said is left alone.
        assert!(out.contains("**[0:09] Dana:** Speaker 2 here."));
        assert_eq!(invited(body), ["Dana", "Sam"]);
    }

    #[test]
    fn a_voiceprint_takes_clean_turns_of_some_length() {
        let s = |speaker, start: f64, end: f64| SpeakerTurn {
            speaker,
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
        };
        let pcm = vec![0.1; (RATE * 60.0) as usize];
        // 25 s of clean speech from 1; a short "yeah" and an overlap don't count.
        let turns = [
            s(1, 0.0, 25.0),
            s(1, 30.0, 31.0),
            s(1, 40.0, 50.0),
            s(2, 45.0, 55.0),
        ];
        assert_eq!(
            speech_of(&pcm, &turns, 1).unwrap().len(),
            (25.0 * RATE) as usize
        );
        // Speaker 2 only spoke over 1: too little.
        assert!(speech_of(&pcm, &turns, 2).is_none());
    }
}
