//! API keys. Stored in the OS keychain (macOS Keychain, Windows Credential
//! Manager). Environment variables override the keychain, which keeps
//! development and the CLI working with a `.env.local` file.

use serde::{Deserialize, Serialize};

const KEYCHAIN_SERVICE: &str = "Splashdown";
/// The keychain service before the app was renamed to Splashdown.
const LEGACY_KEYCHAIN_SERVICE: &str = "just-say-it";

/// Which key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyName {
    Openai,
    Openrouter,
    /// Claude, for the AI pass only (Anthropic has no speech-to-text).
    Anthropic,
    /// Optional key for a custom OpenAI-compatible server.
    Custom,
}

impl KeyName {
    pub const ALL: [KeyName; 4] = [
        KeyName::Openai,
        KeyName::Openrouter,
        KeyName::Anthropic,
        KeyName::Custom,
    ];

    pub fn env_var(self) -> &'static str {
        match self {
            KeyName::Openai => "OPENAI_API_KEY",
            KeyName::Openrouter => "OPENROUTER_API_KEY",
            KeyName::Anthropic => "ANTHROPIC_API_KEY",
            KeyName::Custom => "SPLASHDOWN_CUSTOM_API_KEY",
        }
    }

    fn account(self) -> &'static str {
        match self {
            KeyName::Openai => "openai",
            KeyName::Openrouter => "openrouter",
            KeyName::Anthropic => "anthropic",
            KeyName::Custom => "custom",
        }
    }
}

/// Where a key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeySource {
    Environment,
    Keychain,
}

#[derive(Clone, Default)]
pub struct Secrets {
    pub openai: Option<String>,
    pub openrouter: Option<String>,
    pub anthropic: Option<String>,
    pub custom: Option<String>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mark = |k: &Option<String>| if k.is_some() { "set" } else { "unset" };
        f.debug_struct("Secrets")
            .field("openai", &mark(&self.openai))
            .field("openrouter", &mark(&self.openrouter))
            .field("anthropic", &mark(&self.anthropic))
            .field("custom", &mark(&self.custom))
            .finish()
    }
}

impl Secrets {
    /// Keys from environment variables only (`OPENAI_API_KEY`,
    /// `OPENROUTER_API_KEY`, `SPLASHDOWN_CUSTOM_API_KEY`).
    pub fn from_env() -> Self {
        let mut s = Self::default();
        for name in KeyName::ALL {
            *s.slot(name) = env_key(name);
        }
        s
    }

    /// Environment variables, falling back to the keychain.
    pub fn load() -> Self {
        let mut s = Self::from_env();
        for name in KeyName::ALL {
            let slot = s.slot(name);
            if slot.is_none() {
                *slot = keychain_key(name);
            }
        }
        s
    }

    pub fn get(&self, name: KeyName) -> Option<&str> {
        match name {
            KeyName::Openai => self.openai.as_deref(),
            KeyName::Openrouter => self.openrouter.as_deref(),
            KeyName::Anthropic => self.anthropic.as_deref(),
            KeyName::Custom => self.custom.as_deref(),
        }
    }

    fn slot(&mut self, name: KeyName) -> &mut Option<String> {
        match name {
            KeyName::Openai => &mut self.openai,
            KeyName::Openrouter => &mut self.openrouter,
            KeyName::Anthropic => &mut self.anthropic,
            KeyName::Custom => &mut self.custom,
        }
    }
}

/// Where the key in effect comes from, if any.
pub fn key_source(name: KeyName) -> Option<KeySource> {
    if env_key(name).is_some() {
        Some(KeySource::Environment)
    } else if keychain_key(name).is_some() {
        Some(KeySource::Keychain)
    } else {
        None
    }
}

/// Saves a key to the keychain.
pub fn store_key(name: KeyName, value: &str) -> Result<(), String> {
    entry(name)?
        .set_password(value.trim())
        .map_err(|e| e.to_string())
}

/// Removes a key from the keychain. Removing a key that isn't there is fine.
pub fn delete_key(name: KeyName) -> Result<(), String> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Moves keys saved under the app's former name to today's keychain
/// entries (only where none is saved yet). Returns how many moved.
pub fn migrate_legacy_keys() -> usize {
    KeyName::ALL
        .into_iter()
        .filter(|&name| keychain_key(name).is_none())
        .filter(|&name| {
            let Ok(old) = keyring::Entry::new(LEGACY_KEYCHAIN_SERVICE, name.account()) else {
                return false;
            };
            let Ok(key) = old.get_password() else {
                return false;
            };
            let stored = store_key(name, &key).is_ok();
            if stored {
                let _ = old.delete_credential();
            }
            stored
        })
        .count()
}

fn entry(name: KeyName) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, name.account()).map_err(|e| e.to_string())
}

fn env_key(name: KeyName) -> Option<String> {
    std::env::var(name.env_var())
        .ok()
        .filter(|v| !v.trim().is_empty())
}

fn keychain_key(name: KeyName) -> Option<String> {
    entry(name)
        .ok()?
        .get_password()
        .ok()
        .filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_shows_keys() {
        let s = Secrets {
            openai: Some("sk-very-secret".into()),
            openrouter: None,
            anthropic: None,
            custom: Some("another-secret".into()),
        };
        let shown = format!("{s:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("openai: \"set\""));
        assert!(shown.contains("openrouter: \"unset\""));
    }

    #[test]
    fn each_key_has_its_own_slot_and_names() {
        let mut s = Secrets::default();
        for (i, name) in KeyName::ALL.into_iter().enumerate() {
            *s.slot(name) = Some(i.to_string());
        }
        for (i, name) in KeyName::ALL.into_iter().enumerate() {
            assert_eq!(s.get(name), Some(i.to_string().as_str()));
        }
        let vars: std::collections::HashSet<_> = KeyName::ALL.map(KeyName::env_var).into();
        let accounts: std::collections::HashSet<_> = KeyName::ALL.map(KeyName::account).into();
        assert_eq!(vars.len(), KeyName::ALL.len());
        assert_eq!(accounts.len(), KeyName::ALL.len());
    }
}
