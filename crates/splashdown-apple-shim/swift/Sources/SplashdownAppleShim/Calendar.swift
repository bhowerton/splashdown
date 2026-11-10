// The calendar, for naming a meeting: the event on now, its title and who
// else is invited. Only used when the user turns it on (Settings → Notes →
// Meetings); macOS asks them for access the first time. And for a hook
// before a meeting: the events that start soon.

import EventKit
import Foundation

private let events = EKEventStore()

private func hasAccess() -> Bool {
    let status = EKEventStore.authorizationStatus(for: .event)
    if #available(macOS 14.0, *) {
        return status == .fullAccess
    }
    return status == .authorized
}

/// Asks for access to the calendar (macOS shows its prompt the first time).
/// Replies OK when it's allowed, REFUSED otherwise.
@_cdecl("splashdown_calendar_access")
public func splashdown_calendar_access(_ ctx: UnsafeMutableRawPointer?, _ callback: SplashdownCallback) {
    let reply = Reply(ctx: ctx, callback: callback)
    if hasAccess() {
        reply.send(OK, "")
        return
    }
    let done: @Sendable (Bool, Error?) -> Void = { granted, error in
        reply.send(granted ? OK : REFUSED, error?.localizedDescription ?? "")
    }
    if #available(macOS 14.0, *) {
        events.requestFullAccessToEvents(completion: done)
    } else {
        events.requestAccess(to: .event, completion: done)
    }
}

/// The event on now (started in the last few hours or starting within
/// 10 minutes, not all day), preferring one with other people invited and
/// then the one that started closest to now. Replies OK with its title on
/// the first line and each other person's name on the lines after, OK with
/// nothing when there's none, or NOT_ENABLED without access.
@_cdecl("splashdown_calendar_now")
public func splashdown_calendar_now(_ ctx: UnsafeMutableRawPointer?, _ callback: SplashdownCallback) {
    let reply = Reply(ctx: ctx, callback: callback)
    guard hasAccess() else {
        reply.send(NOT_ENABLED, "")
        return
    }
    let now = Date()
    let range = events.predicateForEvents(
        withStart: now.addingTimeInterval(-6 * 3600), end: now.addingTimeInterval(10 * 60), calendars: nil)
    let on = events.events(matching: range).filter {
        !$0.isAllDay && $0.startDate <= now.addingTimeInterval(10 * 60) && $0.endDate > now
    }
    let others = { (e: EKEvent) in (e.attendees ?? []).contains { !$0.isCurrentUser } }
    let best = on.sorted { a, b in
        if others(a) != others(b) { return others(a) }
        return abs(a.startDate.timeIntervalSince(now)) < abs(b.startDate.timeIntervalSince(now))
    }.first
    guard let event = best else {
        reply.send(OK, "")
        return
    }
    let title = (event.title ?? "").replacingOccurrences(of: "\n", with: " ")
    reply.send(
        OK, ([title] + invited(event).map { $0.replacingOccurrences(of: "\n", with: " ") }).joined(separator: "\n"))
}

/// The other people invited to an event, by name (or address when there's
/// no name).
private func invited(_ event: EKEvent) -> [String] {
    (event.attendees ?? []).filter { !$0.isCurrentUser }.compactMap { person in
        if let name = person.name, !name.isEmpty, !name.contains("@") { return name }
        let address = person.url.absoluteString.replacingOccurrences(of: "mailto:", with: "")
        return address.isEmpty ? nil : address
    }
}

/// The events that start between `from` and `to` seconds from now, in the
/// order they start. Replies OK with one event on each line, its parts
/// separated by tabs: its identifier (the same each time a repeating event
/// comes round), when it starts in seconds since 1970, "all-day",
/// "declined" (the user said no, or it was called off) or nothing, its
/// title, and then each other person invited. Replies NOT_ENABLED without
/// access; it never asks for it.
@_cdecl("splashdown_calendar_upcoming")
public func splashdown_calendar_upcoming(
    _ from: Double,
    _ to: Double,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    guard hasAccess() else {
        reply.send(NOT_ENABLED, "")
        return
    }
    let now = Date()
    let (first, last) = (now.addingTimeInterval(from), now.addingTimeInterval(to))
    let range = events.predicateForEvents(withStart: first, end: last, calendars: nil)
    // The search finds events going on in that time; only ones that start in it count.
    let coming = events.events(matching: range)
        .filter { $0.startDate >= first && $0.startDate <= last }
        .sorted { $0.startDate < $1.startDate }
    let plain = { (text: String) in
        text.replacingOccurrences(of: "\n", with: " ").replacingOccurrences(of: "\t", with: " ")
    }
    let lines = coming.map { (event: EKEvent) -> String in
        let me = (event.attendees ?? []).first { $0.isCurrentUser }
        let said = event.isAllDay
            ? "all-day" : (event.status == .canceled || me?.participantStatus == .declined) ? "declined" : ""
        let parts =
            [event.eventIdentifier ?? "", String(Int(event.startDate.timeIntervalSince1970)), said, event.title ?? ""]
            + invited(event)
        return parts.map(plain).joined(separator: "\t")
    }
    reply.send(OK, lines.joined(separator: "\n"))
}

// Voice actions: adding an event or a reminder ("add dentist to my calendar
// on Thursday at 4"), and taking it back again (Undo). A hook that asks the
// AI can add reminders too, one for each to-do it wrote.

private let reminders = EKEventStore()

/// Local times as the planner writes them.
private func localTime(_ text: String) -> Date? {
    let format = DateFormatter()
    format.locale = Locale(identifier: "en_US_POSIX")
    format.timeZone = TimeZone.current
    format.dateFormat = "yyyy-MM-dd'T'HH:mm"
    return format.date(from: text)
}

/// A day with no time, as a hook writes a to-do's day.
private func localDay(_ text: String) -> Date? {
    let format = DateFormatter()
    format.locale = Locale(identifier: "en_US_POSIX")
    format.timeZone = TimeZone.current
    format.dateFormat = "yyyy-MM-dd"
    return format.date(from: text)
}

/// Runs `then` once access to `type` is settled, asking the first time.
private func withAccess(to type: EKEntityType, store: EKEventStore, _ then: @escaping @Sendable (Bool) -> Void) {
    let status = EKEventStore.authorizationStatus(for: type)
    if #available(macOS 14.0, *) {
        if status == .fullAccess { return then(true) }
        if status == .denied || status == .restricted { return then(false) }
        if type == .event {
            store.requestFullAccessToEvents { granted, _ in then(granted) }
        } else {
            store.requestFullAccessToReminders { granted, _ in then(granted) }
        }
    } else {
        if status == .authorized { return then(true) }
        store.requestAccess(to: type) { granted, _ in then(granted) }
    }
}

/// Adds an event to the default calendar. `fields` holds the title, start,
/// end (may be empty: an hour) and location (may be empty), one per line,
/// times as yyyy-MM-ddTHH:mm local. Replies OK with the event's identifier,
/// REFUSED without access, or an error.
@_cdecl("splashdown_calendar_add_event")
public func splashdown_calendar_add_event(
    _ fields: UnsafePointer<CChar>,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    let lines = String(cString: fields).components(separatedBy: "\n")
    let field = { (i: Int) in i < lines.count ? lines[i] : "" }
    withAccess(to: .event, store: events) { granted in
        guard granted else { return reply.send(REFUSED, "") }
        guard let start = localTime(field(1)) else { return reply.send(-1, "bad start time") }
        let event = EKEvent(eventStore: events)
        event.title = field(0)
        event.startDate = start
        event.endDate = localTime(field(2)).flatMap { $0 > start ? $0 : nil } ?? start.addingTimeInterval(3600)
        if !field(3).isEmpty { event.location = field(3) }
        guard let calendar = events.defaultCalendarForNewEvents else {
            return reply.send(-1, "there's no calendar to add it to")
        }
        event.calendar = calendar
        do {
            try events.save(event, span: .thisEvent, commit: true)
            reply.send(OK, event.eventIdentifier ?? "")
        } catch {
            reply.send(-1, error.localizedDescription)
        }
    }
}

/// Asks for access to Reminders (macOS shows its prompt the first time).
/// Replies OK when it's allowed, REFUSED otherwise.
@_cdecl("splashdown_reminders_access")
public func splashdown_reminders_access(_ ctx: UnsafeMutableRawPointer?, _ callback: SplashdownCallback) {
    let reply = Reply(ctx: ctx, callback: callback)
    withAccess(to: .reminder, store: reminders) { granted in
        reply.send(granted ? OK : REFUSED, "")
    }
}

/// Adds a reminder. `fields` holds the text, when it's due (may be empty:
/// a time as yyyy-MM-ddTHH:mm local, which gets an alert, or a day as
/// yyyy-MM-dd, which gets none), its notes (may be empty) and the name of
/// the list it goes in (empty for the default list), one per line. Replies
/// OK with its identifier, REFUSED without access, or an error: a list
/// that's named has to be there, since making one could hide a typing slip.
@_cdecl("splashdown_calendar_add_reminder")
public func splashdown_calendar_add_reminder(
    _ fields: UnsafePointer<CChar>,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    let lines = String(cString: fields).components(separatedBy: "\n")
    let field = { (i: Int) in i < lines.count ? lines[i] : "" }
    withAccess(to: .reminder, store: reminders) { granted in
        guard granted else { return reply.send(REFUSED, "") }
        let reminder = EKReminder(eventStore: reminders)
        reminder.title = field(0)
        if let due = localTime(field(1)) {
            reminder.dueDateComponents = Calendar.current.dateComponents(
                [.year, .month, .day, .hour, .minute], from: due)
            reminder.addAlarm(EKAlarm(absoluteDate: due))
        } else if let day = localDay(field(1)) {
            reminder.dueDateComponents = Calendar.current.dateComponents([.year, .month, .day], from: day)
        }
        if !field(2).isEmpty { reminder.notes = field(2) }
        let named = field(3).trimmingCharacters(in: .whitespaces)
        if named.isEmpty {
            guard let list = reminders.defaultCalendarForNewReminders() else {
                return reply.send(-1, "there's no reminders list to add it to")
            }
            reminder.calendar = list
        } else {
            // By name in any capitals; of two with one name, one that can be
            // added to.
            let lists = reminders.calendars(for: .reminder).filter {
                $0.title.compare(named, options: [.caseInsensitive, .diacriticInsensitive]) == .orderedSame
            }
            guard let list = lists.first(where: { $0.allowsContentModifications }) ?? lists.first else {
                return reply.send(-1, "Reminders has no list called “\(named)”")
            }
            reminder.calendar = list
        }
        do {
            try reminders.save(reminder, commit: true)
            reply.send(OK, reminder.calendarItemIdentifier)
        } catch {
            reply.send(-1, error.localizedDescription)
        }
    }
}

/// Removes an event or reminder this app just added (Undo). Replies OK,
/// or OK with nothing when it's already gone.
@_cdecl("splashdown_calendar_remove")
public func splashdown_calendar_remove(
    _ id: UnsafePointer<CChar>,
    _ reminder: Bool,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    let id = String(cString: id)
    do {
        if reminder {
            if let item = reminders.calendarItem(withIdentifier: id) as? EKReminder {
                try reminders.remove(item, commit: true)
            }
        } else if let item = events.event(withIdentifier: id) {
            try events.remove(item, span: .thisEvent, commit: true)
        }
        reply.send(OK, "")
    } catch {
        reply.send(-1, error.localizedDescription)
    }
}
