//! The calendar, through the Swift shim's EventKit calls: for naming a
//! meeting (the event on now and who else is invited), only when the user
//! turns it on; for a hook before a meeting, the events that start soon;
//! and for voice actions and hooks, adding an event or a reminder.

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

/// An event that starts soon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upcoming {
    /// The calendar's identifier for it: the same each time a repeating
    /// event comes round.
    pub id: String,
    /// When it starts, in seconds since 1970.
    pub starts: i64,
    pub all_day: bool,
    /// The user said no to it, or it was called off.
    pub declined: bool,
    pub title: String,
    /// The other people invited, by name (or address when there's no name).
    pub people: Vec<String>,
}

/// The events that start between `from` and `to` seconds from now, in the
/// order they start; `None` when the calendar may not be read. Never asks
/// for access: `calendar_access` does.
pub async fn calendar_upcoming(from: f64, to: f64) -> Option<Vec<Upcoming>> {
    // SAFETY: the shim replies exactly once through the callback.
    let (status, text) =
        ffi::call(|ctx, cb| unsafe { ffi::splashdown_calendar_upcoming(from, to, ctx, cb) }).await;
    (status == ffi::OK).then(|| upcoming(&text))
}

/// The events in the shim's reply: one on each line, its parts separated
/// by tabs. A line that isn't one is left out.
fn upcoming(text: &str) -> Vec<Upcoming> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let id = parts.next()?.to_string();
            let starts = parts.next()?.parse().ok()?;
            let said = parts.next()?;
            let title = parts.next()?.trim().to_string();
            Some(Upcoming {
                id,
                starts,
                all_day: said == "all-day",
                declined: said == "declined",
                title,
                people: parts
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
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

/// Asks for access to Reminders; macOS shows its prompt the first time.
/// Whether it's allowed.
pub async fn reminders_access() -> bool {
    // SAFETY: the shim replies exactly once through the callback.
    let (status, _) =
        ffi::call(|ctx, cb| unsafe { ffi::splashdown_reminders_access(ctx, cb) }).await;
    status == ffi::OK
}

/// Whether Reminders has a list called `name`, in any capitals.
pub async fn reminders_list_exists(name: &str) -> Result<bool, AddError> {
    let name = CString::new(name).map_err(|e| AddError::Failed(e.to_string()))?;
    // SAFETY: `name` outlives the call (the shim copies it at once), and
    // the shim replies exactly once through the callback.
    let (status, text) = ffi::call(|ctx, cb| unsafe {
        ffi::splashdown_reminders_has_list(name.as_ptr(), ctx, cb);
    })
    .await;
    match status {
        ffi::OK => Ok(text == "1"),
        ffi::REFUSED => Err(AddError::NoAccess),
        _ => Err(AddError::Failed(text)),
    }
}

/// Adds a reminder, due at `due` if given: `YYYY-MM-DDTHH:MM` local, with
/// an alert then, or `YYYY-MM-DD` for a day with no time and no alert.
/// `notes` goes in the reminder's notes. It goes in the list called `list`
/// (in any capitals), which has to be there, or the default list.
pub async fn add_reminder(
    text: &str,
    due: Option<&str>,
    notes: Option<&str>,
    list: Option<&str>,
) -> Result<Added, AddError> {
    let fields = [
        text,
        due.unwrap_or(""),
        notes.unwrap_or(""),
        list.unwrap_or(""),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upcoming_events_are_read_from_the_shim_s_lines() {
        let events = upcoming(
            "A1\t1794322800\t\tPricing review\tDana Lee\tsam@example.com\nB2\t1794326400\tdeclined\tAll hands\nC3\t1794268800\tall-day\tOffsite\nnot an event\nD4\tsoon\t\tNo time",
        );
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            Upcoming {
                id: "A1".into(),
                starts: 1_794_322_800,
                all_day: false,
                declined: false,
                title: "Pricing review".into(),
                people: vec!["Dana Lee".into(), "sam@example.com".into()],
            }
        );
        assert!(events[1].declined && events[1].people.is_empty());
        assert!(events[2].all_day && !events[2].declined);
        assert!(upcoming("").is_empty());
    }
}
