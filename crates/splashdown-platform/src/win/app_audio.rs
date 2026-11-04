//! What one app plays, to transcribe it: the apps with an audio session on
//! any speaker, and process loopback (Windows 10 2004 or later), which
//! records a process and the processes it started (a browser's tabs) without
//! anything else that's playing.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, S_OK, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync, AudioSessionStateActive,
    DEVICE_STATE_ACTIVE, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, IAgileObject,
    IAgileObject_Impl,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{Interface, Ref, implement};

use crate::{AudioApp, PlatformError, Result};

/// The rate and layout asked for: process loopback converts to it.
const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// How long the capture buffer is, in 100 ns units (20 ms).
const BUFFER: i64 = 200_000;

/// The apps with an audio session on an active speaker or headset, each
/// with the process whose tree plays its sound. Playing ones first.
pub(crate) fn audio_apps() -> Vec<AudioApp> {
    // SAFETY: initializes COM for this thread; an already-initialized
    // thread just gets S_FALSE or a mode error, which is fine.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let me = std::process::id();
    let processes = processes();
    let mut apps: Vec<AudioApp> = Vec::new();
    for (pid, playing) in sessions() {
        if pid == 0 || pid == me {
            continue;
        }
        let Some((_, exe)) = processes.get(&pid) else {
            continue;
        };
        // The browser itself, not the tab's audio process: its tree has
        // every tab.
        let root = root_of(pid, &processes);
        let name = super::apps::exe_path(root)
            .and_then(|path| super::apps::file_description(&path))
            .unwrap_or_else(|| exe.trim_end_matches(".exe").to_string());
        match apps.iter_mut().find(|a| a.id.eq_ignore_ascii_case(exe)) {
            Some(app) => app.playing |= playing,
            None => apps.push(AudioApp {
                id: exe.clone(),
                name,
                playing,
                sources: vec![root],
            }),
        }
    }
    apps.sort_by(|a, b| {
        b.playing
            .cmp(&a.playing)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    apps
}

/// Every audio session's process and whether it's playing now.
fn sessions() -> Vec<(u32, bool)> {
    let mut out = Vec::new();
    // SAFETY: MMDeviceEnumerator is the documented class implementing
    // IMMDeviceEnumerator; no aggregation.
    let Ok(enumerator) = (unsafe {
        CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
    }) else {
        return out;
    };
    // SAFETY: COM calls on live interfaces; every out value is owned by the
    // wrappers returned.
    unsafe {
        let Ok(devices) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return out;
        };
        for i in 0..devices.GetCount().unwrap_or(0) {
            let Ok(device) = devices.Item(i) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(list) = manager.GetSessionEnumerator() else {
                continue;
            };
            for j in 0..list.GetCount().unwrap_or(0) {
                let Ok(control) = list.GetSession(j) else {
                    continue;
                };
                let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                if control.IsSystemSoundsSession() == S_OK {
                    continue;
                }
                let Ok(pid) = control.GetProcessId() else {
                    continue;
                };
                let playing = control
                    .GetState()
                    .is_ok_and(|s| s == AudioSessionStateActive);
                out.push((pid, playing));
            }
        }
    }
    out
}

/// Every process: its parent and executable's file name.
fn processes() -> HashMap<u32, (u32, String)> {
    let mut out = HashMap::new();
    // SAFETY: a snapshot of the process list, closed below; the entries are
    // read into a properly sized struct.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            out.insert(entry.th32ProcessID, (entry.th32ParentProcessID, exe));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

/// The topmost ancestor of `pid` with the same executable (a browser's main
/// process for one of its tabs).
fn root_of(pid: u32, processes: &HashMap<u32, (u32, String)>) -> u32 {
    let mut at = pid;
    for _ in 0..16 {
        let Some((parent, exe)) = processes.get(&at) else {
            break;
        };
        match processes.get(parent) {
            Some((_, parent_exe)) if parent_exe.eq_ignore_ascii_case(exe) && *parent != at => {
                at = *parent;
            }
            _ => break,
        }
    }
    at
}

/// Receives the activation's result (on a COM thread).
#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Activated(mpsc::Sender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

// It may be called on any thread, as activation requires.
impl IAgileObject_Impl for Activated_Impl {}

/// Recording one app's sound; it stops when dropped.
pub struct AppLoopback {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for AppLoopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Records what process `pid` and the processes it started play, handing
/// mono samples and their rate to `sink` on a thread of its own.
pub(crate) fn app_loopback(pid: u32, mut sink: crate::SoundSink) -> Result<AppLoopback> {
    let stop = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = mpsc::channel::<Result<()>>();
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("app-loopback".into())
        .spawn(move || {
            let (client, capture, event) = match start(pid) {
                Ok(parts) => {
                    let _ = started_tx.send(Ok(()));
                    parts
                }
                Err(e) => {
                    let _ = started_tx.send(Err(e));
                    return;
                }
            };
            let mut mono = Vec::new();
            while !flag.load(Ordering::Relaxed) {
                // SAFETY: a valid event handle, closed after the loop.
                if unsafe { WaitForSingleObject(event, 100) } != WAIT_OBJECT_0 {
                    continue;
                }
                // SAFETY: the capture client is live; each buffer is read
                // between GetBuffer and ReleaseBuffer, `frames` frames of
                // the float stereo format it was initialized with.
                unsafe {
                    while capture.GetNextPacketSize().unwrap_or(0) > 0 {
                        let mut data = std::ptr::null_mut();
                        let mut frames = 0u32;
                        let mut flags = 0u32;
                        if capture
                            .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                            .is_err()
                        {
                            break;
                        }
                        mono.clear();
                        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                            mono.resize(frames as usize, 0.0);
                        } else {
                            let samples = std::slice::from_raw_parts(
                                data.cast::<f32>(),
                                frames as usize * usize::from(CHANNELS),
                            );
                            mono.extend(
                                samples
                                    .as_chunks::<2>()
                                    .0
                                    .iter()
                                    .map(|[l, r]| (l + r) / 2.0),
                            );
                        }
                        let _ = capture.ReleaseBuffer(frames);
                        sink(&mono, RATE);
                    }
                }
            }
            // SAFETY: stopping a started client and closing our own event.
            unsafe {
                let _ = client.Stop();
                let _ = CloseHandle(event);
            }
        })
        .map_err(|e| PlatformError::Failed(e.to_string()))?;
    match started_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(AppLoopback {
            stop,
            thread: Some(thread),
        }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err(PlatformError::Failed(
                "Windows didn't start recording the app".into(),
            ))
        }
    }
}

/// Activates process loopback for `pid` and starts it.
fn start(pid: u32) -> Result<(IAudioClient, IAudioCaptureClient, HANDLE)> {
    let failed = |what: &str, e: windows::core::Error| {
        PlatformError::Failed(format!("couldn't record the app ({what}: {e})"))
    };
    // SAFETY: COM for this thread, which lives as long as the capture.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let blob = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: (&raw mut params).cast(),
                    },
                },
                ..Default::default()
            }),
        },
    };
    let (done_tx, done_rx) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done_tx).into();
    // SAFETY: `blob` points at `params`, both alive until activation
    // completes (waited for below); the handler is a COM object we own.
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&raw const blob),
            &handler,
        )
    }
    .map_err(|e| failed("activation", e))?;
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| PlatformError::Failed("Windows didn't answer".into()))?;
    let mut result = windows::core::HRESULT(0);
    let mut activated = None;
    // SAFETY: the operation completed; both out-pointers are valid.
    unsafe { operation.GetActivateResult(&mut result, &mut activated) }
        .map_err(|e| failed("activation result", e))?;
    result.ok().map_err(|e| failed("activation", e))?;
    let client: IAudioClient = activated
        .ok_or_else(|| PlatformError::Failed("no audio client".into()))?
        .cast()
        .map_err(|e| failed("audio client", e))?;
    let block = CHANNELS * 4;
    let format = WAVEFORMATEX {
        // WAVE_FORMAT_IEEE_FLOAT
        wFormatTag: 3,
        nChannels: CHANNELS,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * u32::from(block),
        nBlockAlign: block,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    // SAFETY: plain COM calls on the activated client with a valid format;
    // the event is closed by the capture thread when it ends.
    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                BUFFER,
                0,
                &format,
                None,
            )
            .map_err(|e| failed("set-up", e))?;
        let capture: IAudioCaptureClient = client.GetService().map_err(|e| failed("capture", e))?;
        let event = CreateEventW(None, false, false, None).map_err(|e| failed("event", e))?;
        client
            .SetEventHandle(event)
            .map_err(|e| failed("event", e))?;
        client.Start().map_err(|e| failed("start", e))?;
        Ok((client, capture, event))
    }
}
