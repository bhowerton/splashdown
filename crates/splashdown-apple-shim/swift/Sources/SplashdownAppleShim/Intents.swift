// Splashdown's actions in Shortcuts, Spotlight and Siri (App Intents).
//
// They run inside the app. Each one sends a control request, the same JSON
// the control socket takes (crates/splashdown-control), to the handler the
// Rust side registers at startup (`splashdown_intents_start`, src/intents.rs
// in this crate), and returns its answer. build.rs extracts their metadata
// into Metadata.appintents, which the app bundle carries in
// Contents/Resources; without it the system doesn't know they exist.

import AppIntents
import Foundation
import UniformTypeIdentifiers

/// Rust's side: takes a request as JSON and answers exactly once through
/// `answer` (`OK` and the reply, or an error code and a message).
public typealias SplashdownIntentHandler = @convention(c) (
    UnsafePointer<CChar>?, UnsafeMutableRawPointer?, SplashdownCallback
) -> Void

private let handlerLock = NSLock()
private var registeredHandler: SplashdownIntentHandler?

@_cdecl("splashdown_intents_start")
public func splashdown_intents_start(_ handler: SplashdownIntentHandler) {
    handlerLock.lock()
    registeredHandler = handler
    handlerLock.unlock()
}

private func currentHandler() -> SplashdownIntentHandler? {
    handlerLock.lock()
    defer { handlerLock.unlock() }
    return registeredHandler
}

/// An error Shortcuts and Siri show as it is.
struct SplashdownError: Error {
    let message: String
}

@available(macOS 13.0, *)
extension SplashdownError: CustomLocalizedStringResourceConvertible {
    var localizedStringResource: LocalizedStringResource {
        LocalizedStringResource(stringLiteral: message)
    }
}

private final class Pending {
    let continuation: CheckedContinuation<String, Error>
    init(_ continuation: CheckedContinuation<String, Error>) {
        self.continuation = continuation
    }
}

private let onAnswer: SplashdownCallback = { ctx, status, text in
    guard let ctx else { return }
    let pending = Unmanaged<Pending>.fromOpaque(ctx).takeRetainedValue()
    let message = text.map { String(cString: $0) } ?? ""
    if status == OK {
        pending.continuation.resume(returning: message)
    } else {
        pending.continuation.resume(throwing: SplashdownError(message: message))
    }
}

/// The app's answer (app/src-tauri/src/intents.rs, `Reply`).
struct Answer: Decodable {
    /// For the person: "Added to Inbox".
    var message: String?
    /// Text for the next action.
    var text: String?
    /// A note's file.
    var file: String?
    /// One note to pass on.
    var note: NoteInfo?
    /// Notes found, or the ones an answer drew on.
    var notes: [NoteInfo]?
}

@available(macOS 13.0, *)
extension Answer {
    /// `message` as Siri and Shortcuts say it.
    var dialog: IntentDialog {
        IntentDialog(stringLiteral: message ?? "Done")
    }

    /// The note, as a Markdown file the next action can read or share.
    func noteFile() throws -> IntentFile {
        guard let file else {
            throw SplashdownError(message: "Splashdown didn't say which note.")
        }
        let url = URL(fileURLWithPath: file)
        return IntentFile(
            fileURL: url,
            filename: url.lastPathComponent,
            type: UTType(filenameExtension: "md") ?? .plainText
        )
    }
}

enum Splashdown {
    /// Sends `request` to the app and waits for its answer. When Shortcuts
    /// has just launched the app, waits (up to 15 seconds) for it to start.
    static func send(_ request: [String: Any]) async throws -> Answer {
        var handler = currentHandler()
        var waited = 0
        while handler == nil, waited < 150 {
            try await Task.sleep(nanoseconds: 100_000_000)
            waited += 1
            handler = currentHandler()
        }
        guard let handler else {
            throw SplashdownError(message: "Splashdown is still starting. Try again in a moment.")
        }
        let data = try JSONSerialization.data(withJSONObject: request)
        let json = String(decoding: data, as: UTF8.self)
        let reply: String = try await withCheckedThrowingContinuation { continuation in
            let ctx = Unmanaged.passRetained(Pending(continuation)).toOpaque()
            json.withCString { handler($0, ctx, onAnswer) }
        }
        return try JSONDecoder().decode(Answer.self, from: Data(reply.utf8))
    }
}

// MARK: - Actions

@available(macOS 13.0, *)
struct AddToNoteIntent: AppIntent {
    static var title: LocalizedStringResource = "Add to Note"
    static var description = IntentDescription(
        "Adds text to the end of your Splashdown note (the one set in Settings → Notes: your inbox or today's note), and passes the note on."
    )
    static var openAppWhenRun = false

    @Parameter(
        title: "Text",
        inputOptions: String.IntentInputOptions(multiline: true),
        requestValueDialog: "What should I add?"
    )
    var text: String

    static var parameterSummary: some ParameterSummary {
        Summary("Add \(\.$text) to my Splashdown note")
    }

    func perform() async throws -> some IntentResult & ReturnsValue<IntentFile> & ProvidesDialog {
        let answer = try await Splashdown.send(["cmd": "add_to_note", "text": text])
        return .result(value: try answer.noteFile(), dialog: answer.dialog)
    }
}

@available(macOS 13.0, *)
struct DictateTextIntent: AppIntent {
    static var title: LocalizedStringResource = "Dictate Text"
    static var description = IntentDescription(
        "Listens hands-free and passes on what you said, cleaned up the way Splashdown types it. Finish with your dictation key, or press Esc to cancel."
    )
    static var openAppWhenRun = false

    @Parameter(
        title: "Prompt",
        description: "Shown while Splashdown listens, such as \"Reply to Sam\"."
    )
    var prompt: String?

    static var parameterSummary: some ParameterSummary {
        Summary("Dictate text with Splashdown") {
            \.$prompt
        }
    }

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        var request: [String: Any] = ["cmd": "dictate"]
        if let prompt, !prompt.isEmpty {
            request["prompt"] = prompt
        }
        let answer = try await Splashdown.send(request)
        return .result(value: answer.text ?? "")
    }
}

@available(macOS 13.0, *)
struct ToggleDictationIntent: AppIntent {
    static var title: LocalizedStringResource = "Start or Stop Dictation"
    static var description = IntentDescription(
        "Starts a hands-free dictation into the app you're in, or finishes the one in progress, as if you'd pressed the dictation key."
    )
    static var openAppWhenRun = false

    func perform() async throws -> some IntentResult {
        _ = try await Splashdown.send(["cmd": "toggle_dictation"])
        return .result()
    }
}

@available(macOS 13.0, *)
struct RecordVoiceNoteIntent: AppIntent {
    static var title: LocalizedStringResource = "Record a Voice Note"
    static var description = IntentDescription(
        "Starts a hands-free recording that goes into your Splashdown note, or finishes the one in progress."
    )
    static var openAppWhenRun = false

    func perform() async throws -> some IntentResult {
        _ = try await Splashdown.send(["cmd": "record_note"])
        return .result()
    }
}

@available(macOS 13.0, *)
struct StartMeetingIntent: AppIntent {
    static var title: LocalizedStringResource = "Start Meeting"
    static var description = IntentDescription(
        "Starts recording a meeting into a new note: you, and the other people on the call."
    )
    static var openAppWhenRun = false

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let answer = try await Splashdown.send(["cmd": "start_meeting"])
        return .result(dialog: answer.dialog)
    }
}

@available(macOS 13.0, *)
struct StopMeetingIntent: AppIntent {
    static var title: LocalizedStringResource = "Stop Meeting"
    static var description = IntentDescription(
        "Stops the meeting that's recording, and passes its note on. The note's last words and summary take a moment to finish; turn on Wait for the Note to pass it on finished."
    )
    static var openAppWhenRun = false

    @Parameter(title: "Wait for the Note", default: false)
    var wait: Bool

    static var parameterSummary: some ParameterSummary {
        Summary("Stop the meeting") {
            \.$wait
        }
    }

    func perform() async throws -> some IntentResult & ReturnsValue<IntentFile> & ProvidesDialog {
        let answer = try await Splashdown.send(["cmd": "stop_meeting", "wait": wait])
        return .result(value: try answer.noteFile(), dialog: answer.dialog)
    }
}

// MARK: - Siri and Spotlight

// Every phrase names the app. A phrase can't hold free text, so Siri asks
// for it ("What should I add?").
@available(macOS 13.0, *)
struct SplashdownShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: DictateTextIntent(),
            phrases: ["Dictate with \(.applicationName)"]
        )
        AppShortcut(
            intent: AddToNoteIntent(),
            phrases: [
                "Add to my \(.applicationName) note",
                "Add a note in \(.applicationName)",
            ]
        )
        AppShortcut(
            intent: RecordVoiceNoteIntent(),
            phrases: [
                "Record a note in \(.applicationName)",
                "Record a voice note with \(.applicationName)",
            ]
        )
        AppShortcut(
            intent: StartMeetingIntent(),
            phrases: [
                "Start a meeting in \(.applicationName)",
                "Record a meeting with \(.applicationName)",
            ]
        )
        AppShortcut(
            intent: StopMeetingIntent(),
            phrases: ["Stop the meeting in \(.applicationName)"]
        )
        AppShortcut(
            intent: AskNotesIntent(),
            phrases: [
                "Ask \(.applicationName)",
                "Ask \(.applicationName) about my notes",
            ]
        )
        AppShortcut(
            intent: FindNotesIntent(),
            phrases: ["Find notes in \(.applicationName)"]
        )
    }
}
