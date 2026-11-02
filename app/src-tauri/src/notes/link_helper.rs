//! The link helper, for transcribing pages (YouTube and the like): yt-dlp,
//! which gets a page's audio, and QuickJS-NG, the small JavaScript engine
//! yt-dlp needs for YouTube. Neither comes with the app: they're
//! downloaded on first use, after asking, from their official GitHub
//! releases, and checked against published checksums. yt-dlp updates
//! itself (sites change often, and an older copy gets refused).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

/// A file to download and the SHA-256 it must have.
struct Pinned {
    url: &'static str,
    sha256: &'static str,
}

/// QuickJS-NG v0.17.0, pinned: it changes rarely, and publishes no
/// checksum list of its own (these are GitHub's, for the release files).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const QJS: Option<Pinned> = Some(Pinned {
    url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-darwin-arm64",
    sha256: "8be3ddfe3397d2e692e4e1e8972ee9d032a0a580505d2f8b4ea528cf1b651c11",
});
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const QJS: Option<Pinned> = Some(Pinned {
    url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-darwin-x86_64",
    sha256: "9e5e101b4fd13cda3204222ca9f8be35412c41dcdef3745829633b7a67245412",
});
// Also on ARM Windows, which runs it emulated.
#[cfg(windows)]
const QJS: Option<Pinned> = Some(Pinned {
    url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-windows-x86_64.exe",
    sha256: "2aeabf0092c3262d6b2609824418f7dd7ed1f1df939f73b2b15645230cac0d77",
});
#[cfg(not(any(target_os = "macos", windows)))]
const QJS: Option<Pinned> = None;

/// yt-dlp's standalone build for this computer, by its release file name.
#[cfg(target_os = "macos")]
const YT_DLP_ASSET: Option<&str> = Some("yt-dlp_macos");
#[cfg(all(windows, target_arch = "aarch64"))]
const YT_DLP_ASSET: Option<&str> = Some("yt-dlp_arm64.exe");
#[cfg(all(windows, not(target_arch = "aarch64")))]
const YT_DLP_ASSET: Option<&str> = Some("yt-dlp.exe");
#[cfg(not(any(target_os = "macos", windows)))]
const YT_DLP_ASSET: Option<&str> = None;

/// About how much the helper downloads, for asking first.
pub const APPROX_MB: u32 = if cfg!(windows) { 26 } else { 39 };

/// How often yt-dlp updates itself before a download.
const UPDATE_EVERY: Duration = Duration::from_secs(7 * 24 * 3600);

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// The helper's programs.
pub struct Helper {
    pub yt_dlp: PathBuf,
    /// The JavaScript engine for yt-dlp (none for a yt-dlp of the user's
    /// own, which finds whatever runtime is installed).
    pub qjs: Option<PathBuf>,
    /// Ours, so it may update itself.
    pub managed: bool,
}

fn dir(models_dir: &Path) -> PathBuf {
    models_dir.join("link-helper")
}

/// The helper, if it's here. On a system without a download of ours
/// (Linux), a yt-dlp the user installed.
pub fn find(models_dir: &Path) -> Option<Helper> {
    let dir = dir(models_dir);
    let (yt_dlp, qjs) = (
        dir.join(format!("yt-dlp{EXE}")),
        dir.join(format!("qjs{EXE}")),
    );
    if yt_dlp.is_file() && qjs.is_file() {
        return Some(Helper {
            yt_dlp,
            qjs: Some(qjs),
            managed: true,
        });
    }
    if can_download() {
        return None;
    }
    system_yt_dlp().map(|yt_dlp| Helper {
        yt_dlp,
        qjs: None,
        managed: false,
    })
}

/// Whether there's a download of ours for this computer.
pub fn can_download() -> bool {
    QJS.is_some() && YT_DLP_ASSET.is_some()
}

/// Downloads the helper into the models folder: QuickJS (pinned) and the
/// latest yt-dlp, each checked against its checksum before it's kept.
pub async fn install(models_dir: &Path) -> Result<(), String> {
    let (Some(qjs), Some(asset)) = (QJS, YT_DLP_ASSET) else {
        return Err("There's no link helper for this computer.".into());
    };
    let dir = dir(models_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent("Splashdown")
        .build()
        .map_err(|e| e.to_string())?;

    download(&client, qjs.url, qjs.sha256, &dir.join(format!("qjs{EXE}"))).await?;

    // The latest release, and its checksums, from the same release.
    let latest: serde_json::Value = client
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("Couldn't find the latest yt-dlp: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let tag = latest["tag_name"]
        .as_str()
        .ok_or("Couldn't find the latest yt-dlp.")?;
    let base = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{tag}");
    let sums = client
        .get(format!("{base}/SHA2-256SUMS"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("Couldn't get yt-dlp's checksums: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let sha256 = checksum_for(&sums, asset).ok_or("yt-dlp's checksums don't list it.")?;
    download(
        &client,
        &format!("{base}/{asset}"),
        &sha256,
        &dir.join(format!("yt-dlp{EXE}")),
    )
    .await?;
    touch(&dir);
    tracing::info!("link helper installed: yt-dlp {tag}, QuickJS");
    Ok(())
}

/// The SHA-256 for `name` in a `SHA2-256SUMS` file.
fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_lowercase())
    })
}

/// Downloads `url` to `dest` if its SHA-256 is `sha256`.
async fn download(
    client: &reqwest::Client,
    url: &str,
    sha256: &str,
    dest: &Path,
) -> Result<(), String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("The download failed: {e}"))?;
    let part = dest.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let result = async {
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), resp.chunk())
            .await
            .map_err(|_| "The download stopped coming.".to_string())?
            .map_err(|e| format!("The download stopped: {e}"))?
        {
            if super::files::download_cancelled() {
                return Err(super::files::STOPPED.to_string());
            }
            hash.update(&chunk);
            file.write_all(&chunk).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    .await;
    drop(file);
    let got = format!("{:x}", hash.finalize());
    let result = result.and_then(|()| {
        if got == sha256 {
            Ok(())
        } else {
            Err(format!("The download didn't match its checksum ({url})."))
        }
    });
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    // A virus scanner checks a new program before it can be moved.
    splashdown_providers::models::rename_patiently(&part, dest, std::time::Duration::from_secs(10))
        .map_err(|e| e.to_string())
}

/// Whether yt-dlp hasn't updated itself for a while.
pub fn update_due(helper: &Helper) -> bool {
    helper.managed
        && helper
            .yt_dlp
            .parent()
            .and_then(|d| std::fs::metadata(d.join("updated")).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_none_or(|age| age >= UPDATE_EVERY)
}

/// Has yt-dlp update itself (it checks what it downloads). Blocks for a
/// few seconds; Stop ends it.
pub fn update(helper: &Helper) {
    if !helper.managed {
        return;
    }
    let mut command = std::process::Command::new(&helper.yt_dlp);
    command
        .arg("-U")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match super::files::run_stoppable(command) {
        Ok(out) => tracing::info!(
            "yt-dlp update: {}",
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .last()
                .unwrap_or_default()
        ),
        Err(e) => tracing::warn!("yt-dlp couldn't update itself: {e}"),
    }
    if let Some(dir) = helper.yt_dlp.parent() {
        touch(dir);
    }
}

/// Notes when yt-dlp was last brought up to date.
fn touch(dir: &Path) {
    let _ = std::fs::write(dir.join("updated"), b"");
}

/// A yt-dlp the user installed: on the `PATH`, or where Homebrew put it
/// (an app started from the Dock doesn't get the shell's `PATH`).
fn system_yt_dlp() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter()
        .map(|d| d.join(format!("yt-dlp{EXE}")))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_are_found_by_file_name() {
        let sums = "0f92  yt-dlp_macos\nf538 *yt-dlp.exe\n0066  yt-dlp_arm64.exe\n";
        assert_eq!(checksum_for(sums, "yt-dlp_macos").as_deref(), Some("0f92"));
        assert_eq!(checksum_for(sums, "yt-dlp.exe").as_deref(), Some("f538"));
        assert_eq!(checksum_for(sums, "yt-dlp"), None);
    }

    #[test]
    fn this_computer_has_a_download() {
        assert_eq!(can_download(), cfg!(any(target_os = "macos", windows)));
    }
}
