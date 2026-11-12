//! The project a coding agent works in, found from the focused window (a
//! terminal's working folder, an editor's open file), and its files, for
//! `@file` mentions (`splashdown_core::files`) and spelling hints.

use splashdown_core::sync::Locked;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

pub struct Project {
    /// Paths relative to the project root, with `/` separators.
    pub files: Vec<String>,
    /// File names of the most recently changed files, newest first: the
    /// ones most likely to be mentioned, given to speech-to-text and the
    /// AI as vocabulary.
    pub recent_names: Vec<String>,
    /// The most-used code names (`camelCase`, `snake_case`, `PascalCase`)
    /// in the open file and the most recently changed source files.
    pub identifiers: Vec<String>,
}

/// A listing and when it was made.
type Listed = (Instant, Arc<Project>);

/// A listing is reused for this long.
const FRESH: Duration = Duration::from_secs(60);
/// Stop listing after this many files (a huge folder isn't a project).
const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 12;
const RECENT_NAMES: usize = 40;
/// Folders never listed: dependencies, build output, caches.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    "Pods",
    "DerivedData",
    "__pycache__",
    "venv",
    "coverage",
];

/// The project around `document`, listed (or from the cache). `None` when
/// there's no sensible project folder, e.g. the home folder, or, on a Mac
/// without `allow_protected`, when it's in a folder macOS asks about:
/// listing it (or reading the open file) would show a prompt in the middle
/// of a dictation.
pub fn for_document(document: &Path, allow_protected: bool) -> Option<Arc<Project>> {
    let root = root_for(document)?;
    if cfg!(target_os = "macos")
        && !allow_protected
        && std::env::home_dir().is_some_and(|home| in_protected_place(&root, &home))
    {
        return None;
    }
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Listed>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((at, project)) = cache.locked().get(&root)
        && at.elapsed() < FRESH
    {
        return Some(project.clone());
    }
    let started = Instant::now();
    let open_file = document.is_file().then_some(document);
    let project = Arc::new(list(&root, open_file));
    tracing::debug!(
        "listed {} project files in {:?}",
        project.files.len(),
        started.elapsed()
    );
    cache
        .locked()
        .insert(root, (Instant::now(), project.clone()));
    Some(project)
}

/// Whether `path` is in, or is, a place macOS asks before an app may look
/// inside: Desktop, Documents and Downloads in the home folder, iCloud
/// Drive and other apps' iCloud folders (`Library/Mobile Documents`), sync
/// services' folders (`Library/CloudStorage`), and anything under
/// `/Volumes`. The startup disk can also appear as `/Volumes/<name>`, but
/// telling it apart needs the disk, so all of `/Volumes` counts. Compares
/// path parts, and doesn't touch the file system.
pub fn in_protected_place(path: &Path, home: &Path) -> bool {
    const IN_HOME: &[&[&str]] = &[
        &["Desktop"],
        &["Documents"],
        &["Downloads"],
        &["Library", "Mobile Documents"],
        &["Library", "CloudStorage"],
    ];
    if path.starts_with("/Volumes") && path != Path::new("/Volumes") {
        return true;
    }
    let Ok(inside) = path.strip_prefix(home) else {
        return false;
    };
    IN_HOME
        .iter()
        .any(|place| inside.starts_with(place.iter().collect::<PathBuf>()))
}

/// The enclosing git repository, else the folder itself — but never the
/// home folder or anything above it.
fn root_for(document: &Path) -> Option<PathBuf> {
    let start = if document.is_dir() {
        document
    } else {
        document.parent()?
    };
    let home = std::env::home_dir();
    let too_broad =
        |p: &Path| home.as_deref().is_some_and(|h| h.starts_with(p)) || p.parent().is_none();
    let mut dir = start;
    loop {
        if too_broad(dir) {
            break;
        }
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
    (!too_broad(start)).then(|| start.to_path_buf())
}

fn list(root: &Path, open_file: Option<&Path>) -> Project {
    let mut files = Vec::new();
    let mut recent: Vec<(SystemTime, String, PathBuf)> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH && !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((path, depth + 1));
                }
            } else if kind.is_file() {
                let Ok(rel) = path.strip_prefix(root) else {
                    continue;
                };
                files.push(rel.to_string_lossy().replace('\\', "/"));
                if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                    recent.push((modified, name, path));
                }
                if files.len() >= MAX_FILES {
                    break 'walk;
                }
            }
        }
    }
    recent.sort_by_key(|(modified, _, _)| std::cmp::Reverse(*modified));
    let mut recent_names: Vec<String> = Vec::new();
    for (_, name, _) in &recent {
        if !recent_names.contains(name) {
            recent_names.push(name.clone());
        }
        if recent_names.len() >= RECENT_NAMES {
            break;
        }
    }
    let sources = open_file.into_iter().map(Path::to_path_buf).chain(
        recent
            .into_iter()
            .map(|(_, _, path)| path)
            .filter(|p| is_source(p))
            .take(SOURCE_FILES),
    );
    Project {
        files,
        recent_names,
        identifiers: identifiers(sources),
    }
}

/// Source files read for code names, the most recently changed first.
const SOURCE_FILES: usize = 8;
/// Larger files are skipped (generated code, data).
const MAX_SOURCE_BYTES: u64 = 256 * 1024;
const IDENTIFIERS: usize = 40;
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "py", "go", "java", "kt", "swift", "rb", "c", "cc",
    "cpp", "h", "hpp", "cs", "php", "scala", "m", "mm", "vue", "svelte", "dart", "lua", "zig",
    "ex", "exs",
];

fn is_source(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SOURCE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The most frequent code names in `sources`: words with an underscore or
/// a capital after the first letter, which plain English words don't have.
fn identifiers(sources: impl Iterator<Item = PathBuf>) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for path in sources {
        let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_SOURCE_BYTES);
        let Some(text) = small.then(|| std::fs::read_to_string(&path).ok()).flatten() else {
            continue;
        };
        for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if looks_like_code_name(word) {
                *counts.entry(word.to_string()).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(IDENTIFIERS)
        .map(|(word, _)| word)
        .collect()
}

fn looks_like_code_name(word: &str) -> bool {
    let trimmed = word.trim_matches('_');
    if trimmed.len() < 4 || !trimmed.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return false;
    }
    let snake = trimmed.contains('_') && trimmed.chars().any(|c| c.is_ascii_lowercase());
    let camel = trimmed
        .chars()
        .skip(1)
        .zip(trimmed.chars())
        .any(|(c, prev)| c.is_ascii_uppercase() && prev.is_ascii_lowercase());
    snake || camel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_a_project_and_finds_its_root() {
        let root = std::env::temp_dir().join(format!("splashdown-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in [".git", "src/ui", "node_modules/x", ".hidden"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "src/main.rs",
            "src/ui/app.tsx",
            "README.md",
            "node_modules/x/i.js",
            ".hidden/s",
        ] {
            std::fs::write(root.join(file), "").unwrap();
        }
        // From a file deep inside, the root is the repository.
        assert_eq!(root_for(&root.join("src/ui/app.tsx")), Some(root.clone()));
        let mut files = list(&root, None).files;
        files.sort();
        assert_eq!(files, ["README.md", "src/main.rs", "src/ui/app.tsx"]);
        let _ = std::fs::remove_dir_all(&root);
        // The home folder is never a project.
        if let Some(home) = std::env::home_dir() {
            assert_eq!(root_for(&home), None);
        }
    }

    #[test]
    fn protected_places_are_found_by_path_parts() {
        let home = Path::new("/Users/someone");
        let yes = |p: &str| in_protected_place(Path::new(p), home);
        for place in [
            "Desktop",
            "Documents",
            "Downloads",
            "Library/Mobile Documents",
            "Library/CloudStorage",
        ] {
            let place = format!("/Users/someone/{place}");
            assert!(yes(&place), "{place}");
            assert!(yes(&format!("{place}/proj/src")), "{place}/proj/src");
        }
        assert!(yes(
            "/Users/someone/Library/Mobile Documents/com~apple~CloudDocs/x"
        ));
        assert!(yes("/Users/someone/Library/CloudStorage/Dropbox/x"));
        assert!(yes("/Volumes/Some Disk/x"));
        // Look-alikes and other places.
        assert!(!yes("/Users/someone/Documents2"));
        assert!(!yes("/Users/someone/Documents2/x"));
        assert!(!yes("/Users/someone/dev/Documents/x"));
        assert!(!yes("/Users/someone/Library/Caches/x"));
        assert!(!yes("/Users/someone/Library"));
        assert!(!yes("/Users/someone/dev/x"));
        assert!(!yes("/Users/someone"));
        assert!(!yes("/Users/other/Documents/x"));
        assert!(!yes("/opt/Documents/x"));
        assert!(!yes("/VolumesX/x"));
        assert!(!yes("/Volumes"));
    }

    #[test]
    fn code_names_are_ranked_by_use() {
        assert!(looks_like_code_name("retranscribe_history"));
        assert!(looks_like_code_name("AudioClip"));
        assert!(looks_like_code_name("presetRow"));
        assert!(!looks_like_code_name("hello"));
        assert!(!looks_like_code_name("HTTP"));
        assert!(!looks_like_code_name("MAX"));
        assert!(!looks_like_code_name("a_b"));
        let path =
            std::env::temp_dir().join(format!("splashdown-idents-{}.rs", std::process::id()));
        std::fs::write(
            &path,
            "fn find_pause() { find_pause(); let clip: AudioClip = x; the word }",
        )
        .unwrap();
        assert_eq!(
            identifiers([path.clone()].into_iter()),
            ["find_pause", "AudioClip"]
        );
        let _ = std::fs::remove_file(&path);
    }
}
