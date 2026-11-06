//! Downloading models (speech, the built-in text model, Apple's speech
//! model) with progress for every window.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use splashdown_providers::{LocalModel, SpeechModel};
use tauri::{AppHandle, Emitter, State};

use crate::engine;
use crate::snapshot::*;
use crate::state::*;

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub(super) id: String,
    pub(super) done: u64,
    pub(super) total: Option<u64>,
}

/// How a download ended, for every window (one opened after it started
/// can't see the command's own answer).
#[derive(Clone, Serialize)]
pub(super) struct DownloadDone {
    pub(super) id: String,
    pub(super) error: Option<String>,
}

/// Models downloading now, with how far each has got.
pub(super) static DOWNLOADS: Mutex<Vec<DownloadProgress>> = Mutex::new(Vec::new());

pub(super) fn downloads() -> std::sync::MutexGuard<'static, Vec<DownloadProgress>> {
    DOWNLOADS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Takes a model off `DOWNLOADS` however its download ends.
pub(super) struct Downloading(String);

impl Drop for Downloading {
    fn drop(&mut self) {
        downloads().retain(|d| d.id != self.0);
    }
}

/// Downloads a model for Settings, then rebuilds the engine so it's used.
#[tauri::command]
pub async fn download_model(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Saved, String> {
    fetch_model(&app, &id, |_, _| {}).await?;
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
    })
}

/// Downloads a model, unless it's already here, telling every window how
/// it's going (and `on_progress`). A second request while it downloads
/// (another click, a window opened again, the setup guide) waits for that
/// one rather than writing the same file twice.
pub async fn fetch_model(
    app: &AppHandle,
    id: &str,
    mut on_progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<(), String> {
    use splashdown_providers::models::{Diarizer, Embedder, VoiceModel};
    let dir = engine::paths()?.models_dir;
    let here = |dir: &std::path::Path| {
        if id == splashdown_providers::NaturalVoice::ID {
            Some(splashdown_providers::NaturalVoice::is_downloaded(dir))
        } else if id == Diarizer::ID {
            Some(Diarizer::is_downloaded(dir))
        } else if id == Embedder::ID {
            Some(Embedder::is_downloaded(dir))
        } else if id == VoiceModel::ID {
            Some(VoiceModel::is_downloaded(dir))
        } else if let Some(m) = SpeechModel::from_id(id) {
            Some(m.is_downloaded(dir))
        } else {
            LocalModel::from_id(id).map(|m| m.is_downloaded(dir))
        }
    };
    let Some(already) = here(&dir) else {
        return Err("Unknown model.".into());
    };
    if already {
        return Ok(());
    }
    let started = {
        let mut list = downloads();
        let running = list.iter().any(|d| d.id == id);
        if !running {
            list.push(DownloadProgress {
                id: id.to_string(),
                done: 0,
                total: None,
            });
        }
        !running
    };
    if !started {
        // Downloading already: follow it to the end.
        loop {
            let now = downloads().iter().find(|d| d.id == id).cloned();
            let Some(now) = now else { break };
            on_progress(now.done, now.total);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        return match here(&dir) {
            Some(true) => Ok(()),
            _ => Err("The download didn't finish. Try again.".into()),
        };
    }
    let downloading = Downloading(id.to_string());
    let mut last_emit = Instant::now();
    let result = splashdown_providers::models::download_by_id(id, &dir, |done, total| {
        if let Some(d) = downloads().iter_mut().find(|d| d.id == id) {
            d.done = done;
            d.total = total;
        }
        on_progress(done, total);
        // Throttle progress events to ~10 per second.
        if last_emit.elapsed().as_millis() >= 100 || total == Some(done) {
            last_emit = Instant::now();
            let _ = app.emit(
                "model-download",
                DownloadProgress {
                    id: id.to_string(),
                    done,
                    total,
                },
            );
        }
    })
    .await
    .map(|_| ())
    .map_err(|e| download_error(&e));
    drop(downloading);
    let _ = app.emit(
        "model-downloaded",
        DownloadDone {
            id: id.to_string(),
            error: result.as_ref().err().cloned(),
        },
    );
    result
}

/// A download's failure, in words for the person who clicked Download.
pub(super) fn download_error(e: &splashdown_core::ProviderError) -> String {
    use splashdown_core::ProviderError;
    match e {
        ProviderError::Network(why) => format!(
            "The download didn't finish ({why}). Check the internet connection and try again: it carries on where it stopped."
        ),
        other => format!("The download didn't finish: {other}"),
    }
}

/// Downloads Apple's speech model for the configured language (from
/// Apple), then rebuilds the engine so "Automatic" can pick it up.
#[tauri::command]
pub async fn prepare_apple_speech(state: State<'_, AppState>) -> Result<Saved, String> {
    let language = state.settings().general.language;
    splashdown_providers::apple::prepare_speech(language.as_deref()).await?;
    let engine_error = state.apply().await;
    Ok(Saved {
        state: state.snapshot(),
        engine_error,
    })
}
