//! Recordings kept with the notes (a meeting's, in `Attachments/`) for the
//! Notes window's player, at `notes-media://localhost/<path in the folder>`
//! (`http://notes-media.localhost/…` on Windows). Only audio and video in
//! the notes folder's `Attachments/` is served, in pieces, so a long
//! recording plays and seeks without being read whole.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

pub const SCHEME: &str = "notes-media";
/// The most sent at once.
const PIECE: u64 = 2 * 1024 * 1024;

/// Answers a request for a recording.
pub fn serve(app: &AppHandle, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    respond(app, request).unwrap_or_else(|status| {
        Response::builder()
            .status(status)
            .body(Vec::new())
            .expect("a plain response")
    })
}

fn respond(app: &AppHandle, request: &Request<Vec<u8>>) -> Result<Response<Vec<u8>>, StatusCode> {
    let rel =
        decode(request.uri().path().trim_start_matches('/')).ok_or(StatusCode::BAD_REQUEST)?;
    let kind = content_type(&rel).ok_or(StatusCode::FORBIDDEN)?;
    let mut parts = rel.split('/');
    if parts.next() != Some(super::store::ATTACHMENTS_DIR)
        || rel.contains('\\')
        || rel
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.starts_with('.'))
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let state = app.state::<AppState>();
    let store =
        super::existing_store_for(state.settings().notes.folder).ok_or(StatusCode::NOT_FOUND)?;
    // Inside Attachments/ once links are followed, too.
    let attachments = store
        .root()
        .join(super::store::ATTACHMENTS_DIR)
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let path = store
        .root()
        .join(Path::new(&rel))
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !path.starts_with(&attachments) {
        return Err(StatusCode::FORBIDDEN);
    }

    let mut file = std::fs::File::open(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let size = file.metadata().map_err(|_| StatusCode::NOT_FOUND)?.len();
    if size == 0 {
        return Response::builder()
            .header(header::CONTENT_TYPE, kind)
            .header(header::CONTENT_LENGTH, "0")
            .body(Vec::new())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| byte_range(v, size));
    let (start, end) = match range {
        Some(Ok(r)) => r,
        Some(Err(())) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .body(Vec::new())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }
        None => (0, size.saturating_sub(1)),
    };
    // A whole file asked for without a range is still sent a piece at a
    // time: players ask for the rest.
    let end = end.min(start + PIECE - 1);
    let mut body = vec![0; (end + 1 - start) as usize];
    file.seek(SeekFrom::Start(start))
        .and_then(|_| file.read_exact(&mut body))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let partial = range.is_some() || end + 1 < size;
    let mut response = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, kind)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, body.len().to_string());
    if partial {
        response = response.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
    }
    response
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn content_type(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => return None,
    })
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` as first and last byte; `Err`
/// when it's past the end. Only one range is taken.
fn byte_range(header: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let last = size.checked_sub(1);
    let range = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            last.filter(|_| n > 0)
                .map(|last| (size.saturating_sub(n), last))
        }
        (a, "") => {
            let a: u64 = a.parse().ok()?;
            last.filter(|&l| a <= l).map(|l| (a, l))
        }
        (a, b) => {
            let (a, b): (u64, u64) = (a.parse().ok()?, b.parse().ok()?);
            last.filter(|&l| a <= b && a <= l).map(|l| (a, b.min(l)))
        }
    };
    Some(range.ok_or(()))
}

/// `%20` and the like back to text.
fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_read_like_browsers_send_them() {
        assert_eq!(byte_range("bytes=0-", 100), Some(Ok((0, 99))));
        assert_eq!(byte_range("bytes=10-19", 100), Some(Ok((10, 19))));
        assert_eq!(byte_range("bytes=90-200", 100), Some(Ok((90, 99))));
        assert_eq!(byte_range("bytes=-10", 100), Some(Ok((90, 99))));
        assert_eq!(byte_range("bytes=100-", 100), Some(Err(())));
        assert_eq!(byte_range("bytes=0-", 0), Some(Err(())));
        assert_eq!(byte_range("items=0-", 100), None);
    }

    #[test]
    fn only_recordings_are_served() {
        assert_eq!(
            decode("Attachments/Weekly%20sync.wav").as_deref(),
            Some("Attachments/Weekly sync.wav")
        );
        assert_eq!(content_type("Attachments/a.WAV"), Some("audio/wav"));
        assert_eq!(content_type("Attachments/notes.md"), None);
        assert_eq!(decode("%zz"), None);
    }
}
