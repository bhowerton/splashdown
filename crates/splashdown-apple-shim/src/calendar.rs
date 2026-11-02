//! The calendar, through the Swift shim's EventKit calls: for naming a
//! meeting (the event on now and who else is invited), only when the user
//! turns it on; and for voice actions, adding an event or a reminder.

use std::ffi::CString;

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

/// Something added to the calendar or reminders, to take back (Undo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub id: String,
    pub reminder: bool,
}

/// Why something couldn't be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// The user hasn't allowed it (System Settings → Privacy & Security).
    NoAccess,
    Failed(String),
}

/// Adds an event: `start` and `end` as `YYYY-MM-DDTHH:MM` local (no end:
/// an hour). macOS asks for access the first time.
pub async fn add_event(
    title: &str,
    start: &str,
    end: Option<&str>,
    location: Option<&str>,
) -> Result<Added, AddError> {
    let fields = [title, start, end.unwrap_or(""), location.unwrap_or("")]
        .map(|f| f.replace('\n', " "))
        .join("\n");
    add(&fields, false).await
}

/// Adds a reminder, due at `due` (`YYYY-MM-DDTHH:MM` local) if given.
pub async fn add_reminder(text: &str, due: Option<&str>) -> Result<Added, AddError> {
    let fields = [text, due.unwrap_or("")]
        .map(|f| f.replace('\n', " "))
        .join("\n");
    add(&fields, true).await
}

async fn add(fields: &str, reminder: bool) -> Result<Added, AddError> {
    let fields = CString::new(fields).map_err(|e| AddError::Failed(e.to_string()))?;
    // SAFETY: `fields` outlives the call (the shim copies it at once), and
    // the shim replies exactly once through the callback.
    let (status, text) = ffi::call(|ctx, cb| unsafe {
        if reminder {
            ffi::splashdown_calendar_add_reminder(fields.as_ptr(), ctx, cb);
        } else {
            ffi::splashdown_calendar_add_event(fields.as_ptr(), ctx, cb);
        }
    })
    .await;
    match status {
        ffi::OK => Ok(Added { id: text, reminder }),
        ffi::REFUSED => Err(AddError::NoAccess),
        _ => Err(AddError::Failed(text)),
    }
}

/// Takes back what `add_event` or `add_reminder` added.
pub async fn remove(added: &Added) -> Result<(), String> {
    let id = CString::new(added.id.as_str()).map_err(|e| e.to_string())?;
    // SAFETY: `id` outlives the call (copied at once); one reply.
    let (status, text) = ffi::call(|ctx, cb| unsafe {
        ffi::splashdown_calendar_remove(id.as_ptr(), added.reminder, ctx, cb);
    })
    .await;
    if status == ffi::OK { Ok(()) } else { Err(text) }
}
