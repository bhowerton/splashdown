//! Setting up dictation on this computer, for the setup guide: downloads
//! the speech model, then the model that tidies up words, in the
//! background, and switches each on as it arrives. It carries on if the
//! guide is closed, and a second start after a failure carries on where
//! the downloads stopped.
//!
//! The models are the best the computer can run: Nemotron for speech
//! everywhere, Qwen3 4B for tidying up where there's memory for it. With
//! about 8 GB, Apple's built-in model where the Mac has one (no download,
//! no memory of its own, and as good in the eval), otherwise Qwen3 4B
//! packed smaller.
//!
//! After the tidy-up model comes the natural reading voice (small), for
//! "read this to me".
//!
//! Once the tidy-up model is in, it's timed on an ordinary dictation: a
//! slow computer (no graphics chip it can use, an older processor) can
//! take many seconds, and then tidying up is turned off by itself (words
//! go in as they were said, at once) unless the person chose to keep it.
//!
//! Nobody is asked to decide anything about disk space either: with room
//! for speech but not the rest, only speech is set up; with no room even
//! for that, the guide says how much to free.

use splashdown_core::sync::Locked;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use splashdown_config::{AiProvider, Settings, SpeechProvider};
use splashdown_providers::{LocalModel, NaturalVoice, SpeechModel};
use tauri::{AppHandle, Emitter, Manager};

use crate::engine;
use crate::state::AppState;

/// Below this much memory, Qwen3 4B (about 3 GB while in use) would
/// crowd out everything else.
const ROOMY: u64 = 12 << 30;
/// Room to leave on the disk after the downloads.
const SPARE: u64 = 1 << 30;

/// Two ordinary dictations for timing the tidy-up model: the first loads
/// it, the second is timed (a different one, so the model can't reuse
/// what it worked out the first time).
const WARM_UP: &str = "um so I was thinking we could move the team meeting to thursday \
                       afternoon, actually no, friday morning works better for everyone";
const TIMED: &str = "hey can you send me the notes from yesterday's call when you get a \
                     chance, and uh let me know if the budget numbers changed at all since \
                     last week";

/// How the words get tidied up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cleanup {
    /// A model downloaded to this computer.
    Local(LocalModel),
    /// Apple's, built into the Mac.
    Apple,
    /// Not by this setup (a cloud key does it).
    None,
}

/// What `memory` and Apple's model allow for tidying up.
fn cleanup_for(memory: Option<u64>, apple: bool) -> Cleanup {
    if memory.is_none_or(|m| m >= ROOMY) {
        Cleanup::Local(LocalModel::Qwen3_4B)
    } else if apple {
        Cleanup::Apple
    } else {
        Cleanup::Local(LocalModel::Qwen3_4BCompact)
    }
}

fn speech_model() -> SpeechModel {
    SpeechModel::from_id(splashdown_config::defaults::LOCAL_SPEECH).unwrap_or(SpeechModel::Nemotron)
}

/// What setting up would download, for the guide to say before it starts.
#[derive(Serialize)]
pub struct Plan {
    /// "local" (downloaded), "apple" (built into the Mac) or "none".
    cleanup: &'static str,
    /// The smaller tidy-up model, for a computer with less memory.
    small: bool,
    /// Megabytes still to download.
    download_mb: u32,
    /// Of which speech to text, and the AI that cleans up (0 when it's
    /// Apple's, already here, or not part of this setup).
    speech_mb: u32,
    ai_mb: u32,
    /// Of which the natural reading voice (it comes with the tidy-up).
    voice_mb: u32,
    /// Free space where the models go, in MB, if known.
    free_mb: Option<u64>,
    /// Not enough room for the downloads, with some to spare.
    too_little_space: bool,
    /// What there's room for: "all", "speech" (speech to text but not the
    /// rest) or "none".
    room: &'static str,
    /// With no room even for speech, how much to free, in MB.
    free_up_mb: u64,
}

fn plan(speech_only: bool) -> Result<Plan, String> {
    let dir = engine::paths()?.models_dir;
    let settings = engine::load_settings();
    let apple = splashdown_providers::apple::on_device(settings.general.language.as_deref())
        .apple_language_model;
    let cleanup = if speech_only {
        Cleanup::None
    } else {
        cleanup_for(splashdown_platform::memory_bytes(), apple)
    };
    let speech = speech_model();
    let speech_mb = if speech.is_downloaded(&dir) {
        0
    } else {
        speech.approx_mb()
    };
    let ai_mb = match cleanup {
        Cleanup::Local(m) if !m.is_downloaded(&dir) => m.approx_mb(),
        _ => 0,
    };
    let voice_mb = if cleanup == Cleanup::None || NaturalVoice::is_downloaded(&dir) {
        0
    } else {
        NaturalVoice::APPROX_MB
    };
    let download_mb = speech_mb + ai_mb + voice_mb;
    // The folder may not exist yet: ask about the nearest one that does.
    let free = dir
        .ancestors()
        .find(|d| d.exists())
        .and_then(splashdown_platform::free_disk_bytes);
    let (room, free_up_mb) = room_for(free, speech_mb, download_mb);
    Ok(Plan {
        cleanup: match cleanup {
            Cleanup::Local(_) => "local",
            Cleanup::Apple => "apple",
            Cleanup::None => "none",
        },
        small: cleanup == Cleanup::Local(LocalModel::Qwen3_4BCompact),
        download_mb,
        speech_mb,
        ai_mb,
        voice_mb,
        free_mb: free.map(|b| b / 1_000_000),
        too_little_space: room != "all",
        room,
        free_up_mb,
    })
}

/// What `free` bytes leave room for, "all", "speech" or "none", and with
/// none, how many MB to free (always leaving `SPARE`). Unknown free space
/// doesn't stop anything.
fn room_for(free: Option<u64>, speech_mb: u32, all_mb: u32) -> (&'static str, u64) {
    let need = |mb: u32| {
        if mb == 0 {
            0
        } else {
            u64::from(mb) * 1_000_000 + SPARE
        }
    };
    let Some(free) = free else {
        return ("all", 0);
    };
    if free >= need(all_mb) {
        ("all", 0)
    } else if free >= need(speech_mb) {
        ("speech", 0)
    } else {
        ("none", (need(speech_mb) - free).div_ceil(1_000_000))
    }
}

/// How setting up is going.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    /// Still downloading.
    working: bool,
    /// Dictation works: the speech model is here and in use.
    speech_ready: bool,
    /// Tidying up is on (or wasn't part of this setup).
    cleanup_ready: bool,
    /// Bytes downloaded so far, and in all, over both models.
    done: u64,
    total: u64,
    /// Of `total`, speech to text's (downloaded first).
    speech_total: u64,
    /// Timing the tidy-up model, after its download.
    checking: bool,
    /// Only speech was set up, as there wasn't room for the rest.
    no_room_for_ai: bool,
    /// Tidying up was turned off by itself, as it's too slow here.
    slow_off: bool,
    /// The tidy-up model this setup switched on, to switch on again if the
    /// person keeps it although it's slow.
    #[serde(skip)]
    model: Option<LocalModel>,
    /// What that found: milliseconds for an ordinary dictation, and
    /// "fast", "usable" or "slow".
    local_ms: Option<u32>,
    speed: Option<&'static str>,
    /// Why it stopped, in words for anyone.
    error: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);

fn status() -> std::sync::MutexGuard<'static, Option<Status>> {
    STATUS.locked()
}

/// Whether models are being downloaded and set up right now.
pub(crate) fn working() -> bool {
    status().as_ref().is_some_and(|s| s.working)
}

/// What `local_setup_plan` says before anything starts.
#[tauri::command]
pub fn local_setup_plan(speech_only: bool) -> Result<Plan, String> {
    plan(speech_only)
}

/// How setting up is going, if it has started.
#[tauri::command]
pub fn local_setup_status() -> Option<Status> {
    status().clone()
}

/// Starts setting up dictation on this computer (`speech_only`: just the
/// speech model, when a cloud key tidies up). Answers at once; the guide
/// follows `setup-progress` events. Starting again after a failure
/// carries on where it stopped; while it's working, it's left alone.
#[tauri::command]
pub fn start_local_setup(speech_only: bool, app: AppHandle) -> Result<Status, String> {
    if let Some(now) = status().as_ref().filter(|s| s.working) {
        return Ok(now.clone());
    }
    let plan = plan(speech_only)?;
    let mut cleanup = match plan.cleanup {
        "local" if plan.small => Cleanup::Local(LocalModel::Qwen3_4BCompact),
        "local" => Cleanup::Local(LocalModel::Qwen3_4B),
        "apple" => Cleanup::Apple,
        _ => Cleanup::None,
    };
    if plan.room == "none" {
        return Err(no_room(plan.free_up_mb));
    }
    // Room for speech but not the rest: speech alone, rather than a
    // question nobody can answer well. The guide says so afterwards.
    let no_room_for_ai = plan.room == "speech" && cleanup != Cleanup::None;
    if no_room_for_ai {
        cleanup = Cleanup::None;
    }
    let dir = engine::paths()?.models_dir;
    let speech = speech_model();
    let size = |done: bool, mb: u32| if done { 0 } else { u64::from(mb) * 1_000_000 };
    let speech_size = size(speech.is_downloaded(&dir), speech.approx_mb());
    let cleanup_size = match cleanup {
        Cleanup::Local(m) => size(m.is_downloaded(&dir), m.approx_mb()),
        _ => 0,
    };
    let voice_size = if cleanup == Cleanup::None {
        0
    } else {
        size(NaturalVoice::is_downloaded(&dir), NaturalVoice::APPROX_MB)
    };
    let start = Status {
        working: true,
        total: speech_size + cleanup_size + voice_size,
        speech_total: speech_size,
        no_room_for_ai,
        ..Status::default()
    };
    *status() = Some(start.clone());
    let sizes = Sizes {
        speech: speech_size,
        cleanup: cleanup_size,
        voice: voice_size,
    };
    tauri::async_runtime::spawn(async move {
        if no_room_for_ai
            && let Err(e) = change(&app, |s| {
                s.ai.provider = AiProvider::Off;
                s.ai.model = None;
            })
            .await
        {
            tracing::warn!("couldn't turn tidying up off: {e}");
        }
        if let Err(e) = run(&app, speech, cleanup, sizes).await {
            tracing::warn!("setting up on this computer: {e}");
            update(&app, |s| {
                s.working = false;
                s.error = Some(e);
            });
        }
    });
    Ok(start)
}

/// With no room even for speech to text: how much to free, in words.
fn no_room(free_up_mb: u64) -> String {
    let bin = if cfg!(target_os = "macos") {
        "Trash"
    } else {
        "Recycle Bin"
    };
    let gb = (free_up_mb as f64 / 1000.0).max(0.1);
    format!(
        "There isn't enough free space on this computer. Free up about {gb:.1} GB (empty the {bin}, or delete large files you don't need), then check again."
    )
}

/// What each download is expected to weigh, in bytes (0: already here or
/// not part of this setup).
struct Sizes {
    speech: u64,
    cleanup: u64,
    voice: u64,
}

/// The downloads, one after the other: speech first, so dictation can be
/// tried while the bigger tidy-up model is still on its way, then the
/// tidy-up model, then the reading voice.
async fn run(
    app: &AppHandle,
    speech: SpeechModel,
    cleanup: Cleanup,
    sizes: Sizes,
) -> Result<(), String> {
    // Sizes are guesses until each download says how big it is.
    let mut speech_total = sizes.speech;
    let mut cleanup_total = sizes.cleanup;
    let voice_total = sizes.voice;
    let mut last = Instant::now();
    crate::models::fetch_model(app, speech.id(), |done, total| {
        speech_total = total.unwrap_or(speech_total);
        if last.elapsed().as_millis() >= 250 {
            last = Instant::now();
            update(app, |s| {
                s.done = done;
                s.total = speech_total + cleanup_total + voice_total;
                s.speech_total = speech_total;
            });
        }
    })
    .await?;
    change(app, |s| {
        s.speech.provider = SpeechProvider::WhisperLocal;
        s.speech.model = None;
    })
    .await?;
    update(app, |s| {
        s.speech_ready = true;
        s.done = speech_total;
        s.total = speech_total + cleanup_total + voice_total;
        s.speech_total = speech_total;
    });

    match cleanup {
        Cleanup::Local(model) => {
            crate::models::fetch_model(app, model.id(), |done, total| {
                cleanup_total = total.unwrap_or(cleanup_total);
                if last.elapsed().as_millis() >= 250 {
                    last = Instant::now();
                    update(app, |s| {
                        s.done = speech_total + done;
                        s.total = speech_total + cleanup_total + voice_total;
                    });
                }
            })
            .await?;
            change(app, |s| use_local(s, model)).await?;
            update(app, |s| s.model = Some(model));
            fetch_voice(app, speech_total + cleanup_total, voice_total).await;
            update(app, |s| {
                s.done = s.total;
                s.checking = true;
            });
            let measured = speed_test(app).await;
            if let Some(ms) = measured {
                tracing::info!("the built-in model tidied up a dictation in {ms} ms");
                remember_speed(app, ms);
            }
            // Too slow to wait for at every dictation: off, unless the
            // person already said to keep it. The guide says so, with a
            // way to keep it anyway (`keep_slow_tidy_up`).
            let slow = measured.is_some_and(|ms| speed_of(ms) == "slow");
            let kept = app.state::<AppState>().settings().ai.slow_local_ok;
            if slow && !kept {
                tracing::info!("tidying up is too slow here: turned off");
                change(app, |s| {
                    s.ai.provider = AiProvider::Off;
                    s.ai.model = None;
                })
                .await?;
            }
            update(app, |s| {
                s.checking = false;
                s.local_ms = measured;
                s.speed = measured.map(speed_of);
                s.slow_off = slow && !kept;
            });
        }
        Cleanup::Apple => {
            change(app, |s| {
                s.ai.provider = AiProvider::Apple;
                s.ai.model = None;
            })
            .await?;
            fetch_voice(app, speech_total, voice_total).await;
        }
        Cleanup::None => {}
    }
    update(app, |s| {
        s.working = false;
        s.cleanup_ready = true;
        s.done = s.total;
        s.error = None;
    });
    Ok(())
}

/// Switches tidying up to the built-in `model`.
fn use_local(s: &mut Settings, model: LocalModel) {
    s.ai.provider = AiProvider::Local;
    s.ai.model = (model != LocalModel::Qwen3_4B).then(|| model.id().to_string());
}

/// The natural reading voice, last. It's small and only needed for
/// reading back, so a failure is noted and setup carries on without it
/// (Settings can fetch it later).
async fn fetch_voice(app: &AppHandle, before: u64, size: u64) {
    if size == 0 {
        return;
    }
    let mut last = Instant::now();
    let fetched = crate::models::fetch_model(app, NaturalVoice::ID, |done, _| {
        if last.elapsed().as_millis() >= 250 {
            last = Instant::now();
            update(app, |s| s.done = before + done.min(size));
        }
    })
    .await;
    match fetched {
        Ok(()) => {
            if let Some(e) = app.state::<AppState>().apply().await {
                tracing::warn!("after the reading voice arrived: {e}");
            }
        }
        Err(e) => tracing::warn!("the reading voice didn't download: {e}"),
    }
}

/// "Keep it on anyway", after tidying up was turned off for being slow:
/// on again, and not turned off for that again.
#[tauri::command]
pub async fn keep_slow_tidy_up(app: AppHandle) -> Result<(), String> {
    let model = status()
        .as_ref()
        .and_then(|s| s.model)
        .ok_or("Nothing to switch back on.")?;
    change(&app, |s| {
        use_local(s, model);
        s.ai.slow_local_ok = true;
    })
    .await?;
    update(&app, |s| s.slow_off = false);
    Ok(())
}

/// Changes the status, and tells the guide.
fn update(app: &AppHandle, f: impl FnOnce(&mut Status)) {
    let current = {
        let mut guard = status();
        let current = guard.get_or_insert_with(Status::default);
        f(current);
        current.clone()
    };
    let _ = app.emit("setup-progress", current);
}

/// "fast", "usable" or "slow", for `ms` per ordinary dictation.
pub fn speed_of(ms: u32) -> &'static str {
    if ms < splashdown_config::LOCAL_FAST_MS {
        "fast"
    } else if ms <= splashdown_config::LOCAL_SLOW_MS {
        "usable"
    } else {
        "slow"
    }
}

/// Times an ordinary dictation through the tidy-up model now in use, after
/// one that loads it. `None` if it couldn't run.
async fn speed_test(app: &AppHandle) -> Option<u32> {
    let state = app.state::<AppState>();
    let session = state.session.clone();
    let preset = state.settings().presets().into_iter().find(|p| p.ai)?.id;
    tauri::async_runtime::spawn_blocking(move || {
        session
            .redo(WARM_UP.into(), None, preset.clone())
            .inspect_err(|e| tracing::warn!("couldn't time the built-in model: {e}"))
            .ok()?;
        let started = Instant::now();
        session.redo(TIMED.into(), None, preset).ok()?;
        Some(u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX))
    })
    .await
    .ok()
    .flatten()
}

/// Keeps the measured speed (for Settings → AI Models), without rebuilding
/// anything: it changes nothing about how dictation runs.
fn remember_speed(app: &AppHandle, ms: u32) {
    let state = app.state::<AppState>();
    if let Err(e) = state.update(app, |s| s.ai.local_ms = Some(ms)) {
        tracing::warn!("couldn't keep the measured speed: {e}");
    }
}

/// Saves a change to the settings and puts it to use.
async fn change(app: &AppHandle, f: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.update(app, f)?;
    if let Some(e) = state.apply().await {
        tracing::warn!("after setting up: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_are_named() {
        assert_eq!(speed_of(900), "fast");
        assert_eq!(speed_of(3_400), "usable");
        assert_eq!(speed_of(5_000), "usable");
        assert_eq!(speed_of(9_400), "slow");
    }

    #[test]
    fn short_of_space_takes_speech_alone_or_says_how_much_to_free() {
        const MB: u64 = 1_000_000;
        assert_eq!(room_for(None, 495, 3_063), ("all", 0), "unknown");
        assert_eq!(room_for(Some(80_000 * MB), 495, 3_063), ("all", 0));
        assert_eq!(room_for(Some(2_000 * MB), 495, 3_063), ("speech", 0));
        let (room, free_up) = room_for(Some(1_000 * MB), 495, 3_063);
        assert_eq!(room, "none");
        // 495 MB and the spare GiB, less the 1,000 MB there is.
        assert_eq!(free_up, 495 + SPARE.div_ceil(MB) - 1_000);
        assert_eq!(room_for(Some(0), 0, 0), ("all", 0), "all here already");
    }

    #[test]
    fn the_best_tidy_up_the_computer_can_run() {
        const GB: u64 = 1 << 30;
        let big = Cleanup::Local(LocalModel::Qwen3_4B);
        let small = Cleanup::Local(LocalModel::Qwen3_4BCompact);
        assert_eq!(cleanup_for(Some(16 * GB), true), big, "Apple's does worse");
        assert_eq!(cleanup_for(Some(16 * GB), false), big);
        assert_eq!(cleanup_for(None, false), big, "memory unknown");
        assert_eq!(cleanup_for(Some(8 * GB), true), Cleanup::Apple);
        assert_eq!(cleanup_for(Some(8 * GB), false), small);
    }
}
