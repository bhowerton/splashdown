//! Apps installed on this computer, by the names people know them by, for
//! voice actions ("open Spotify"): app bundles in the usual folders on
//! macOS; Start menu shortcuts and Store apps on Windows.

use std::path::{Path, PathBuf};

use crate::{PlatformError, Result};

/// An app that can be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    how: Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Open {
    /// An app bundle (macOS) or a Start menu shortcut (Windows).
    Path(PathBuf),
    /// A Store app's AppUserModelID (Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    AppId(String),
}

/// Every app found, one per name. Takes a moment on Windows (Store apps
/// are asked of PowerShell), so callers keep the list.
pub fn installed_apps() -> Vec<InstalledApp> {
    let mut apps = list();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

/// Opens `app`, or brings it to the front if it's already open.
pub fn open_app(app: &InstalledApp) -> Result<()> {
    let failed = |e: String| PlatformError::Failed(format!("couldn't open {}: {e}", app.name));
    match &app.how {
        #[cfg(target_os = "macos")]
        Open::Path(path) => std::process::Command::new("open")
            .arg("-a")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| failed(e.to_string())),
        // What a double-click on the shortcut does.
        #[cfg(not(target_os = "macos"))]
        Open::Path(path) => {
            crate::open_url(&path.to_string_lossy()).map_err(|e| failed(e.to_string()))
        }
        Open::AppId(id) => {
            crate::open_url(&format!("shell:AppsFolder\\{id}")).map_err(|e| failed(e.to_string()))
        }
    }
}

#[cfg(target_os = "macos")]
fn list() -> Vec<InstalledApp> {
    let mut dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::home_dir() {
        dirs.push(home.join("Applications"));
    }
    let mut apps = Vec::new();
    for dir in dirs {
        // Bundles at the top, and one folder down (Utilities, a vendor's
        // folder), but not inside other bundles.
        bundles(&dir, 2, &mut apps);
    }
    apps.push(bundle(Path::new("/System/Library/CoreServices/Finder.app")));
    apps
}

#[cfg(target_os = "macos")]
fn bundles(dir: &Path, depth: u32, out: &mut Vec<InstalledApp>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "app") {
            out.push(bundle(&path));
        } else if depth > 1 && path.is_dir() {
            bundles(&path, depth - 1, out);
        }
    }
}

#[cfg(target_os = "macos")]
fn bundle(path: &Path) -> InstalledApp {
    InstalledApp {
        name: stem(path),
        how: Open::Path(path.to_path_buf()),
    }
}

#[cfg(windows)]
fn list() -> Vec<InstalledApp> {
    let mut apps = Vec::new();
    for root in ["ProgramData", "APPDATA"] {
        // The Start menu for everyone, and the user's own.
        if let Some(base) = std::env::var_os(root) {
            let start = PathBuf::from(base).join("Microsoft\\Windows\\Start Menu\\Programs");
            shortcuts(&start, 4, &mut apps);
        }
    }
    apps.extend(store_apps());
    apps
}

#[cfg(windows)]
fn shortcuts(dir: &Path, depth: u32, out: &mut Vec<InstalledApp>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 1 {
                shortcuts(&path, depth - 1, out);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
        {
            let name = stem(&path);
            // Not the uninstallers and help files shortcuts often sit with.
            let lower = name.to_lowercase();
            if !["uninstall", "readme", "help", "documentation", "website"]
                .iter()
                .any(|w| lower.contains(w))
            {
                out.push(InstalledApp {
                    name,
                    how: Open::Path(path),
                });
            }
        }
    }
}

/// Store apps (Calculator, Photos…), which have no shortcut file: their
/// names and IDs from PowerShell's `Get-StartApps`.
#[cfg(windows)]
fn store_apps() -> Vec<InstalledApp> {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`: no console flashing up.
    const NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            // UTF-8 out: captured, Windows PowerShell otherwise writes in the
            // console's legacy code page, garbling "Paramètres" or 設定.
            "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); Get-StartApps | Where-Object { $_.AppID -like '*!*' } | ForEach-Object { $_.Name + \"`t\" + $_.AppID }",
        ])
        .creation_flags(NO_WINDOW)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, id) = line.trim().split_once('\t')?;
            Some(InstalledApp {
                name: name.trim().to_string(),
                how: Open::AppId(id.trim().to_string()),
            })
        })
        .collect()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn list() -> Vec<InstalledApp> {
    Vec::new()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn finds_the_apps_every_mac_has() {
        let apps = installed_apps();
        for name in ["Safari", "Finder", "Calculator", "Notes"] {
            assert!(apps.iter().any(|a| a.name == name), "{name} missing");
        }
    }
}
