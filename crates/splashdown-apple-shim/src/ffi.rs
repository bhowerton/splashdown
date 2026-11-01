//! The Swift shim's C ABI, and a bridge from its callbacks to Rust futures.

use std::ffi::{CStr, c_char, c_void};

use futures_channel::oneshot;

// Status codes; keep in sync with swift/Sources/SplashdownAppleShim/Shim.swift.
pub const OK: i32 = 0;
pub const NOT_ENABLED: i32 = 1;
pub const NOT_ELIGIBLE: i32 = 2;
pub const NOT_READY: i32 = 3;
pub const OS_TOO_OLD: i32 = 4;
pub const UNSUPPORTED_LANGUAGE: i32 = 6;
pub const NEEDS_DOWNLOAD: i32 = 7;
pub const DOWNLOADING: i32 = 8;
pub const TOO_LONG: i32 = 10;
pub const GUARDRAIL: i32 = 11;
pub const REFUSED: i32 = 12;

pub type Callback = extern "C" fn(ctx: *mut c_void, status: i32, text: *const c_char);
/// Mono samples at a sample rate, from the system audio tap.
pub type AudioCallback =
    extern "C" fn(ctx: *mut c_void, samples: *const f32, count: i32, rate: f64);

unsafe extern "C" {
    pub fn splashdown_apple_llm_status() -> i32;
    pub fn splashdown_apple_llm_complete(
        system: *const c_char,
        user: *const c_char,
        temperature: f64,
        max_tokens: i32,
        ctx: *mut c_void,
        callback: Callback,
    );
    pub fn splashdown_apple_stt_status(locale: *const c_char, ctx: *mut c_void, callback: Callback);
    pub fn splashdown_apple_stt_prepare(
        locale: *const c_char,
        ctx: *mut c_void,
        callback: Callback,
    );
    pub fn splashdown_tap_start(
        processes: *const u32,
        count: i32,
        ctx: *mut c_void,
        callback: AudioCallback,
        code: *mut i32,
        detail: *mut i32,
    ) -> *mut c_void;
    pub fn splashdown_tap_stop(handle: *mut c_void);
    pub fn splashdown_apple_transcribe(
        path: *const c_char,
        locale: *const c_char,
        ctx: *mut c_void,
        callback: Callback,
    );
}

type Sender = oneshot::Sender<(i32, String)>;

/// Receives the single callback for a call and completes its future.
extern "C" fn on_reply(ctx: *mut c_void, status: i32, text: *const c_char) {
    // SAFETY: `ctx` is the `Box<Sender>` leaked by `call`, and the shim
    // calls back exactly once per call (every Swift path ends in one
    // `Reply.send`), so the box is reclaimed exactly once.
    let tx = unsafe { Box::from_raw(ctx.cast::<Sender>()) };
    let text = if text.is_null() {
        String::new()
    } else {
        // SAFETY: non-null, and the shim passes a NUL-terminated UTF-8 string
        // that stays valid for the duration of this callback; we copy it out.
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    let _ = tx.send((status, text));
}

/// Starts a shim call (`start` receives the context and callback to pass
/// through) and waits for its reply.
pub async fn call(start: impl FnOnce(*mut c_void, Callback)) -> (i32, String) {
    let (tx, rx) = oneshot::channel::<(i32, String)>();
    let ctx = Box::into_raw(Box::new(tx)).cast::<c_void>();
    start(ctx, on_reply);
    rx.await
        .unwrap_or((i32::MAX, "the Apple model stopped unexpectedly".into()))
}
