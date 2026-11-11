//! An invite to a team's vocabulary: a link that says where the file is
//! in a way any teammate's computer can follow. A path like
//! `/home/priya/Dropbox/Team/words.txt` only means something on Priya's
//! computer, so the link names the sync service and the path inside it
//! ("Dropbox", "Team/words.txt"), and each computer looks in its own
//! folder for that service.
//!
//! The link is a page on the website with the two parts after its `#`,
//! which a browser never sends to the site. The page hands them to the
//! app as `splashdown://team-words?...`. Settings takes either form
//! pasted, for where the app can't be handed a link (Windows, today).
//!
//! An invite never gives a file's contents or reaches outside the
//! person's own sync folders, and joining always asks first.

use std::path::{Component, Path, PathBuf};

use crate::notes::SyncBase;
use crate::triggers::{decode, encode};

/// The page that hands an invite to the app.
const PAGE: &str = "https://splashdown.app/team/";
/// What the app's own link is called: `splashdown://team-words?...`.
pub const LINK_NAME: &str = "team-words";
/// A longer path than this isn't an invite.
const LONGEST_PATH: usize = 500;
/// How far below a service's folder a moved file is looked for, and how
/// many files and folders are looked at before giving up.
const SEARCH_DEPTH: usize = 3;
const SEARCH_ENTRIES: usize = 3000;

/// Where a team's file is: the sync service, and the path inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// "Dropbox", as `notes::sync_bases` names it.
    pub service: String,
    /// The path inside the service's folder, its parts joined by `/`.
    pub path: String,
}

/// Why an invite couldn't be followed here.
#[derive(Debug, PartialEq, Eq)]
pub enum InviteError {
    NotAnInvite,
    /// This computer has no folder for the service.
    NoService(String),
    NotFound {
        service: String,
        file: String,
    },
}

impl std::fmt::Display for InviteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnInvite => write!(f, "That isn't an invite to a team's words."),
            Self::NoService(service) => write!(
                f,
                "This list is kept in {service}, and this computer has no {service} folder. Set up {service} here, then open the invite again."
            ),
            Self::NotFound { service, file } => write!(
                f,
                "Couldn't find “{file}” in your {service}. Ask for its folder to be shared with you, let it sync, then open the invite again."
            ),
        }
    }
}

impl Invite {
    /// The invite for `file`, if it's inside one of the sync folders.
    pub fn for_file(file: &Path, bases: &[SyncBase]) -> Option<Self> {
        bases.iter().find_map(|base| {
            let inside = file.strip_prefix(&base.dir).ok()?;
            let mut parts: Vec<String> = base.inside.iter().cloned().collect();
            for part in inside.components() {
                match part {
                    Component::Normal(part) => parts.push(part.to_str()?.to_string()),
                    _ => return None,
                }
            }
            // The folder itself isn't a file in it.
            (parts.len() > base.inside.iter().count()).then(|| Self {
                service: base.service.clone(),
                path: parts.join("/"),
            })
        })
    }

    /// The link to give teammates.
    pub fn link(&self) -> String {
        format!(
            "{PAGE}#in={}&file={}",
            encode(&self.service),
            encode(&self.path)
        )
    }

    /// The file's name, for messages.
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// Reads an invite from the website's link or the app's own. `None`
    /// for anything else, or for a path that isn't plainly inside a
    /// folder: absolute, with `..`, or with a drive or a backslash in it.
    pub fn parse(link: &str) -> Option<Self> {
        let link = link.trim();
        let query = if let Some(rest) = link.strip_prefix(PAGE) {
            rest.strip_prefix('#')?
        } else {
            let (scheme, rest) = link.split_once(':')?;
            if !scheme.eq_ignore_ascii_case("splashdown") {
                return None;
            }
            let (name, query) = rest.trim_start_matches('/').split_once('?')?;
            if !name.trim_end_matches('/').eq_ignore_ascii_case(LINK_NAME) {
                return None;
            }
            query
        };
        let part = |name: &str| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
                .map(decode)
        };
        let service = part("in")?.trim().to_string();
        let path = part("file")?;
        let plain = |piece: &str| {
            !piece.is_empty()
                && piece != "."
                && piece != ".."
                && !piece.contains(['\\', ':'])
                && !piece.chars().any(char::is_control)
        };
        let usable = !service.is_empty()
            && service.chars().all(|c| c.is_alphanumeric() || c == ' ')
            && path.len() <= LONGEST_PATH
            && path.split('/').all(plain);
        usable.then_some(Self { service, path })
    }

    /// The file on this computer: at the same path inside one of the
    /// service's folders or, if it isn't there (a shared folder can sit
    /// somewhere else in each person's drive), a file of the same name a
    /// few folders down.
    pub fn find(&self, bases: &[SyncBase]) -> Result<PathBuf, InviteError> {
        let here: Vec<&SyncBase> = bases
            .iter()
            .filter(|base| base.service.eq_ignore_ascii_case(&self.service))
            .collect();
        if here.is_empty() {
            return Err(InviteError::NoService(self.service.clone()));
        }
        let same_place = here.iter().find_map(|base| {
            // A folder that is one part of the service holds only the
            // paths that start with its name.
            let path = match &base.inside {
                Some(name) => self.path.strip_prefix(name.as_str())?.strip_prefix('/')?,
                None => self.path.as_str(),
            };
            let file = path
                .split('/')
                .fold(base.dir.clone(), |p, part| p.join(part));
            file.is_file().then_some(file)
        });
        same_place
            .or_else(|| {
                here.iter()
                    .find_map(|base| search(&base.dir, self.file_name()))
            })
            .ok_or_else(|| InviteError::NotFound {
                service: self.service.clone(),
                file: self.file_name().to_string(),
            })
    }
}

/// A file called `name` in `base` or a few folders down, nearest first.
fn search(base: &Path, name: &str) -> Option<PathBuf> {
    let mut looked_at = 0;
    let mut level = vec![base.to_path_buf()];
    for _ in 0..=SEARCH_DEPTH {
        let mut next = Vec::new();
        for dir in &level {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                looked_at += 1;
                if looked_at > SEARCH_ENTRIES {
                    return None;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_file() && entry.file_name().to_str() == Some(name) {
                    return Some(entry.path());
                }
                // Not through links, and not into hidden folders.
                if kind.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
                    next.push(entry.path());
                }
            }
        }
        level = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sd-invite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn base(service: &str, dir: impl Into<PathBuf>) -> SyncBase {
        SyncBase {
            service: service.into(),
            dir: dir.into(),
            inside: None,
        }
    }

    fn invite(service: &str, path: &str) -> Invite {
        Invite {
            service: service.into(),
            path: path.into(),
        }
    }

    #[test]
    fn a_file_in_a_sync_folder_has_an_invite() {
        let bases = vec![
            base("Dropbox", "/home/priya/Dropbox"),
            base("Google Drive", "/home/priya/GDrive"),
        ];
        let made = Invite::for_file(
            Path::new("/home/priya/GDrive/Shared drives/Acme/Team words.txt"),
            &bases,
        )
        .unwrap();
        assert_eq!(
            made,
            invite("Google Drive", "Shared drives/Acme/Team words.txt")
        );
        assert_eq!(
            made.link(),
            "https://splashdown.app/team/#in=Google%20Drive&file=Shared%20drives%2FAcme%2FTeam%20words.txt"
        );
        // Not in a sync folder: nothing a teammate's computer could follow.
        assert_eq!(
            Invite::for_file(Path::new("/home/priya/Documents/words.txt"), &bases),
            None
        );
        assert_eq!(
            Invite::for_file(Path::new("/home/priya/Dropbox"), &bases),
            None
        );
    }

    #[test]
    fn both_kinds_of_link_are_read() {
        let made = invite("Google Drive", "Shared drives/Acme/Team words.txt");
        assert_eq!(Invite::parse(&made.link()), Some(made.clone()));
        assert_eq!(
            Invite::parse(
                "splashdown://team-words?in=Google%20Drive&file=Shared%20drives%2FAcme%2FTeam%20words.txt"
            ),
            Some(made)
        );
        assert_eq!(
            Invite::parse("https://example.com/team/#in=Dropbox&file=a.txt"),
            None
        );
        assert_eq!(Invite::parse("splashdown://toggle"), None);
        assert_eq!(Invite::parse("splashdown://team-words?in=Dropbox"), None);
    }

    #[test]
    fn a_link_cant_point_outside_the_folder() {
        for path in [
            "..%2F..%2F.ssh%2Fid_rsa",
            "%2Fetc%2Fpasswd",
            "Team%2F..%2F..%2Fsecret.txt",
            "C%3A%5CUsers%5Csam%5Csecret.txt",
            "Team%5C..%5Csecret.txt",
            "",
        ] {
            let link = format!("splashdown://team-words?in=Dropbox&file={path}");
            assert_eq!(Invite::parse(&link), None, "{path}");
        }
        assert_eq!(
            Invite::parse("splashdown://team-words?in=..%2F&file=a.txt"),
            None
        );
    }

    #[test]
    fn the_file_is_found_where_the_link_says() {
        let dropbox = folder("same");
        std::fs::create_dir_all(dropbox.join("Team")).unwrap();
        std::fs::write(dropbox.join("Team/words.txt"), "Priya\n").unwrap();
        let bases = vec![base("Dropbox", &dropbox)];
        assert_eq!(
            invite("dropbox", "Team/words.txt").find(&bases),
            Ok(dropbox.join("Team/words.txt"))
        );
        assert_eq!(
            invite("Box", "Team/words.txt").find(&bases),
            Err(InviteError::NoService("Box".into()))
        );
        assert!(matches!(
            invite("Dropbox", "Team/other.txt").find(&bases),
            Err(InviteError::NotFound { .. })
        ));
    }

    /// Google Drive can keep My Drive in the home folder, apart from its
    /// Shared drives: the link reads the same either way.
    #[test]
    fn my_drive_kept_in_the_home_folder_gives_the_same_link() {
        let home = folder("mirror");
        let my_drive = home.join("My Drive");
        let own = home.join("CloudStorage");
        std::fs::create_dir_all(my_drive.join("Team")).unwrap();
        std::fs::create_dir_all(own.join("Shared drives/Acme")).unwrap();
        std::fs::write(my_drive.join("Team/words.txt"), "Priya\n").unwrap();
        std::fs::write(own.join("Shared drives/Acme/words.txt"), "Priya\n").unwrap();
        let mirrored = vec![
            SyncBase {
                service: "Google Drive".into(),
                dir: my_drive.clone(),
                inside: Some("My Drive".into()),
            },
            base("Google Drive", &own),
        ];
        let mine = Invite::for_file(&my_drive.join("Team/words.txt"), &mirrored).unwrap();
        assert_eq!(mine, invite("Google Drive", "My Drive/Team/words.txt"));
        assert_eq!(mine.find(&mirrored), Ok(my_drive.join("Team/words.txt")));
        let shared = Invite::for_file(&own.join("Shared drives/Acme/words.txt"), &mirrored);
        let shared = shared.unwrap();
        assert_eq!(
            shared,
            invite("Google Drive", "Shared drives/Acme/words.txt")
        );
        assert_eq!(
            shared.find(&mirrored),
            Ok(own.join("Shared drives/Acme/words.txt"))
        );
        // The folder itself isn't a list, and a file beside it isn't in it.
        assert_eq!(Invite::for_file(&my_drive, &mirrored), None);
        assert_eq!(Invite::for_file(&home.join("notes.txt"), &mirrored), None);

        // Someone whose My Drive is with the rest follows the same link.
        std::fs::create_dir_all(own.join("My Drive/Team")).unwrap();
        std::fs::write(own.join("My Drive/Team/words.txt"), "Priya\n").unwrap();
        let streamed = vec![base("Google Drive", &own)];
        assert_eq!(
            mine.find(&streamed),
            Ok(own.join("My Drive/Team/words.txt"))
        );
    }

    #[test]
    fn a_shared_folder_kept_somewhere_else_is_still_found() {
        let dropbox = folder("moved");
        std::fs::create_dir_all(dropbox.join("Work/Shared with me/Team")).unwrap();
        std::fs::write(
            dropbox.join("Work/Shared with me/Team/words.txt"),
            "Priya\n",
        )
        .unwrap();
        // Too deep to be looked for.
        std::fs::create_dir_all(dropbox.join("a/b/c/d/e")).unwrap();
        std::fs::write(dropbox.join("a/b/c/d/e/deep.txt"), "x\n").unwrap();
        let bases = vec![base("Dropbox", &dropbox)];
        assert_eq!(
            invite("Dropbox", "Team/words.txt").find(&bases),
            Ok(dropbox.join("Work/Shared with me/Team/words.txt"))
        );
        assert!(invite("Dropbox", "Team/deep.txt").find(&bases).is_err());
    }
}
