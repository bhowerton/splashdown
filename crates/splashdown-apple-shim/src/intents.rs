//! Splashdown's actions in Shortcuts, Spotlight and Siri (App Intents, in
//! `swift/Sources/SplashdownAppleShim/Intents.swift`). Each action sends a
//! control request as JSON; [`start`] registers what answers it.

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::OnceLock;

use crate::ffi;

/// Any code but `ffi::OK` is an error; the Swift side shows the message.
const FAILED: i32 = 19;

type Answer = extern "C" fn(ctx: *mut c_void, status: i32, text: *const c_char);
type Handler = extern "C" fn(request: *const c_char, ctx: *mut c_void, answer: Answer);

unsafe extern "C" {
    fn splashdown_intents_start(handler: Handler);
}

type Respond = dyn Fn(String) -> Result<String, String> + Send + Sync;

static RESPOND: OnceLock<Box<Respond>> = OnceLock::new();

/// Lets the actions run: `respond` gets each request's JSON and returns
/// the answer or an error message. It's called on a thread of its own, so
/// it may wait (dictating takes as long as the person speaks). Until this
/// is called, actions wait for the app to finish starting. Only the first
/// call counts.
pub fn start(respond: impl Fn(String) -> Result<String, String> + Send + Sync + 'static) {
    if RESPOND.set(Box::new(respond)).is_err() {
        return;
    }
    // SAFETY: `on_request` matches the Swift `SplashdownIntentHandler`
    // type and lives for the whole program.
    unsafe { splashdown_intents_start(on_request) };
}

/// The Swift side's context pointer, moved to the thread that answers.
struct Context(*mut c_void);
// SAFETY: the pointer is an opaque retained Swift object that's only
// handed back through `answer`, once, from whichever thread.
unsafe impl Send for Context {}

extern "C" fn on_request(request: *const c_char, ctx: *mut c_void, answer: Answer) {
    let request = if request.is_null() {
        String::new()
    } else {
        // SAFETY: non-null, NUL-terminated UTF-8 that's valid for this
        // call; copied before returning.
        unsafe { CStr::from_ptr(request) }
            .to_string_lossy()
            .into_owned()
    };
    let ctx = Context(ctx);
    std::thread::spawn(move || {
        // The whole `Context`, not just its (unsendable) pointer field.
        let ctx = ctx;
        let result = match RESPOND.get() {
            Some(respond) => respond(request),
            None => Err("Splashdown is still starting. Try again in a moment.".into()),
        };
        let (status, text) = match result {
            Ok(text) => (ffi::OK, text),
            Err(message) => (FAILED, message),
        };
        let text = CString::new(text.replace('\0', "")).unwrap_or_default();
        // Exactly once: the Swift side releases its context here.
        answer(ctx.0, status, text.as_ptr());
    });
}
