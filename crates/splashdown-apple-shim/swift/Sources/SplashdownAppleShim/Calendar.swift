// The calendar, for naming a meeting: the event on now, its title and who
// else is invited. Only used when the user turns it on (Settings → Notes →
// Meetings); macOS asks them for access the first time.

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
    let people: [String] = (event.attendees ?? []).filter { !$0.isCurrentUser }.compactMap { person in
        if let name = person.name, !name.isEmpty, !name.contains("@") { return name }
        let address = person.url.absoluteString.replacingOccurrences(of: "mailto:", with: "")
        return address.isEmpty ? nil : address
    }
    let title = (event.title ?? "").replacingOccurrences(of: "\n", with: " ")
    reply.send(OK, ([title] + people.map { $0.replacingOccurrences(of: "\n", with: " ") }).joined(separator: "\n"))
}
