//! Other apps: the one in front, bringing one forward, the apps running,
//! their names and icons, which ones use the microphone or play sound,
//! recording one app's sound, and opening links and files.

use super::*;

/// Opens a web page in the default browser.
/// On Windows, `ShellExecuteW` (no console, and any characters in a path),
/// which also opens `ms-settings:` pages and files such as an `.ics`.
pub fn open_url(url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        imp::shell_open(url)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| PlatformError::Failed(e.to_string()))
    }
}

/// Shows `path` selected in Finder or File Explorer.
pub fn reveal_in_file_manager(path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        // Explorer wants `/select,"C:\a b\c.md"` as written: Rust's usual
        // quoting of the whole argument confuses it, and so do the forward
        // slashes of a note in a subfolder.
        let path = path.to_string_lossy().replace('/', "\\");
        let mut c = std::process::Command::new("explorer");
        c.raw_arg(format!("/select,\"{path}\""));
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    };
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| PlatformError::Failed(e.to_string()))
}

/// The OS version, e.g. "26.0.1" (`None` where it isn't known).
pub fn os_version() -> Option<String> {
    imp::os_version()
}

/// An installed app as people know it.
#[derive(Debug, Clone)]
pub struct AppDisplay {
    pub name: String,
    /// A small square icon (PNG), if one could be drawn.
    pub icon_png: Option<Vec<u8>>,
}

/// The name and icon of the app with this id (a bundle ID on macOS), or
/// `None` if it isn't installed or can't be looked up here.
pub fn app_display(app_id: &str) -> Option<AppDisplay> {
    imp::app_display(app_id)
}

/// The app in front: its process id, name, and id (bundle ID on macOS,
/// executable name on Windows). Doesn't ask Accessibility, so it's cheap
/// and safe on the main thread, even when we are in front.
pub fn frontmost_app() -> Option<(i32, String, String)> {
    imp::frontmost_app()
}

/// Brings the app with this process id to the front, to hand focus back
/// after one of our windows had it. Call while our app is active (macOS
/// only lets the active app pass activation on).
pub fn activate_app(pid: i32) -> Result<()> {
    imp::activate_app(pid)
}

/// Apps with a window open on Windows, by name, with their process ids
/// (for "switch to …"). Empty on macOS, where opening an app that's
/// already running brings it forward anyway.
pub fn running_apps() -> Vec<(String, i32)> {
    #[cfg(windows)]
    {
        imp::running_apps()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Other apps recording from a microphone right now, by bundle ID on
/// macOS (14 or later; none on earlier versions) or by executable name or
/// package family name on Windows. Our own process isn't included.
pub fn microphone_users() -> Vec<String> {
    imp::microphone_users()
}

/// An app that plays sound, to transcribe what it plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioApp {
    /// Its bundle ID (macOS) or executable's file name (Windows).
    pub id: String,
    pub name: String,
    /// Playing sound now.
    pub playing: bool,
    /// What to record: the app's Core Audio process objects (macOS, for
    /// the shim's tap), or the process whose tree plays its sound (Windows,
    /// for `app_loopback`).
    pub sources: Vec<u32>,
}

/// The apps that play (or have played) sound since they started, playing
/// ones first. Our own app isn't included.
pub fn audio_apps() -> Vec<AudioApp> {
    imp::audio_apps()
}

/// Where recorded sound goes: mono samples and their rate, on an audio
/// thread.
pub type SoundSink = Box<dyn FnMut(&[f32], u32) + Send>;

/// Records what process `pid` and the processes it started play (one of
/// `AudioApp::sources`), handing mono samples and their rate to `sink` as
/// they come. Windows 10 2004 or later.
#[cfg(windows)]
pub fn app_loopback(pid: u32, sink: SoundSink) -> Result<AppLoopback> {
    imp::app_loopback(pid, sink)
}
