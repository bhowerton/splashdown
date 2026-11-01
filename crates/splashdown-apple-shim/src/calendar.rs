//! The calendar, for naming a meeting: the event on now and who else is
//! invited, through the Swift shim's EventKit calls. Only used when the
//! user turns it on.

use crate::ffi;

/// The event on now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub title: String,
    /// The other people invited, by name (or address when there's no name).
    pub people: Vec<String>,
}

/// Asks for access to the calendar; macOS shows its prompt the first time.
/// Whether it's allowed.
pub async fn calendar_access() -> bool {
    // SAFETY: the shim replies exactly once through the callback.
    let (status, _) =
        ffi::call(|ctx, cb| unsafe { ffi::splashdown_calendar_access(ctx, cb) }).await;
    status == ffi::OK
}

/// The event on now, if there's one and the calendar may be read.
pub async fn calendar_now() -> Option<CalendarEvent> {
    // SAFETY: the shim replies exactly once through the callback.
    let (status, text) =
        ffi::call(|ctx, cb| unsafe { ffi::splashdown_calendar_now(ctx, cb) }).await;
    if status != ffi::OK {
        return None;
    }
    let mut lines = text.lines().map(str::trim);
    let title = lines.next().filter(|t| !t.is_empty())?.to_string();
    Some(CalendarEvent {
        title,
        people: lines
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
    })
}
