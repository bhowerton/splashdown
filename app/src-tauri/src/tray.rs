//! Menu bar (macOS) / system tray (Windows) icon and menu.

use std::sync::Arc;

use splashdown_config::Preset;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::actions;
use crate::history::History;

const SETTINGS_WINDOW: &str = "settings";
pub const ONBOARDING_WINDOW: &str = "onboarding";
/// Shown next to the menu item; the global shortcut itself is registered in
/// `main.rs`.
const PASTE_RAW_ACCELERATOR: &str = "Ctrl+Alt+V";
/// Shown in the menu; registered in `shortcuts.rs`.
const UNDO_AI_ACCELERATOR: &str = "Ctrl+Alt+Z";

// macOS draws a monochrome template icon that adapts to light/dark menu
// bars; Windows shows the full-color app icon. The tray glyphs are the 36 px
// (@2x) renders of icons/source/tray-*.svg; the menu bar shows them at 18 pt.
#[cfg(target_os = "macos")]
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray-template@2x.png");
#[cfg(not(target_os = "macos"))]
const TRAY_ICON: &[u8] = include_bytes!("../icons/32x32.png");
const TRAY_RECORDING: &[u8] = include_bytes!("../icons/tray-recording@2x.png");
#[cfg(target_os = "macos")]
const TRAY_ATTENTION: &[u8] = include_bytes!("../icons/tray-attention@2x.png");
#[cfg(not(target_os = "macos"))]
const TRAY_ATTENTION: &[u8] = include_bytes!("../icons/32x32.png");

const TRAY_ID: &str = "main";
const RECORD_MEETING: &str = "Record a meeting";

/// The meeting item, relabeled while a meeting records.
static MEETING_ITEM: std::sync::OnceLock<MenuItem<Wry>> = std::sync::OnceLock::new();
/// "Retry last dictation", enabled while a failed recording is kept.
static RETRY_ITEM: std::sync::OnceLock<MenuItem<Wry>> = std::sync::OnceLock::new();

/// Enables "Retry last dictation" while a failed recording is kept.
pub fn show_retry(kept: bool) {
    if let Some(item) = RETRY_ITEM.get() {
        let _ = item.set_enabled(kept);
    }
}

/// Shows a meeting in progress (`Some("12:30")`) or none: the menu item's
/// label, and the time next to the menu bar icon on macOS.
pub fn show_meeting(app: &AppHandle, elapsed: Option<&str>) {
    if let Some(item) = MEETING_ITEM.get() {
        let _ = item.set_text(match elapsed {
            Some(t) => format!("Stop meeting ({t})"),
            None => RECORD_MEETING.to_string(),
        });
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        #[cfg(target_os = "macos")]
        let _ = tray.set_title(elapsed.map(|t| format!("● {t}")));
        let _ = tray.set_tooltip(Some(match elapsed {
            Some(t) => format!("Splashdown · recording a meeting ({t})"),
            None => "Splashdown".to_string(),
        }));
    }
}

/// The spoken-language choices in the menu bar (the same as Settings →
/// General). An empty code means "detect automatically".
const LANGUAGES: [(&str, &str); 11] = [
    ("", "Detect automatically"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
    ("ko", "Korean"),
];
const CHECK_FOR_UPDATES: &str = "Check for updates…";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIconState {
    Idle,
    Recording,
    /// Setup is incomplete (permission missing, no provider).
    Attention,
}

/// The parts of the menu the session updates.
#[derive(Clone)]
pub struct TrayMenu {
    pub status: MenuItem<Wry>,
    presets: Submenu<Wry>,
    languages: Submenu<Wry>,
    whisper: CheckMenuItem<Wry>,
    mini_bar: CheckMenuItem<Wry>,
    /// "Check for updates…", or "Update to vX.Y.Z…" once one is found.
    update: MenuItem<Wry>,
}

/// Installs the tray icon and its menu.
pub fn install(app: &AppHandle, history: Arc<History>) -> tauri::Result<TrayMenu> {
    let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
    let presets = Submenu::with_id(app, "presets", "Preset", true)?;
    let languages = Submenu::with_id(app, "languages", "Language", true)?;
    let whisper = CheckMenuItem::with_id(
        app,
        "whisper",
        "Whisper mode",
        true,
        crate::engine::load_settings().general.whisper_mode(),
        None::<&str>,
    )?;
    let mini_bar = CheckMenuItem::with_id(
        app,
        "mini-bar",
        "Show mini bar",
        true,
        crate::engine::load_settings().general.mini_bar(),
        None::<&str>,
    )?;
    let current = crate::engine::load_settings().general.language;
    for (code, name) in LANGUAGES {
        languages.append(&CheckMenuItem::with_id(
            app,
            format!("lang:{code}"),
            name,
            true,
            current.as_deref().unwrap_or("") == code,
            None::<&str>,
        )?)?;
    }
    let paste_raw = MenuItem::with_id(
        app,
        "paste-raw",
        "Paste last raw transcript",
        true,
        Some(PASTE_RAW_ACCELERATOR),
    )?;
    let undo_ai = MenuItem::with_id(
        app,
        "undo-ai",
        "Undo AI edit",
        true,
        Some(UNDO_AI_ACCELERATOR),
    )?;
    let copy_last = MenuItem::with_id(app, "copy-last", "Copy last result", true, None::<&str>)?;
    let retry = MenuItem::with_id(
        app,
        "retry-last",
        "Retry last dictation",
        crate::rescue::kept().is_some(),
        None::<&str>,
    )?;
    let _ = RETRY_ITEM.set(retry.clone());
    let notes = MenuItem::with_id(app, "notes", "Notes", true, Some(crate::notes::ACCELERATOR))?;
    let meeting = MenuItem::with_id(
        app,
        "meeting",
        RECORD_MEETING,
        true,
        Some(crate::meeting::ACCELERATOR),
    )?;
    let _ = MEETING_ITEM.set(meeting.clone());
    let what_to_say = MenuItem::with_id(app, "help", "What you can say…", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let update = MenuItem::with_id(app, "update", CHECK_FOR_UPDATES, true, None::<&str>)?;
    let feedback = MenuItem::with_id(app, "feedback", "Send feedback…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Splashdown", true, Some("CmdOrCtrl+Q"))?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &presets,
            &languages,
            &whisper,
            &mini_bar,
            &undo_ai,
            &paste_raw,
            &copy_last,
            &retry,
            &notes,
            &meeting,
            &PredefinedMenuItem::separator(app)?,
            &what_to_say,
            &settings,
            &update,
            &feedback,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(TRAY_ICON)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Splashdown")
        .menu(&menu)
        // Mac menu bar items open on click; on Windows, left-click opens
        // Settings and right-click opens the menu.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            match id {
                "paste-raw" => actions::paste_last_raw(&history),
                "copy-last" => actions::copy_last(&history),
                "retry-last" => {
                    if let Some(state) = app.try_state::<crate::commands::AppState>() {
                        state.session.retry_last();
                    }
                }
                "undo-ai" => {
                    if let Some(state) = app.try_state::<crate::commands::AppState>() {
                        state.session.undo_ai_edit();
                    }
                }
                "notes" => crate::notes::open(app),
                "meeting" => crate::meeting::toggle(app),
                "help" => crate::help::open(app),
                "settings" => open_settings(app),
                "mini-bar" => toggle_mini_bar(app),
                "update" => show_updates(app),
                "feedback" => send_feedback(app),
                "quit" if crate::meeting::is_running() => crate::meeting::quit(app),
                "quit" => {
                    splashdown_platform::restore_output();
                    app.exit(0);
                }
                _ => {
                    if id == "whisper" {
                        toggle_whisper(app);
                    } else if let Some(code) = id.strip_prefix("lang:") {
                        set_language(app, code);
                    } else if let Some(preset) = id.strip_prefix("preset:")
                        && let Some(state) = app.try_state::<crate::commands::AppState>()
                    {
                        state
                            .session
                            .pin_preset((preset != "auto").then(|| preset.to_string()));
                    }
                }
            }
        });

    #[cfg(not(target_os = "macos"))]
    let builder = builder.on_tray_icon_event(|tray, event| {
        use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            open_settings(tray.app_handle());
        }
    });

    builder.build(app)?;
    Ok(TrayMenu {
        status,
        presets,
        languages,
        whisper,
        mini_bar,
        update,
    })
}

impl TrayMenu {
    pub fn set_icon(&self, app: &AppHandle, state: TrayIconState) {
        let Some(tray) = app.tray_by_id(TRAY_ID) else {
            return;
        };
        let (bytes, template) = match state {
            TrayIconState::Idle => (TRAY_ICON, cfg!(target_os = "macos")),
            TrayIconState::Recording => (TRAY_RECORDING, false),
            TrayIconState::Attention => (TRAY_ATTENTION, cfg!(target_os = "macos")),
        };
        if let Ok(image) = Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(image));
            let _ = tray.set_icon_as_template(template);
        }
    }

    /// "Automatic" plus every preset, with a check on the active choice.
    pub fn set_presets(&self, app: &AppHandle, presets: &[Preset], pinned: Option<&str>) {
        let result: tauri::Result<()> = (|| {
            clear(&self.presets)?;
            self.presets.append(&CheckMenuItem::with_id(
                app,
                "preset:auto",
                "Automatic (by app)",
                true,
                pinned.is_none(),
                None::<&str>,
            )?)?;
            self.presets.append(&PredefinedMenuItem::separator(app)?)?;
            for p in presets {
                self.presets.append(&CheckMenuItem::with_id(
                    app,
                    format!("preset:{}", p.id),
                    &p.name,
                    true,
                    pinned == Some(p.id.as_str()),
                    None::<&str>,
                )?)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!("couldn't update preset menu: {e}");
        }
    }

    /// Ticks "Whisper mode" to match the setting.
    pub fn show_whisper(&self, on: bool) {
        let _ = self.whisper.set_checked(on);
    }

    /// Ticks "Show mini bar" to match the setting.
    pub fn show_mini_bar(&self, on: bool) {
        let _ = self.mini_bar.set_checked(on);
    }

    /// Ticks the spoken language in effect (`None`: detect automatically).
    pub fn show_language(&self, language: Option<&str>) {
        let current = language.unwrap_or("");
        let Ok(items) = self.languages.items() else {
            return;
        };
        for item in items {
            if let Some(check) = item.as_check_menuitem() {
                let code = check
                    .id()
                    .as_ref()
                    .strip_prefix("lang:")
                    .unwrap_or_default();
                let _ = check.set_checked(code == current);
            }
        }
    }

    /// Names the available update, if any, in the menu.
    pub fn set_update(&self, version: Option<&str>) {
        let label = match version {
            Some(v) => format!("Update to v{}…", v.trim_start_matches('v')),
            None => CHECK_FOR_UPDATES.to_string(),
        };
        if let Err(e) = self.update.set_text(label) {
            tracing::warn!("couldn't update the updates menu item: {e}");
        }
    }
}

/// Whisper mode from the menu bar: saves the new setting and rebuilds.
fn toggle_whisper(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<crate::commands::AppState>() else {
            return;
        };
        if let Err(e) = state.toggle_whisper_mode().await {
            tracing::warn!("couldn't change whisper mode: {e}");
        }
        crate::commands::settings_changed_elsewhere(&app);
    });
}

/// "Show mini bar" from the menu bar: saves the new setting.
fn toggle_mini_bar(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<crate::commands::AppState>() else {
            return;
        };
        let on = !state.settings().general.mini_bar();
        if let Err(e) = state.set_mini_bar(on) {
            tracing::warn!("couldn't change the mini bar: {e}");
        }
        crate::commands::settings_changed_elsewhere(&app);
    });
}

/// Saves the spoken language chosen in the menu bar and rebuilds the
/// engine with it.
fn set_language(app: &AppHandle, code: &str) {
    let language = (!code.is_empty()).then(|| code.to_string());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<crate::commands::AppState>() else {
            return;
        };
        if let Err(e) = state.set_language(language).await {
            tracing::warn!("couldn't change the language: {e}");
        }
        crate::commands::settings_changed_elsewhere(&app);
    });
}

fn clear(menu: &Submenu<Wry>) -> tauri::Result<()> {
    for item in menu.items()? {
        menu.remove(&item)?;
    }
    Ok(())
}

pub fn open_settings(app: &AppHandle) {
    open_window(
        app,
        SETTINGS_WINDOW,
        "index.html",
        "Splashdown Settings",
        (960.0, 680.0),
        true,
    );
}

/// App event asking an open Settings window to switch tab.
const SHOW_TAB: &str = "show-tab";

/// Opens Settings on one tab ("about").
pub fn open_settings_tab(app: &AppHandle, tab: &str) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = app.emit_to(SETTINGS_WINDOW, SHOW_TAB, tab);
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    open_window(
        app,
        SETTINGS_WINDOW,
        &format!("index.html#{tab}"),
        "Splashdown Settings",
        (960.0, 680.0),
        true,
    );
}

/// Settings → About, where the update (or "Check now") is. Without a known
/// update, also starts a check so the page has something to say.
fn show_updates(app: &AppHandle) {
    open_settings_tab(app, "about");
    let has_update = app
        .try_state::<crate::commands::AppState>()
        .is_some_and(|s| s.updates.available().is_some());
    if !has_update {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::updates::check(&app, true).await;
        });
    }
}

/// Opens a prefilled bug report in the browser. Off the main thread:
/// resolving the providers asks macOS about its on-device models.
fn send_feedback(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(state) = app.try_state::<crate::commands::AppState>()
            && let Err(e) = state.send_feedback()
        {
            tracing::warn!("couldn't open the feedback page: {e}");
        }
    });
}

/// The first-run setup guide.
pub fn open_onboarding(app: &AppHandle) {
    open_window(
        app,
        ONBOARDING_WINDOW,
        "onboarding.html",
        "Welcome to Splashdown",
        (680.0, 600.0),
        false,
    );
}

/// Shows the window if it's open, otherwise creates it.
fn open_window(
    app: &AppHandle,
    label: &str,
    page: &str,
    title: &str,
    (width, height): (f64, f64),
    resizable: bool,
) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title(title)
        .inner_size(width, height)
        .min_inner_size(width.min(640.0), height.min(480.0))
        .resizable(resizable)
        .center()
        .focused(true)
        .build();
    if let Err(err) = result {
        tracing::error!("failed to open {label} window: {err}");
    }
}
