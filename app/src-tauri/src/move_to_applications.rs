//! macOS: an app opened straight from its disk image, or from Downloads or
//! the Desktop, runs from a place it shouldn't live. From a disk image it
//! can't update itself, and once the image is ejected it's gone; opened
//! where it was downloaded, macOS may run it from a hidden read-only copy
//! instead. Starting at login and the permissions the user grants are tied
//! to where the app is, so they break when it moves later. So the first
//! thing the app does there is offer to move itself to the Applications
//! folder, before the setup guide or any permission prompt.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The bundle's name in the Applications folder, whatever the copy being
/// moved is called ("Splashdown 2.app" from a second download).
const APP_NAME: &str = "Splashdown.app";

/// A place the app shouldn't run from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Straight from the opened disk image.
    DiskImage,
    /// The hidden read-only copy macOS runs a downloaded app from
    /// ("app translocation") until it's moved.
    Translocated,
    Downloads,
    Desktop,
}

impl Place {
    /// Someone may keep the app in Downloads or on the Desktop on purpose:
    /// there they can say not to be asked again. A disk image or a hidden
    /// copy is never a place to stay.
    pub fn may_stay(self) -> bool {
        matches!(self, Place::Downloads | Place::Desktop)
    }

    fn running_from(self) -> &'static str {
        match self {
            Place::DiskImage => "It's running from the disk image you opened.",
            Place::Translocated => "It's running from a temporary copy that macOS made.",
            Place::Downloads => "It's running from your Downloads folder.",
            Place::Desktop => "It's running from your Desktop.",
        }
    }
}

/// The app bundle that `exe` (`<bundle>.app/Contents/MacOS/<name>`) is
/// part of. `None` for a program run on its own, as `cargo run` does.
pub fn bundle_of(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then_some(bundle)
}

/// Whether to offer the move for the app bundle at `bundle`, and why.
/// `home` is the user's home folder; `read_only` says the bundle's disk
/// can't be written to, as a disk image's can't. Anywhere not named here
/// (the Applications folders, a developer's build, a drive someone keeps
/// apps on) is left alone.
pub fn offer_from(bundle: &Path, home: &Path, read_only: bool) -> Option<Place> {
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Some(Place::Translocated);
    }
    // A build folder: a developer runs the app from there on purpose.
    if bundle.components().any(|c| c.as_os_str() == "target") {
        return None;
    }
    if let Ok(on_disk) = bundle.strip_prefix("/Volumes") {
        // A disk image holds the app at its top: /Volumes/Splashdown/
        // Splashdown.app. Deeper down, only on a disk that can't be
        // written to, not on a drive someone keeps their apps on.
        let at_top = on_disk.components().count() == 2;
        return (at_top || read_only).then_some(Place::DiskImage);
    }
    let in_home = bundle.strip_prefix(home).ok()?;
    match in_home.components().next()?.as_os_str().to_str()? {
        "Downloads" => Some(Place::Downloads),
        "Desktop" => Some(Place::Desktop),
        _ => None,
    }
}

/// Where the app goes: the Applications folder everyone shares, or the
/// user's own (`~/Applications`) when the shared one can't be written to
/// (a standard account).
pub fn destination(home: &Path, shared_writable: bool) -> PathBuf {
    let folder = if shared_writable {
        PathBuf::from("/Applications")
    } else {
        home.join("Applications")
    };
    folder.join(APP_NAME)
}

/// The question, its explanation, and the buttons: move (the default),
/// not now (also Escape), and for a place the user may keep it, don't ask
/// again.
pub fn question(place: Place) -> (&'static str, String, Vec<&'static str>) {
    let text = format!(
        "{} In your Applications folder, Splashdown can update itself and start when you log in.",
        place.running_from()
    );
    let mut buttons = vec!["Move to Applications", "Not Now"];
    if place.may_stay() {
        buttons.push("Don't Ask Again");
    }
    (
        "Move Splashdown to your Applications folder?",
        text,
        buttons,
    )
}

/// Offers the move when this copy runs from a place it shouldn't. True if
/// the copy in the Applications folder is about to start and this one must
/// quit at once; false to carry on (no offer, "not now", or the move
/// failed, which the user is told).
pub fn offer() -> bool {
    let Some(exe) = std::env::current_exe().ok() else {
        return false;
    };
    let (Some(bundle), Some(home)) = (bundle_of(&exe), std::env::home_dir()) else {
        return false;
    };
    let Some(place) = offer_from(bundle, &home, read_only(bundle)) else {
        return false;
    };
    if place.may_stay() && declined() {
        return false;
    }
    let (title, text, buttons) = question(place);
    match splashdown_platform::alert(title, &text, &buttons) {
        Some(0) => {}
        Some(2) => {
            remember_declined();
            return false;
        }
        _ => return false,
    }
    let to = destination(&home, writable(Path::new("/Applications")));
    match move_app(bundle, &to) {
        Ok(()) => {
            tracing::info!("moved the app to {}; starting it there", to.display());
            // The copy the user downloaded would offer this again: it goes
            // to the Trash, where it can be got back. A disk image's and
            // macOS's hidden copy can't be changed, and needn't be.
            if place.may_stay()
                && let Err(e) = trash(bundle)
            {
                tracing::warn!("couldn't move the downloaded copy to the Trash: {e}");
            }
            true
        }
        Err(why) => {
            tracing::warn!("couldn't move the app to {}: {why}", to.display());
            splashdown_platform::alert(
                "Splashdown couldn't move itself",
                &format!("{why}\n\nIt will keep running from where it is now."),
                &["OK"],
            );
            false
        }
    }
}

/// Copies the bundle to `to`, replacing an older copy there, and starts it
/// once this process has quit. On an error, says what went wrong in plain
/// words; whatever was there before is still there or in the Trash.
fn move_app(bundle: &Path, to: &Path) -> Result<(), String> {
    let folder = to.parent().ok_or("There's no Applications folder.")?;
    let shown = folder.display();
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("Couldn't make the folder {shown}: {e}."))?;
    if to.exists() && splashdown_platform::app_running_at(to) {
        return Err(format!(
            "Another copy of Splashdown is open from {shown}. Quit it, then open this one again."
        ));
    }
    // Copied beside the old one first, under a hidden name, so a failed
    // copy never costs the user the copy they had.
    let copying = folder.join(format!(".Splashdown-{}.app", std::process::id()));
    let _ = std::fs::remove_dir_all(&copying);
    let forget_copy = |why: String| {
        let _ = std::fs::remove_dir_all(&copying);
        why
    };
    // ditto keeps what a plain copy loses: links inside the bundle, its
    // signature and extended attributes.
    run("/usr/bin/ditto", &[bundle.as_os_str(), copying.as_os_str()])
        .map_err(|e| forget_copy(format!("Couldn't copy it to {shown}: {e}.")))?;
    if !copying.join("Contents/Info.plist").exists() {
        return Err(forget_copy(format!(
            "The copy in {shown} came out incomplete."
        )));
    }
    // The downloaded-from-the-internet flag, which macOS checked when the
    // app was first opened: left on, the new copy would be checked again
    // and run from a hidden copy, which is what moving it avoids. As other
    // apps that move themselves do. Missing is fine.
    if let Err(e) = run(
        "/usr/bin/xattr",
        &[
            OsStr::new("-d"),
            OsStr::new("-r"),
            OsStr::new("com.apple.quarantine"),
            copying.as_os_str(),
        ],
    ) {
        tracing::debug!("clearing the quarantine flag: {e}");
    }
    if to.exists() {
        trash(to).map_err(|e| {
            forget_copy(format!(
                "Couldn't move the older copy in {shown} to the Trash: {e}."
            ))
        })?;
    }
    std::fs::rename(&copying, to).map_err(|e| {
        forget_copy(format!(
            "Couldn't put it in {shown}: {e}. If an older copy was there, it's in the Trash."
        ))
    })?;
    start_after_quit(to).map_err(|e| {
        format!("It's in {shown} now, but couldn't be started: {e}. Open it from there.")
    })
}

/// Starts the app at `to` once this process has quit (or after half a
/// minute). Started any sooner, it would find this copy still running.
fn start_after_quit(to: &Path) -> std::io::Result<()> {
    const SCRIPT: &str = r#"n=0
while kill -0 "$1" 2>/dev/null && [ "$n" -lt 150 ]; do sleep 0.2; n=$((n+1)); done
exec /usr/bin/open "$2""#;
    Command::new("/bin/sh")
        .arg("-c")
        .arg(SCRIPT)
        .arg("splashdown-move")
        .arg(std::process::id().to_string())
        .arg(to)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Runs a program, and if it fails, what it said.
fn run(program: &str, args: &[&OsStr]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if said.is_empty() {
        out.status.to_string()
    } else {
        said
    })
}

/// To the Trash through the file system, not by asking Finder (which
/// would need a permission of its own).
fn trash(path: &Path) -> Result<(), String> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};
    let mut context = trash::TrashContext::default();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context.delete(path).map_err(|e| e.to_string())
}

fn c_path(path: &Path) -> Option<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).ok()
}

/// The disk `path` is on can't be written to: a disk image.
fn read_only(path: &Path) -> bool {
    let Some(path) = c_path(path) else {
        return false;
    };
    // SAFETY: `statvfs` reads a NUL-terminated path and fills the struct
    // it's given, which is plain data that may start zeroed.
    unsafe {
        let mut disk: libc::statvfs = std::mem::zeroed();
        libc::statvfs(path.as_ptr(), &mut disk) == 0 && disk.f_flag & libc::ST_RDONLY != 0
    }
}

/// This user can add to the folder at `path`.
fn writable(path: &Path) -> bool {
    let Some(path) = c_path(path) else {
        return false;
    };
    // SAFETY: `access` only reads the NUL-terminated path.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// The user said not to ask again (from Downloads or the Desktop). Read
/// from the file directly: the app's settings aren't loaded yet.
fn declined() -> bool {
    crate::engine::paths()
        .ok()
        .and_then(|p| splashdown_config::Settings::load(&p.config_file()).ok())
        .is_some_and(|s| s.general.move_offer_declined)
}

fn remember_declined() {
    let Ok(paths) = crate::engine::paths() else {
        return;
    };
    let file = paths.config_file();
    // A file that can't be read is left for start-up to deal with (it
    // keeps a copy and tells the user); this flag isn't worth risking it.
    let saved = splashdown_config::Settings::load(&file)
        .map_err(|e| e.to_string())
        .and_then(|mut settings| {
            settings.general.move_offer_declined = true;
            settings.save(&file)
        });
    if let Err(e) = saved {
        tracing::warn!("couldn't remember not to offer the move again: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/priya";

    fn place(path: &str) -> Option<Place> {
        offer_from(Path::new(path), Path::new(HOME), false)
    }

    #[test]
    fn offered_from_a_disk_image() {
        assert_eq!(
            place("/Volumes/Splashdown/Splashdown.app"),
            Some(Place::DiskImage)
        );
        assert_eq!(
            place("/Volumes/Splashdown 1/Splashdown.app"),
            Some(Place::DiskImage)
        );
        // Deeper on a disk that can't be written to.
        assert_eq!(
            offer_from(
                Path::new("/Volumes/Backup/Apps/Splashdown.app"),
                Path::new(HOME),
                true
            ),
            Some(Place::DiskImage)
        );
    }

    #[test]
    fn offered_from_the_hidden_copy_macos_makes() {
        assert_eq!(
            place(
                "/private/var/folders/x1/abc123/T/AppTranslocation/1B2C3D4E-0000-4F5A-9B8C-ABCDEF012345/d/Splashdown.app"
            ),
            Some(Place::Translocated)
        );
    }

    #[test]
    fn offered_from_downloads_and_the_desktop() {
        assert_eq!(
            place("/Users/priya/Downloads/Splashdown.app"),
            Some(Place::Downloads)
        );
        assert_eq!(
            place("/Users/priya/Downloads/Splashdown-mac/Splashdown 2.app"),
            Some(Place::Downloads)
        );
        assert_eq!(
            place("/Users/priya/Desktop/Splashdown.app"),
            Some(Place::Desktop)
        );
    }

    #[test]
    fn silent_where_the_app_belongs_or_is_put_on_purpose() {
        for path in [
            "/Applications/Splashdown.app",
            "/Users/priya/Applications/Splashdown.app",
            // Developers' builds, wherever the code is.
            "/Users/priya/dev/splashdown/target/macos/Splashdown.app",
            "/Users/priya/Desktop/splashdown/target/macos/Splashdown.app",
            "/Users/priya/Downloads/splashdown-main/target/release/bundle/macos/Splashdown.app",
            "/Volumes/Work/splashdown/target/macos/Splashdown.app",
            "/Volumes/Work/splashdown/target/universal-apple-darwin/release/bundle/macos/Splashdown.app",
            // A drive someone keeps apps on.
            "/Volumes/External/Apps/Splashdown.app",
            // Only the folders themselves, not names that start the same.
            "/Users/priya/Downloads old/Splashdown.app",
            "/Users/priya/Desktops/Splashdown.app",
            // Another user's Downloads isn't this user's.
            "/Users/alex/Downloads/Splashdown.app",
            "/opt/Splashdown.app",
        ] {
            assert_eq!(place(path), None, "{path}");
        }
        // Even on a disk that can't be written to, a build is left alone.
        assert_eq!(
            offer_from(
                Path::new("/Volumes/Work/splashdown/target/macos/Splashdown.app"),
                Path::new(HOME),
                true
            ),
            None
        );
    }

    #[test]
    fn the_bundle_is_found_from_its_program() {
        assert_eq!(
            bundle_of(Path::new(
                "/Volumes/Splashdown/Splashdown.app/Contents/MacOS/splashdown"
            )),
            Some(Path::new("/Volumes/Splashdown/Splashdown.app"))
        );
        // `cargo run`: no bundle, so no offer.
        assert_eq!(
            bundle_of(Path::new(
                "/Users/priya/dev/splashdown/target/debug/splashdown"
            )),
            None
        );
        assert_eq!(
            bundle_of(Path::new("/Users/priya/Downloads/MacOS/splashdown")),
            None
        );
    }

    #[test]
    fn it_goes_to_the_shared_applications_folder_or_the_users_own() {
        assert_eq!(
            destination(Path::new(HOME), true),
            Path::new("/Applications/Splashdown.app")
        );
        assert_eq!(
            destination(Path::new(HOME), false),
            Path::new("/Users/priya/Applications/Splashdown.app")
        );
    }

    #[test]
    fn dont_ask_again_only_where_the_app_may_stay() {
        assert_eq!(question(Place::DiskImage).2.len(), 2);
        assert_eq!(question(Place::Translocated).2.len(), 2);
        assert_eq!(
            question(Place::Downloads).2,
            ["Move to Applications", "Not Now", "Don't Ask Again"]
        );
        assert_eq!(question(Place::Desktop).2.len(), 3);
        let (title, text, _) = question(Place::DiskImage);
        assert_eq!(title, "Move Splashdown to your Applications folder?");
        assert!(text.contains("update itself and start when you log in"));
    }
}
