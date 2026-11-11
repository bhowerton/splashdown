//! Menu bar (macOS) / system tray (Windows) icon and menu.

use std::sync::Arc;

use splashdown_config::{Preset, Settings};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::actions;
use crate::history::History;

const SETTINGS_WINDOW: &str = "settings";
pub const ONBOARDING_WINDOW: &str = "onboarding";
/// Shown next to the menu item; the global shortcut itself is registered in
/// `shortcuts.rs`.
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
static FORGET_ITEM: std::sync::OnceLock<MenuItem<Wry>> = std::sync::OnceLock::new();
const FORGET_NONE: &str = "Forget a learned word";

/// The Forget item in the menu bar: the word learned last, or none.
pub fn show_forget(word: Option<&str>) {
    if let Some(item) = FORGET_ITEM.get() {
        let _ = item.set_text(match word {
            Some(w) => format!("Forget \u{201c}{w}\u{201d}"),
            None => FORGET_NONE.to_string(),
        });
        let _ = item.set_enabled(word.is_some());
    }
}
/// Last dictation → Retry, enabled while a failed recording is kept.
static RETRY_ITEM: std::sync::OnceLock<MenuItem<Wry>> = std::sync::OnceLock::new();

/// Enables Last dictation → Retry while a failed recording is kept.
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
    /// "Translate into": the languages the translate key and ⌃⌥T use.
    translate: Submenu<Wry>,
    menu: Menu<Wry>,
    /// "Update to vX.Y.Z…", in the menu (under the status) only while an
    /// update is available.
    update_ready: MenuItem<Wry>,
}

/// Where "Update to vX.Y.Z…" goes: right under the status.
const UPDATE_READY_AT: usize = 1;

/// Installs the tray icon and its menu.
pub fn install(app: &AppHandle, history: Arc<History>) -> tauri::Result<TrayMenu> {
    let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
    let presets = Submenu::with_id(app, "presets", "Preset", true)?;
    let languages = Submenu::with_id(app, "languages", "Language", true)?;
    let translate = Submenu::with_id(app, "translate", "Translate into", true)?;
    let settings = crate::engine::load_settings();
    fill_translate(app, &translate, &settings)?;
    let current = settings.general.language;
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
        "Paste raw transcript",
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
    let copy_last = MenuItem::with_id(app, "copy-last", "Copy result", true, None::<&str>)?;
    let read_last = MenuItem::with_id(app, "read-last", "Read it to me", true, None::<&str>)?;
    let retry = MenuItem::with_id(
        app,
        "retry-last",
        "Retry",
        crate::rescue::kept().is_some(),
        None::<&str>,
    )?;
    let _ = RETRY_ITEM.set(retry.clone());
    let forget = MenuItem::with_id(app, "forget-learned", FORGET_NONE, false, None::<&str>)?;
    let _ = FORGET_ITEM.set(forget.clone());
    let notes = MenuItem::with_id(app, "notes", "Notes", true, Some(crate::notes::ACCELERATOR))?;
    let meeting = MenuItem::with_id(
        app,
        "meeting",
        RECORD_MEETING,
        true,
        Some(crate::meeting::ACCELERATOR),
    )?;
    let _ = MEETING_ITEM.set(meeting.clone());
    let last = Submenu::with_id_and_items(
        app,
        "last",
        "Last dictation",
        true,
        &[
            &undo_ai, &paste_raw, &copy_last, &read_last, &retry, &forget,
        ],
    )?;
    let what_to_say = MenuItem::with_id(app, "help", "What you can say…", true, None::<&str>)?;
    let feedback = MenuItem::with_id(app, "feedback", "Send feedback…", true, None::<&str>)?;
    let check_updates = MenuItem::with_id(app, "update", CHECK_FOR_UPDATES, true, None::<&str>)?;
    let help = Submenu::with_id_and_items(
        app,
        "help-menu",
        "Help",
        true,
        &[&what_to_say, &feedback, &check_updates],
    )?;
    let update_ready = MenuItem::with_id(app, "update-ready", "Update…", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let quit = MenuItem::with_id(app, "quit", "Quit Splashdown", true, Some("CmdOrCtrl+Q"))?;
    // Kept short: what's used every day at the top level, the rest one
    // level down, and set-once choices (whisper mode, the mini bar) in
    // Settings only.
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &notes,
            &meeting,
            &PredefinedMenuItem::separator(app)?,
            &presets,
            &languages,
            &translate,
            &last,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &help,
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
                "read-last" => actions::read_last(&history),
                "retry-last" => {
                    if let Some(state) = app.try_state::<crate::state::AppState>() {
                        state.session.retry_last();
                    }
                }
                "forget-learned" => crate::learning::forget_last(app),
                "undo-ai" => {
                    if let Some(state) = app.try_state::<crate::state::AppState>() {
                        state.session.undo_ai_edit();
                    }
                }
                "notes" => crate::notes::open(app),
                "meeting" => crate::meeting::toggle(app),
                "help" => crate::help::open(app),
                "settings" => open_settings(app),
                "update" | "update-ready" => show_updates(app),
                "feedback" => send_feedback(app),
                "quit" if crate::meeting::is_running() => crate::meeting::quit(app),
                "quit" => {
                    splashdown_platform::restore_output();
                    app.exit(0);
                }
                "translate-settings" => open_settings(app),
                _ => {
                    if let Some(code) = id.strip_prefix("lang:") {
                        set_language(app, code);
                    } else if let Some(language) = id.strip_prefix("translate:") {
                        set_translate_to(app, language);
                    } else if let Some(preset) = id.strip_prefix("preset:")
                        && let Some(state) = app.try_state::<crate::state::AppState>()
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
        translate,
        menu,
        update_ready,
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

    /// Lists the languages to translate into, ticking the one in use.
    pub fn show_translate(&self, app: &AppHandle, settings: &Settings) {
        let result =
            clear(&self.translate).and_then(|()| fill_translate(app, &self.translate, settings));
        if let Err(e) = result {
            tracing::warn!("couldn't update the Translate into menu: {e}");
        }
    }

    /// Shows `line` under the status ("Update to v1.2.3…" while an update
    /// is available, "Updated to v1.2.3" after one installed itself), and
    /// takes it away when there's nothing to say.
    pub fn set_update(&self, line: Option<&str>) {
        let shown = self.menu.get("update-ready").is_some();
        let result = match line {
            Some(line) => self.update_ready.set_text(line).and_then(|()| {
                if shown {
                    Ok(())
                } else {
                    self.menu.insert(&self.update_ready, UPDATE_READY_AT)
                }
            }),
            None if shown => self.menu.remove(&self.update_ready),
            None => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!("couldn't update the updates menu item: {e}");
        }
    }
}

/// Saves the spoken language chosen in the menu bar and rebuilds the
/// engine with it.
fn set_language(app: &AppHandle, code: &str) {
    let language = (!code.is_empty()).then(|| code.to_string());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<crate::state::AppState>() else {
            return;
        };
        if let Err(e) = state.set_language(&app, language).await {
            tracing::warn!("couldn't change the language: {e}");
        }
    });
}

/// The Translate into menu's items: each language, the one in use ticked,
/// then where to choose them.
fn fill_translate(app: &AppHandle, menu: &Submenu<Wry>, settings: &Settings) -> tauri::Result<()> {
    let general = &settings.general;
    let current = general.translate_target();
    let targets: Vec<&str> = general
        .translate_targets
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    for language in &targets {
        menu.append(&CheckMenuItem::with_id(
            app,
            format!("translate:{language}"),
            language,
            true,
            current == Some(*language),
            None::<&str>,
        )?)?;
    }
    if !targets.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(
        app,
        "translate-settings",
        if targets.is_empty() {
            "Choose languages…"
        } else {
            "Change languages…"
        },
        true,
        None::<&str>,
    )?)?;
    // How to use it, where it's chosen.
    let letter = match settings.hotkeys.translate_key() {
        splashdown_core::gesture::PhysicalKey::Letter(c) => c.to_ascii_uppercase(),
        _ => 'T',
    };
    let hint = format!(
        "Hold {}, press {letter}",
        settings.hotkeys.dictation_key().label()
    );
    menu.append(&MenuItem::with_id(
        app,
        "translate-hint",
        hint,
        false,
        None::<&str>,
    )?)
}

/// Saves the language to translate into, picked in the menu bar.
fn set_translate_to(app: &AppHandle, language: &str) {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let language = language.to_string();
    if let Err(e) = state.update(app, |s| s.general.translate_to = Some(language)) {
        tracing::warn!("couldn't change the translation language: {e}");
    }
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
        (960.0, 750.0),
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
        (960.0, 750.0),
        true,
    );
}

/// Settings → About, where the update (or "Check now") is. Without a known
/// update, also starts a check so the page has something to say.
fn show_updates(app: &AppHandle) {
    open_settings_tab(app, "about");
    let has_update = app
        .try_state::<crate::state::AppState>()
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
        if let Some(state) = app.try_state::<crate::state::AppState>()
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
