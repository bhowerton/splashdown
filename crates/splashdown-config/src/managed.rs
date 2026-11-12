//! Settings an organization has set for its people, the way it sets any
//! other app's: a configuration profile on macOS (managed preferences for
//! `app.splashdown`) or Group Policy on Windows (the registry, under
//! `Software\Policies\Splashdown`). No server of ours: the computer's own
//! management delivers them, and Splashdown reads them.
//!
//! | Name | Kind | What it does |
//! |---|---|---|
//! | `LocalOnly` | true | Only models on this computer; nothing said is sent to a cloud provider |
//! | `KeepHistory` | false | No history of dictations, and no recordings kept with it |
//! | `SharedVocabulary` | text | The team's vocabulary file (`crate::shared`) |
//! | `Updates` | `off`, `notify` or `install` | Whether updates are looked for, and installed unasked |
//!
//! A managed setting replaces the user's own choice whenever settings are
//! read or saved, and Settings shows it as set by the organization. It
//! only ever narrows what the app does (`LocalOnly` can't be used to
//! force the cloud, nor `KeepHistory` to force a history). This is the
//! app honouring its organization's choices, not a lock against someone
//! who can replace the app.

use serde::Serialize;

use crate::Settings;

/// A managed value as the system stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Text(String),
}

/// What the organization chose about updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Updates {
    /// Never looked for: the organization installs them itself.
    Off,
    /// Looked for and offered.
    Notify,
    /// Installed when nothing is going on.
    Install,
}

/// What an organization has set. All unset on a computer nobody manages.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Managed {
    pub local_only: bool,
    pub no_history: bool,
    pub shared_vocabulary: Option<String>,
    pub updates: Option<Updates>,
}

impl Managed {
    /// What's managed, from a lookup of managed values by name. A value
    /// of the wrong kind, or one that would widen what the app does, is
    /// taken as not set.
    pub fn from(get: impl Fn(&str) -> Option<Value>) -> Self {
        let text = |name: &str| match get(name) {
            Some(Value::Text(t)) if !t.trim().is_empty() => Some(t.trim().to_string()),
            _ => None,
        };
        Self {
            local_only: get("LocalOnly") == Some(Value::Bool(true)),
            no_history: get("KeepHistory") == Some(Value::Bool(false)),
            shared_vocabulary: text("SharedVocabulary"),
            updates: text("Updates").and_then(|u| match u.to_ascii_lowercase().as_str() {
                "off" => Some(Updates::Off),
                "notify" => Some(Updates::Notify),
                "install" => Some(Updates::Install),
                _ => None,
            }),
        }
    }

    /// What this computer's management has set for Splashdown.
    pub fn current() -> Self {
        Self::from(system::value)
    }

    pub fn any(&self) -> bool {
        *self != Self::default()
    }

    /// Puts the managed choices in place of the user's own.
    pub fn apply(&self, settings: &mut Settings) {
        let general = &mut settings.general;
        if self.local_only {
            general.local_only = true;
        }
        if self.no_history {
            general.keep_history = None;
            general.keep_audio = None;
        }
        if let Some(file) = &self.shared_vocabulary {
            general.shared_vocabulary = Some(file.clone());
        }
        match self.updates {
            Some(Updates::Off) => {
                general.check_for_updates = Some(false);
                general.install_updates = None;
            }
            Some(Updates::Notify) => {
                general.check_for_updates = None;
                general.install_updates = None;
            }
            Some(Updates::Install) => {
                general.check_for_updates = None;
                general.install_updates = Some(true);
            }
            None => {}
        }
    }
}

/// Managed preferences as macOS keeps them: a property list per app, for
/// the whole computer and for each user, written by the system from a
/// configuration profile (only an administrator can write there).
#[cfg(any(target_os = "macos", test))]
mod plists {
    use std::path::{Path, PathBuf};

    // Only reading the lists needs it; the tests elsewhere use `files` alone.
    #[cfg(target_os = "macos")]
    use super::Value;

    /// The app's managed preferences in `root`: the computer's, then the
    /// user's. The computer's value is used where both set one.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn files(root: &Path, user: Option<&str>) -> Vec<PathBuf> {
        let name = "app.splashdown.plist";
        let mut files = vec![root.join(name)];
        if let Some(user) = user.filter(|u| !u.is_empty() && !u.contains('/')) {
            files.push(root.join(user).join(name));
        }
        files
    }

    #[cfg(target_os = "macos")]
    pub(super) fn value(files: &[PathBuf], name: &str) -> Option<Value> {
        files.iter().find_map(|file| {
            let list = plist::Value::from_file(file).ok()?;
            match list.as_dictionary()?.get(name)? {
                plist::Value::Boolean(b) => Some(Value::Bool(*b)),
                plist::Value::String(s) => Some(Value::Text(s.clone())),
                // Profiles made by hand often say 1 and 0.
                plist::Value::Integer(i) => i.as_signed().map(|i| Value::Bool(i != 0)),
                _ => None,
            }
        })
    }
}

#[cfg(target_os = "macos")]
mod system {
    use std::path::Path;

    use super::{Value, plists};

    pub(super) fn value(name: &str) -> Option<Value> {
        let user = std::env::var("USER").ok();
        let files = plists::files(Path::new("/Library/Managed Preferences"), user.as_deref());
        plists::value(&files, name)
    }
}

#[cfg(windows)]
mod system {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::PCWSTR;

    use super::Value;

    /// Where Group Policy puts an app's settings.
    const KEY: &str = r"Software\Policies\Splashdown";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn number(root: HKEY, name: &[u16]) -> Option<u32> {
        let key = wide(KEY);
        let mut data = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: NUL-terminated strings that outlive the call; `data` is
        // writable for the `size` bytes passed.
        let err = unsafe {
            RegGetValueW(
                root,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut data).cast()),
                Some(&mut size),
            )
        };
        (err == ERROR_SUCCESS).then_some(data)
    }

    fn text(root: HKEY, name: &[u16]) -> Option<String> {
        let key = wide(KEY);
        let mut buf = [0u16; 1024];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: NUL-terminated strings that outlive the call; `buf` is
        // writable for the `size` bytes passed.
        let err = unsafe {
            RegGetValueW(
                root,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if err != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    /// The computer's policy, then the user's.
    pub(super) fn value(name: &str) -> Option<Value> {
        let name = wide(name);
        [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
            .into_iter()
            .find_map(|root| {
                number(root, &name)
                    .map(|n| Value::Bool(n != 0))
                    .or_else(|| text(root, &name).map(Value::Text))
            })
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod system {
    pub(super) fn value(_name: &str) -> Option<super::Value> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed(values: &[(&str, Value)]) -> Managed {
        Managed::from(|name| {
            values
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        })
    }

    fn text(t: &str) -> Value {
        Value::Text(t.into())
    }

    #[test]
    fn nothing_is_managed_unless_set() {
        let none = managed(&[]);
        assert!(!none.any());
        let mut settings = Settings::default();
        settings.general.keep_history = Some(true);
        let before = settings.clone();
        none.apply(&mut settings);
        assert_eq!(settings, before);
    }

    #[test]
    fn managed_choices_replace_the_users_own() {
        let m = managed(&[
            ("LocalOnly", Value::Bool(true)),
            ("KeepHistory", Value::Bool(false)),
            ("SharedVocabulary", text(" /Volumes/Team/words.txt ")),
            ("Updates", text("Off")),
        ]);
        assert!(m.any());
        let mut settings = Settings::default();
        settings.general.keep_history = Some(true);
        settings.general.keep_audio = Some(true);
        settings.general.install_updates = Some(true);
        settings.general.shared_vocabulary = Some("~/mine.txt".into());
        m.apply(&mut settings);
        let g = &settings.general;
        assert!(g.local_only);
        assert!(!g.keep_history() && !g.keep_audio());
        assert_eq!(
            g.shared_vocabulary.as_deref(),
            Some("/Volumes/Team/words.txt")
        );
        assert!(!g.check_for_updates() && !g.install_updates());
    }

    #[test]
    fn a_managed_setting_only_narrows() {
        // Not a way to force the cloud, or a record of what's said.
        let m = managed(&[
            ("LocalOnly", Value::Bool(false)),
            ("KeepHistory", Value::Bool(true)),
            ("Updates", text("sometimes")),
            ("SharedVocabulary", Value::Bool(true)),
        ]);
        assert!(!m.any());
        let mut settings = Settings::default();
        settings.general.local_only = true;
        m.apply(&mut settings);
        assert!(settings.general.local_only);
        assert!(!settings.general.keep_history());
    }

    #[test]
    fn updates_can_be_made_to_install() {
        let m = managed(&[("Updates", text("install"))]);
        let mut settings = Settings::default();
        settings.general.check_for_updates = Some(false);
        m.apply(&mut settings);
        assert!(settings.general.install_updates());
    }

    #[test]
    fn the_computers_list_comes_before_the_users() {
        let root = std::path::Path::new("/managed");
        assert_eq!(
            plists::files(root, Some("priya")),
            [
                root.join("app.splashdown.plist"),
                root.join("priya").join("app.splashdown.plist")
            ]
        );
        assert_eq!(plists::files(root, Some("../etc")).len(), 1);
        assert_eq!(plists::files(root, None).len(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_a_profiles_property_list() {
        let dir = std::env::temp_dir().join(format!("sd-managed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("priya")).unwrap();
        let list = |body: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>{body}</dict></plist>"#
            )
        };
        std::fs::write(
            dir.join("app.splashdown.plist"),
            list("<key>LocalOnly</key><true/><key>Updates</key><string>off</string>"),
        )
        .unwrap();
        std::fs::write(
            dir.join("priya/app.splashdown.plist"),
            list("<key>LocalOnly</key><false/><key>KeepHistory</key><integer>0</integer>"),
        )
        .unwrap();
        let files = plists::files(&dir, Some("priya"));
        let m = Managed::from(|name| plists::value(&files, name));
        assert_eq!(
            m,
            Managed {
                local_only: true,
                no_history: true,
                shared_vocabulary: None,
                updates: Some(Updates::Off),
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
