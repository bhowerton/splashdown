//! Finding notes by what they mean, for Ask: each note is cut into pieces
//! of about a paragraph, and the downloaded embedding model turns each
//! piece into a vector; a question's vector is compared with them all, so
//! "why was the launch delayed" finds the note that says it was pushed
//! back. Everything runs on this computer.
//!
//! The vectors are kept in the app's cache folder (one file per notes
//! folder), not beside the notes, so they don't fill a synced folder; lost,
//! they're made again. A note's are made again whenever it changes.

use splashdown_core::sync::Locked;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use splashdown_providers::LocalEmbedder;
use splashdown_providers::models::Embedder;

use super::store::{Store, Summary};

/// About how many characters each piece of a note has.
const PIECE_CHARS: usize = 900;
/// Pieces read from one note (the rest of a very long one isn't).
const MAX_PIECES: usize = 64;
/// Pieces sent to the model at once.
const BATCH: usize = 16;
/// Pieces brought up to date before answering, rather than afterwards in
/// the background: about a second or two.
const BEFORE_ASKING: usize = 16;
/// A piece this close to the question at least (cosine), and not far below
/// the closest, counts as a match.
const CLOSE: f32 = 0.35;
const NEAR_BEST: f32 = 0.12;
/// The index file's first bytes, with its version.
const MAGIC: &[u8; 4] = b"SDM1";

/// Part of a note, by character position in its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub start: usize,
    pub len: usize,
}

struct Entry {
    /// The note's last change when its vectors were made.
    modified: u64,
    pieces: Vec<(Piece, Vec<i8>)>,
}

/// One notes folder's vectors, by note path.
struct Index {
    root: PathBuf,
    notes: HashMap<String, Entry>,
}

/// A note that matches a question, with its closest piece.
pub struct Hit {
    pub path: String,
    pub piece: Piece,
    pub score: f32,
}

fn index_slot() -> &'static Mutex<Option<Index>> {
    static INDEX: OnceLock<Mutex<Option<Index>>> = OnceLock::new();
    INDEX.get_or_init(Mutex::default)
}

/// The embedding model, started once and kept (its helper process exits by
/// itself when idle, and starts again on the next use).
fn embedder() -> Option<Arc<LocalEmbedder>> {
    static EMBEDDER: OnceLock<Mutex<Option<Arc<LocalEmbedder>>>> = OnceLock::new();
    let mut slot = EMBEDDER.get_or_init(Mutex::default).locked();
    if slot.is_none() {
        let models = crate::engine::paths().ok()?.models_dir;
        match LocalEmbedder::new(&Embedder::path_in(&models)) {
            Ok(e) => *slot = Some(Arc::new(e)),
            Err(e) => {
                tracing::debug!("notes by meaning unavailable: {e}");
                return None;
            }
        }
    }
    slot.clone()
}

/// Whether the model is downloaded, so notes can be found by meaning.
pub fn available() -> bool {
    crate::engine::paths().is_ok_and(|p| Embedder::is_downloaded(&p.models_dir))
}

/// The notes closest in meaning to `question`, closest first, up to
/// `limit`. Notes changed since they were last read are read first when
/// there are only a few; otherwise that happens in the background and
/// they're left out this time. Empty without the model.
pub fn search(store: &Arc<Store>, notes: &[Summary], question: &str, limit: usize) -> Vec<Hit> {
    let Some(model) = embedder() else {
        return Vec::new();
    };
    let stale = stale_pieces(store.root(), notes);
    if stale > 0 && stale <= BEFORE_ASKING {
        update(store.root(), notes, &model);
    } else if stale > 0 {
        update_soon(store.clone());
    }
    let asked = match model.embed_question(question) {
        Ok(v) => quantize(&v),
        Err(e) => {
            tracing::warn!("couldn't read the question for meaning: {e}");
            return Vec::new();
        }
    };
    let current: HashMap<&str, u64> = notes
        .iter()
        .map(|n| (n.path.as_str(), n.modified))
        .collect();
    let slot = index_slot().locked();
    let Some(index) = slot.as_ref().filter(|i| i.root == store.root()) else {
        return Vec::new();
    };
    let mut hits: Vec<Hit> = index
        .notes
        .iter()
        .filter(|(path, entry)| current.get(path.as_str()) == Some(&entry.modified))
        .filter_map(|(path, entry)| {
            let (piece, score) = entry
                .pieces
                .iter()
                .map(|(piece, v)| (*piece, similarity(&asked, v)))
                .max_by(|a, b| a.1.total_cmp(&b.1))?;
            Some(Hit {
                path: path.clone(),
                piece,
                score,
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = hits.first().map_or(0.0, |h| h.score);
    hits.retain(|h| h.score >= CLOSE && h.score >= best - NEAR_BEST);
    hits.truncate(limit);
    hits
}

/// Brings the vectors up to date in the background (one run at a time; a
/// call during a run makes it go round again).
pub fn update_soon(store: Arc<Store>) {
    static RUNNING: Mutex<(bool, bool)> = Mutex::new((false, false));
    {
        let mut running = RUNNING.locked();
        if running.0 {
            running.1 = true;
            return;
        }
        *running = (true, false);
    }
    std::thread::spawn(move || {
        loop {
            if let Some(model) = embedder() {
                update(store.root(), &store.list(), &model);
            }
            let mut running = RUNNING.locked();
            if running.1 {
                running.1 = false;
            } else {
                running.0 = false;
                break;
            }
        }
    });
}

/// How far along the vectors are for `notes`: notes read, of all of them.
pub fn progress(root: &Path, notes: &[Summary]) -> (usize, usize) {
    let mut slot = index_slot().locked();
    let index = loaded(&mut slot, root);
    let done = notes
        .iter()
        .filter(|n| {
            index
                .notes
                .get(&n.path)
                .is_some_and(|e| e.modified == n.modified)
        })
        .count();
    (done, notes.len())
}

/// Pieces of `notes` whose vectors are missing or out of date.
fn stale_pieces(root: &Path, notes: &[Summary]) -> usize {
    let mut slot = index_slot().locked();
    let index = loaded(&mut slot, root);
    notes
        .iter()
        .filter(|n| {
            index
                .notes
                .get(&n.path)
                .is_none_or(|e| e.modified != n.modified)
        })
        .map(|n| pieces(&n.text).len())
        .sum()
}

/// Makes vectors for the notes that changed, and forgets deleted ones;
/// saved as it goes, so a long first run isn't lost if the app quits.
fn update(root: &Path, notes: &[Summary], model: &LocalEmbedder) {
    let changed: Vec<&Summary> = {
        let mut slot = index_slot().locked();
        let index = loaded(&mut slot, root);
        let present: std::collections::HashSet<&str> =
            notes.iter().map(|n| n.path.as_str()).collect();
        index
            .notes
            .retain(|path, _| present.contains(path.as_str()));
        notes
            .iter()
            .filter(|n| {
                index
                    .notes
                    .get(&n.path)
                    .is_none_or(|e| e.modified != n.modified)
            })
            .collect()
    };
    if changed.is_empty() {
        return;
    }
    tracing::info!("reading {} note(s) for meaning", changed.len());
    let started = std::time::Instant::now();
    for (i, note) in changed.iter().enumerate() {
        let parts = pieces(&note.text);
        let texts: Vec<String> = parts
            .iter()
            .map(|p| format!("{}\n{}", note.title, cut(&note.text, *p)))
            .collect();
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH) {
            match model.embed(batch) {
                Ok(v) => vectors.extend(v.iter().map(|v| quantize(v))),
                Err(e) => {
                    tracing::warn!("couldn't read notes for meaning: {e}");
                    save_now(root);
                    return;
                }
            }
        }
        {
            let mut slot = index_slot().locked();
            let index = loaded(&mut slot, root);
            index.notes.insert(
                note.path.clone(),
                Entry {
                    modified: note.modified,
                    pieces: parts.into_iter().zip(vectors).collect(),
                },
            );
        }
        if (i + 1) % 25 == 0 {
            save_now(root);
        }
    }
    save_now(root);
    tracing::info!(
        "read {} note(s) for meaning in {:.1}s",
        changed.len(),
        started.elapsed().as_secs_f32()
    );
}

/// The index for `root`, loaded from the cache (or empty) if another
/// folder's, or none, is loaded.
fn loaded<'a>(slot: &'a mut Option<Index>, root: &Path) -> &'a mut Index {
    if slot.as_ref().is_none_or(|i| i.root != root) {
        let notes = file_for(root)
            .and_then(|f| std::fs::File::open(f).ok())
            .and_then(|f| read(io::BufReader::new(f)).ok())
            .unwrap_or_default();
        *slot = Some(Index {
            root: root.to_path_buf(),
            notes,
        });
    }
    slot.as_mut().expect("just loaded")
}

fn save_now(root: &Path) {
    let Some(file) = file_for(root) else {
        return;
    };
    let slot = index_slot().locked();
    let Some(index) = slot.as_ref().filter(|i| i.root == root) else {
        return;
    };
    let result = (|| {
        std::fs::create_dir_all(file.parent().expect("in a folder"))?;
        let temp = file.with_extension("tmp");
        let mut out = io::BufWriter::new(std::fs::File::create(&temp)?);
        write(&mut out, &index.notes)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(temp, &file)
    })();
    if let Err(e) = result {
        tracing::warn!("couldn't save the notes' vectors: {e}");
    }
}

/// `<cache>/meaning/<folder hash>.bin`.
fn file_for(root: &Path) -> Option<PathBuf> {
    let models = crate::engine::paths().ok()?.models_dir;
    // FNV-1a: the same name for a folder in every build.
    let hash = root
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    Some(
        models
            .parent()?
            .join("meaning")
            .join(format!("{hash:016x}.bin")),
    )
}

// ---------------------------------------------------------------- pieces

/// `text` cut into pieces of about `PIECE_CHARS`, at line breaks where it
/// can (a meeting's turns, a list's items stay whole), else at a space.
pub fn pieces(text: &str) -> Vec<Piece> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() && out.len() < MAX_PIECES {
        // Skip blank lines and spaces between pieces.
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
        if start >= chars.len() {
            break;
        }
        let limit = (start + PIECE_CHARS).min(chars.len());
        let end = if limit == chars.len() {
            limit
        } else {
            let window = &chars[start..limit];
            // The last line break past the middle, else the last space.
            let at = |c: char| {
                window
                    .iter()
                    .rposition(|&x| x == c)
                    .filter(|&i| i > PIECE_CHARS / 2)
            };
            at('\n')
                .or_else(|| at(' '))
                .map_or(limit, |i| start + i + 1)
        };
        let piece: String = chars[start..end].iter().collect();
        if !piece.trim().is_empty() {
            out.push(Piece {
                start,
                len: end - start,
            });
        }
        start = end;
    }
    out
}

/// The text of `piece`.
pub fn cut(text: &str, piece: Piece) -> String {
    text.chars()
        .skip(piece.start)
        .take(piece.len)
        .collect::<String>()
        .trim()
        .to_string()
}

// ---------------------------------------------------------------- vectors

/// A unit vector in a byte per number: a quarter of the space, and close
/// enough to rank notes the same.
fn quantize(v: &[f32]) -> Vec<i8> {
    v.iter()
        .map(|x| (x * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

/// Cosine similarity of two quantized unit vectors.
fn similarity(a: &[i8], b: &[i8]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: i32 = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| i32::from(x) * i32::from(y))
        .sum();
    dot as f32 / (127.0 * 127.0)
}

// ---------------------------------------------------------------- file

fn write(out: &mut impl Write, notes: &HashMap<String, Entry>) -> io::Result<()> {
    out.write_all(MAGIC)?;
    out.write_all(&(notes.len() as u32).to_le_bytes())?;
    for (path, entry) in notes {
        out.write_all(&(path.len() as u32).to_le_bytes())?;
        out.write_all(path.as_bytes())?;
        out.write_all(&entry.modified.to_le_bytes())?;
        out.write_all(&(entry.pieces.len() as u32).to_le_bytes())?;
        for (piece, v) in &entry.pieces {
            out.write_all(&(piece.start as u32).to_le_bytes())?;
            out.write_all(&(piece.len as u32).to_le_bytes())?;
            out.write_all(&(v.len() as u32).to_le_bytes())?;
            out.write_all(&v.iter().map(|&x| x as u8).collect::<Vec<u8>>())?;
        }
    }
    Ok(())
}

fn read(mut input: impl Read) -> io::Result<HashMap<String, Entry>> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut magic = [0; 4];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(bad("not a vectors file"));
    }
    let count = read_u32(&mut input)?;
    let mut notes = HashMap::new();
    for _ in 0..count {
        let len = read_u32(&mut input)? as usize;
        let mut path = vec![0; len];
        input.read_exact(&mut path)?;
        let path = String::from_utf8(path).map_err(|_| bad("bad path"))?;
        let mut modified = [0; 8];
        input.read_exact(&mut modified)?;
        let n = read_u32(&mut input)? as usize;
        let mut pieces = Vec::with_capacity(n.min(MAX_PIECES));
        for _ in 0..n {
            let start = read_u32(&mut input)? as usize;
            let len = read_u32(&mut input)? as usize;
            let dims = read_u32(&mut input)? as usize;
            if dims > 16_384 {
                return Err(bad("too many dimensions"));
            }
            let mut v = vec![0u8; dims];
            input.read_exact(&mut v)?;
            pieces.push((
                Piece { start, len },
                v.into_iter().map(|x| x as i8).collect(),
            ));
        }
        notes.insert(
            path,
            Entry {
                modified: u64::from_le_bytes(modified),
                pieces,
            },
        );
    }
    Ok(notes)
}

fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    input.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_cut_at_line_breaks() {
        let line = "word ".repeat(40); // 200 characters
        let text = format!("{line}\n{line}\n{line}\n{line}\n{line}\n{line}");
        let parts = pieces(&text);
        assert_eq!(parts.len(), 2);
        // The first piece ends at a line break, with whole lines.
        assert_eq!(
            cut(&text, parts[0]),
            format!("{line}\n{line}\n{line}\n{line}").trim()
        );
        assert!(pieces("").is_empty());
        assert_eq!(pieces("  short  "), vec![Piece { start: 2, len: 7 }]);
    }

    #[test]
    fn a_long_line_is_cut_at_a_space() {
        let text = "abcdefghi ".repeat(200);
        let parts = pieces(&text);
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|p| p.len <= PIECE_CHARS));
        assert!(cut(&text, parts[0]).ends_with("abcdefghi"));
    }

    /// With the model downloaded and the helper built (`cargo build -p
    /// splashdown-llm`; `SPLASHDOWN_LLM_HELPER` points at it): a question
    /// finds the note that answers it in other words, ahead of one that
    /// shares its words.
    #[test]
    #[ignore = "needs the embedding model and the helper"]
    fn finds_notes_by_meaning() {
        let root = std::env::temp_dir().join(format!("splashdown-meaning-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Arc::new(Store::new(root.clone()));
        store.ensure().unwrap();
        for (name, body) in [
            (
                "Supplier call.md",
                "# Supplier call\nThe batteries won't arrive until February, so we're pushing the product launch back to March.",
            ),
            (
                "Groceries.md",
                "# Groceries\nEggs, oat milk, spinach, coffee beans.",
            ),
            (
                "Release checklist.md",
                "# Release checklist\nWhy we write release notes: so support knows what changed. Steps: tag, build, sign.",
            ),
            (
                "Dentist.md",
                "# Dentist\nThursday at 3pm, bring the insurance card.",
            ),
        ] {
            std::fs::write(root.join(name), body).unwrap();
        }
        let question = "why was the release delayed?";
        let words = super::super::matching(&store, question, 3, 400, false);
        assert_eq!(words[0].0, "Release checklist.md");
        let both = super::super::matching(&store, question, 3, 400, true);
        let found: Vec<&str> = both.iter().map(|(p, _)| p.as_str()).collect();
        assert!(found.contains(&"Supplier call.md"), "{found:?}");
        assert!(!found.contains(&"Groceries.md"), "{found:?}");
        let teeth = super::super::matching(
            &store,
            "when do I see the doctor for my teeth?",
            3,
            400,
            true,
        );
        assert_eq!(teeth[0].0, "Dentist.md");
        if let Some(f) = file_for(&root) {
            let _ = std::fs::remove_file(f);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn vectors_survive_the_file() {
        let mut notes = HashMap::new();
        notes.insert(
            "a/b.md".to_string(),
            Entry {
                modified: 42,
                pieces: vec![(Piece { start: 3, len: 9 }, quantize(&[0.6, -0.8]))],
            },
        );
        let mut bytes = Vec::new();
        write(&mut bytes, &notes).unwrap();
        let back = read(bytes.as_slice()).unwrap();
        let entry = &back["a/b.md"];
        assert_eq!(entry.modified, 42);
        assert_eq!(entry.pieces[0].0, Piece { start: 3, len: 9 });
        let v = &entry.pieces[0].1;
        assert!((similarity(v, v) - 1.0).abs() < 0.01);
        assert!(read(&b"nope"[..]).is_err());
    }
}
