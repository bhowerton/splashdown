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
struct SplashdownError: Error, CustomLocalizedStringResourceConvertible {
    let message: String
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

enum Splashdown {
    /// Sends `request` to the app and waits for its answer. When Shortcuts
    /// has just launched the app, waits (up to 15 seconds) for it to start.
    static func send(_ request: [String: Any]) async throws -> String {
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
        return try await withCheckedThrowingContinuation { continuation in
            let ctx = Unmanaged.passRetained(Pending(continuation)).toOpaque()
            json.withCString { handler($0, ctx, onAnswer) }
        }
    }
}

// MARK: - Actions

@available(macOS 13.0, *)
struct AddToNoteIntent: AppIntent {
    static var title: LocalizedStringResource = "Add to Note"
    static var description = IntentDescription(
        "Adds text to the end of your Splashdown note (today's daily note, or the capture note set in Settings → Notes)."
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

    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let answer = try await Splashdown.send(["cmd": "note", "text": text])
        return .result(value: answer, dialog: IntentDialog(stringLiteral: answer))
    }
}

// MARK: - Siri and Spotlight

@available(macOS 13.0, *)
struct SplashdownShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: AddToNoteIntent(),
            phrases: [
                "Add to my \(.applicationName) note",
                "Add a note in \(.applicationName)",
            ]
        )
    }
}
