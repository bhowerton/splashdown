//! A team's vocabulary file in the running app (`splashdown_config::shared`):
//! its words go into the engine with the user's own, Settings chooses the
//! file and shows what it gave, and a change to the file (a teammate added
//! a name, and the folder synced) is picked up without a restart.

mod invite;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use splashdown_config::Settings;
use splashdown_config::shared::{self, SharedWords};
use splashdown_core::sync::Locked;
use tauri::{AppHandle, Manager, State};

use self::invite::{Invite, InviteError};
use crate::state::AppState;

/// What a team's list is called when Splashdown makes it.
const FILE_NAME: &str = "Splashdown team words.txt";

/// How often the file is looked at for a change.
const TICK: Duration = Duration::from_secs(30);

/// The file the engine was last built with, and when it was last changed
/// and how long it was then (`None` if it wasn't there).
type Stamp = Option<(SystemTime, u64)>;
static BUILT_WITH: Mutex<Option<(PathBuf, Stamp)>> = Mutex::new(None);

fn stamp(file: &Path) -> Stamp {
    let meta = std::fs::metadata(file).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The team's words for an engine being built, remembering the file as it
/// is now. Nothing when no file is chosen or it can't be used.
pub fn load(settings: &Settings) -> SharedWords {
    let file = shared::file(settings);
    *BUILT_WITH.locked() = file.as_ref().map(|f| (f.clone(), stamp(f)));
    match shared::load(settings) {
        Ok(words) => {
            if !words.is_empty() {
                tracing::info!(
                    "shared vocabulary: {} word(s), {} correction(s)",
                    words.vocabulary.len(),
                    words.replacements.len()
                );
            }
            words
        }
        Err(e) => {
            tracing::warn!("the shared vocabulary isn't used: {e}");
            SharedWords::default()
        }
    }
}

/// Rebuilds the engine when the file has changed since it was built.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("splashdown-shared-words".into())
        .spawn(move || {
            loop {
                std::thread::sleep(TICK);
                let changed = BUILT_WITH
                    .locked()
                    .as_ref()
                    .is_some_and(|(file, was)| stamp(file) != *was);
                if changed {
                    tracing::info!("the shared vocabulary changed: using the new one");
                    crate::state::rebuild_in_background(&app);
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("couldn't watch the shared vocabulary: {e}");
    }
}

/// What Settings shows about the team's file.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Status {
    /// The file chosen, as it's saved.
    pub file: Option<String>,
    pub words: usize,
    pub corrections: usize,
    /// Entries not taken: past the limits, too long, or not a word.
    pub left_out: usize,
    /// Why the file isn't being used.
    pub problem: Option<String>,
    /// The link that lets a teammate use the same file, when it's in a
    /// sync folder their computer could find it in.
    pub invite: Option<String>,
}

fn status_of(settings: &Settings) -> Status {
    let Some(path) = shared::file(settings) else {
        return Status::default();
    };
    let file = settings.general.shared_vocabulary.clone();
    let invite = Invite::for_file(&path, &crate::notes::sync_bases()).map(|i| i.link());
    match shared::read(&path) {
        Ok(words) => Status {
            file,
            words: words.vocabulary.len(),
            corrections: words.replacements.len(),
            left_out: words.left_out,
            problem: None,
            invite,
        },
        Err(e) => Status {
            file,
            problem: Some(e.to_string()),
            invite,
            ..Default::default()
        },
    }
}

#[tauri::command]
pub fn shared_words_status(state: State<'_, AppState>) -> Status {
    status_of(&state.settings())
}

/// A native file picker for the team's file. `None` if cancelled.
#[tauri::command]
pub async fn shared_words_pick(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose your team's vocabulary file")
            .add_filter("Word lists", &["txt", "csv", "tsv", "json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// The organization chose the file: it isn't the user's to change.
fn managed() -> Result<(), String> {
    match splashdown_config::managed::Managed::current().shared_vocabulary {
        Some(_) => Err("Your organization chose this file.".into()),
        None => Ok(()),
    }
}

/// What `file` gives, as settings that use it would see it.
fn status_of_file(file: &str) -> Status {
    let mut trial = Settings::default();
    trial.general.shared_vocabulary = Some(file.to_string());
    status_of(&trial)
}

/// Uses `file` as the team's vocabulary from now on (`None` stops using
/// one). A file that can't be read isn't taken.
async fn use_file(
    app: &AppHandle,
    state: &AppState,
    file: Option<String>,
) -> Result<Status, String> {
    managed()?;
    let file = file.map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
    if let Some(file) = &file
        && let Some(problem) = status_of_file(file).problem
    {
        return Err(format!("That file can't be used: {problem}."));
    }
    let settings = state.update(app, |s| s.general.shared_vocabulary = file)?;
    if let Some(e) = state.apply().await {
        tracing::warn!("after changing the shared vocabulary: {e}");
    }
    Ok(status_of(&settings))
}

#[tauri::command]
pub async fn shared_words_set(
    app: AppHandle,
    state: State<'_, AppState>,
    file: Option<String>,
) -> Result<Status, String> {
    use_file(&app, &state, file).await
}

/// The file an invite means on this computer.
fn locate(link: &str) -> Result<(Invite, PathBuf), String> {
    let invite = Invite::parse(link).ok_or_else(|| InviteError::NotAnInvite.to_string())?;
    let file = invite
        .find(&crate::notes::sync_bases())
        .map_err(|e| e.to_string())?;
    Ok((invite, file))
}

/// Joins a team's list from an invite link pasted into Settings.
#[tauri::command]
pub async fn shared_words_join(
    app: AppHandle,
    state: State<'_, AppState>,
    link: String,
) -> Result<Status, String> {
    let (_, file) = locate(&link)?;
    use_file(&app, &state, Some(file.display().to_string())).await
}

/// Whether `url` is the app's own link for an invite (well-formed or not).
pub fn is_invite_link(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, rest)| {
        scheme.eq_ignore_ascii_case("splashdown")
            && rest
                .trim_start_matches('/')
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(invite::LINK_NAME))
    })
}

fn count(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// An invite link was opened: says what it would add and asks before
/// using it. A link can come from anywhere, so nothing changes without a
/// yes. Returns at once; the question is asked on a thread of its own.
pub fn open_invite(app: &AppHandle, link: &str) {
    let app = app.clone();
    let link = link.to_string();
    std::thread::spawn(move || {
        let tell = |title: &str, text: &str| {
            crate::prompt::ask(&app, title, text, &["OK"]);
        };
        const COULDNT: &str = "Couldn't use your team's words";
        if let Err(e) = managed() {
            return tell(COULDNT, &e);
        }
        let (invite, file) = match locate(&link) {
            Ok(found) => found,
            Err(e) => return tell(COULDNT, &e),
        };
        let file = file.display().to_string();
        let status = status_of_file(&file);
        if let Some(problem) = status.problem {
            return tell(COULDNT, &format!("That list can't be used: {problem}."));
        }
        let state = app.state::<AppState>();
        let now = state.settings().general.shared_vocabulary;
        if now.as_deref() == Some(file.as_str()) {
            return tell("Your team's words", "You're already using this list.");
        }
        let text = format!(
            "{} › {}: {} and {}. They're used along with your own, and whoever can edit that file can change them.{}",
            invite.service,
            invite.path.replace('/', " › "),
            count(status.words, "word"),
            count(status.corrections, "correction"),
            if now.is_some() {
                " This takes the place of the list you use now."
            } else {
                ""
            }
        );
        if crate::prompt::ask(
            &app,
            "Use your team's words?",
            &text,
            &["Use them", "Not now"],
        ) != Some(0)
        {
            return;
        }
        match tauri::async_runtime::block_on(use_file(&app, &state, Some(file))) {
            Ok(_) => crate::tray::open_settings_tab(&app, "words"),
            Err(e) => tell(COULDNT, &e),
        }
    });
}

/// A native folder picker for where a new team list goes. `None` if
/// cancelled.
#[tauri::command]
pub async fn shared_words_pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a folder your team shares")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

/// What a new list starts with: how to write it, as notes, and the
/// user's own words and plain corrections if they asked for that.
fn starter(settings: &Settings, with_mine: bool) -> String {
    let mut text = String::from(
        "# Splashdown team words: names and terms for everyone's dictation to spell right.\n\
         # One per line. For a correction, write what gets written by mistake, a comma,\n\
         # then what it should be, like this:\n\
         # pria,Priya\n\
         # Lines that start with \"# \" like these are notes, not words.\n",
    );
    if with_mine {
        // A comma would make a word a correction, and a correction two.
        let plain = |t: &str| !t.trim().is_empty() && !t.contains([',', '\n', '\r']);
        for word in settings.general.vocabulary.iter().filter(|w| plain(w)) {
            text.push_str(word.trim());
            text.push('\n');
        }
        for rule in &settings.replacements {
            if !rule.regex && plain(&rule.find) && plain(&rule.replace) {
                text.push_str(&format!("{},{}\n", rule.find.trim(), rule.replace.trim()));
            }
        }
    }
    text
}

/// A list just started, or found already there.
#[derive(Debug, Serialize)]
pub struct Started {
    pub status: Status,
    /// The folder already had a list: it's used as it is.
    pub existed: bool,
}

/// Starts a team list in `folder` and uses it. A list already there is
/// used, not written over.
#[tauri::command]
pub async fn shared_words_start(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    with_mine: bool,
) -> Result<Started, String> {
    managed()?;
    let file = Path::new(&folder).join(FILE_NAME);
    let existed = file.exists();
    if !existed {
        std::fs::write(&file, starter(&state.settings(), with_mine))
            .map_err(|e| format!("Couldn't make the list there: {e}."))?;
    }
    let status = use_file(&app, &state, Some(file.display().to_string())).await?;
    Ok(Started { status, existed })
}

/// Opens the team's list in a text editor.
#[tauri::command]
pub fn shared_words_edit(state: State<'_, AppState>) -> Result<(), String> {
    let file = shared::file(&state.settings()).ok_or("No team list is in use.")?;
    #[cfg(target_os = "macos")]
    let editor = std::process::Command::new("/usr/bin/open")
        .arg("-t")
        .arg(&file)
        .spawn();
    #[cfg(windows)]
    let editor = std::process::Command::new("notepad.exe").arg(&file).spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let editor: std::io::Result<()> = {
        let _ = file;
        Err(std::io::Error::other("no text editor is known here"))
    };
    editor
        .map(|_| ())
        .map_err(|e| format!("Couldn't open it: {e}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of this test's own, emptied first.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sd-shared-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn status_says_what_the_file_gave_or_why_not() {
        assert_eq!(status_of(&Settings::default()), Status::default());

        let file = folder("status").join("team.txt");
        std::fs::write(&file, "Priya\nTauri\npria,Priya\n").unwrap();
        let mut settings = Settings::default();
        settings.general.shared_vocabulary = Some(file.display().to_string());
        let status = status_of(&settings);
        assert_eq!((status.words, status.corrections), (2, 1));
        assert_eq!(status.problem, None);

        std::fs::remove_file(&file).unwrap();
        let status = status_of(&settings);
        assert_eq!(status.words, 0);
        assert!(status.problem.unwrap().contains("isn't there"));
    }

    #[test]
    fn a_new_list_explains_itself_and_reads_as_empty() {
        let file = folder("starter").join(FILE_NAME);
        std::fs::write(&file, starter(&Settings::default(), true)).unwrap();
        let status = status_of_file(&file.display().to_string());
        assert_eq!(
            (status.words, status.corrections, status.left_out),
            (0, 0, 0)
        );
        assert_eq!(status.problem, None);
    }

    #[test]
    fn a_new_list_can_start_with_the_users_own_words() {
        let mut settings = Settings::default();
        settings.general.vocabulary = vec!["Priya".into(), "Smith, Jones & Co".into()];
        settings.replacements = vec![
            splashdown_config::Replacement {
                find: "pria".into(),
                replace: "Priya".into(),
                regex: false,
            },
            splashdown_config::Replacement {
                find: r"\bfoo+\b".into(),
                replace: "bar".into(),
                regex: true,
            },
        ];
        let file = folder("mine").join(FILE_NAME);
        std::fs::write(&file, starter(&settings, true)).unwrap();
        let status = status_of_file(&file.display().to_string());
        // The name with a comma and the pattern stay the user's own.
        assert_eq!((status.words, status.corrections), (1, 1));
        assert!(!starter(&settings, false).lines().any(|l| l == "Priya"));
    }

    #[test]
    fn only_the_apps_invite_links_are_invites() {
        assert!(is_invite_link(
            "splashdown://team-words?in=Dropbox&file=a.txt"
        ));
        assert!(is_invite_link("SPLASHDOWN:team-words"));
        assert!(!is_invite_link("splashdown://toggle"));
        assert!(!is_invite_link(
            "https://splashdown.app/team/#in=Dropbox&file=a.txt"
        ));
    }

    #[test]
    fn a_changed_file_has_another_stamp() {
        let file = folder("stamp").join("team.txt");
        assert_eq!(stamp(&file), None);
        std::fs::write(&file, "Priya\n").unwrap();
        let first = stamp(&file);
        assert!(first.is_some());
        std::fs::write(&file, "Priya\nTauri\n").unwrap();
        assert_ne!(stamp(&file), first);
    }
}
