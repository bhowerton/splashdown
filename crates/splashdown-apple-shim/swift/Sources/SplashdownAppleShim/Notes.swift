// Splashdown's notes in Shortcuts: the note type actions pass between them,
// and the actions that find, read, ask about, transcribe into and open
// notes. Settings → Privacy → "Let shortcuts find and read notes" decides
// whether they may (app/src-tauri/src/intents.rs); notes hidden from agents
// never come back.

import AppIntents
import Foundation
import UniformTypeIdentifiers

/// A note as the app describes it (intents.rs, `NoteRef`).
struct NoteInfo: Decodable {
    let path: String
    let title: String
    let kind: String
    let created: String?
    let folder: String
}

@available(macOS 13.0, *)
struct NoteEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Splashdown Note"
    static var defaultQuery = NoteQuery()

    /// The note's path inside the notes folder.
    var id: String

    @Property(title: "Title")
    var title: String

    /// Meeting, transcript, capture or note.
    @Property(title: "Kind")
    var kind: String

    @Property(title: "Folder")
    var folder: String

    @Property(title: "Created")
    var created: Date?

    init(_ info: NoteInfo) {
        id = info.path
        title = info.title
        kind = info.kind
        folder = info.folder
        created = info.created.flatMap { ISO8601DateFormatter().date(from: $0) }
    }

    var displayRepresentation: DisplayRepresentation {
        let place = folder.isEmpty ? kind.capitalized : "\(kind.capitalized) · \(folder)"
        return DisplayRepresentation(title: "\(title)", subtitle: "\(place)")
    }
}

/// How Shortcuts finds notes: by path, by what's typed in the picker, and
/// the newest as suggestions.
@available(macOS 13.0, *)
struct NoteQuery: EntityStringQuery {
    func entities(for identifiers: [String]) async throws -> [NoteEntity] {
        let answer = try await Splashdown.send(["cmd": "notes_at", "paths": identifiers])
        return (answer.notes ?? []).map(NoteEntity.init)
    }

    func entities(matching string: String) async throws -> [NoteEntity] {
        let answer = try await Splashdown.send(["cmd": "find_notes", "query": string])
        return (answer.notes ?? []).map(NoteEntity.init)
    }

    func suggestedEntities() async throws -> [NoteEntity] {
        let answer = try await Splashdown.send(["cmd": "find_notes", "limit": 20])
        return (answer.notes ?? []).map(NoteEntity.init)
    }
}

@available(macOS 13.0, *)
enum NoteKind: String, AppEnum {
    case meeting, transcript, capture, note

    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Kind of Note"
    static var caseDisplayRepresentations: [NoteKind: DisplayRepresentation] = [
        .meeting: "Meetings",
        .transcript: "Transcripts",
        .capture: "Captures",
        .note: "Notes",
    ]
}

@available(macOS 13.0, *)
enum NoteTime: String, AppEnum {
    case today
    case yesterday
    case thisWeek = "this_week"
    case lastWeek = "last_week"
    case last7Days = "last_7_days"
    case thisMonth = "this_month"
    case lastMonth = "last_month"
    case last30Days = "last_30_days"

    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Time"
    static var caseDisplayRepresentations: [NoteTime: DisplayRepresentation] = [
        .today: "Today",
        .yesterday: "Yesterday",
        .thisWeek: "This Week",
        .lastWeek: "Last Week",
        .last7Days: "Last 7 Days",
        .thisMonth: "This Month",
        .lastMonth: "Last Month",
        .last30Days: "Last 30 Days",
    ]
}

@available(macOS 13.0, *)
enum NoteSection: String, AppEnum {
    case all
    case summary
    case myNotes = "my_notes"
    case transcript

    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Part of Note"
    static var caseDisplayRepresentations: [NoteSection: DisplayRepresentation] = [
        .all: "Whole Note",
        .summary: "Summary",
        .myNotes: "My Notes",
        .transcript: "Transcript",
    ]
}

// MARK: - Actions

@available(macOS 13.0, *)
struct FindNotesIntent: AppIntent {
    static var title: LocalizedStringResource = "Find Notes"
    static var description = IntentDescription(
        "Finds Splashdown notes whose title or text contains what you search for, newest first. With nothing to search for, passes on the newest notes."
    )
    static var openAppWhenRun = false

    @Parameter(title: "Search For")
    var query: String?

    @Parameter(title: "Kind")
    var kind: NoteKind?

    @Parameter(title: "From")
    var time: NoteTime?

    @Parameter(title: "Limit", default: 20, inclusiveRange: (1, 50))
    var limit: Int

    static var parameterSummary: some ParameterSummary {
        Summary("Find Splashdown notes matching \(\.$query)") {
            \.$kind
            \.$time
            \.$limit
        }
    }

    func perform() async throws -> some IntentResult & ReturnsValue<[NoteEntity]> {
        var request: [String: Any] = ["cmd": "find_notes", "limit": limit]
        if let query, !query.isEmpty {
            request["query"] = query
        }
        if let kind {
            request["kind"] = kind.rawValue
        }
        if let time {
            request["when"] = time.rawValue
        }
        let answer = try await Splashdown.send(request)
        return .result(value: (answer.notes ?? []).map(NoteEntity.init))
    }
}

@available(macOS 13.0, *)
struct GetNoteTextIntent: AppIntent {
    static var title: LocalizedStringResource = "Get Note Text"
    static var description = IntentDescription(
        "Passes on a Splashdown note's text: all of it, or a meeting's summary, your own notes from it, or its transcript."
    )
    static var openAppWhenRun = false

    @Parameter(title: "Note")
    var note: NoteEntity

    @Parameter(title: "Part", default: .all)
    var section: NoteSection

    static var parameterSummary: some ParameterSummary {
        Summary("Get \(\.$section) of \(\.$note)")
    }

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        let answer = try await Splashdown.send([
            "cmd": "read_note", "path": note.id, "section": section.rawValue,
        ])
        return .result(value: answer.text ?? "")
    }
}

@available(macOS 13.0, *)
struct AskNotesIntent: AppIntent {
    static var title: LocalizedStringResource = "Ask My Notes"
    static var description = IntentDescription(
        "Answers a question from your Splashdown notes, with the AI model set in Settings, and passes the answer on."
    )
    static var openAppWhenRun = false

    @Parameter(
        title: "Question",
        inputOptions: String.IntentInputOptions(multiline: true),
        requestValueDialog: "What's your question?"
    )
    var question: String

    static var parameterSummary: some ParameterSummary {
        Summary("Ask my Splashdown notes \(\.$question)")
    }

    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let answer = try await Splashdown.send(["cmd": "ask", "question": question])
        let text = answer.text ?? ""
        return .result(value: text, dialog: IntentDialog(stringLiteral: text))
    }
}

@available(macOS 13.0, *)
struct TranscribeFileIntent: AppIntent {
    static var title: LocalizedStringResource = "Transcribe File"
    static var description = IntentDescription(
        "Transcribes an audio or video file into a new Splashdown note, and passes the note on once it's done. Long files take a while."
    )
    static var openAppWhenRun = false

    @Parameter(
        title: "File",
        supportedTypeIdentifiers: ["public.audio", "public.movie"]
    )
    var file: IntentFile

    static var parameterSummary: some ParameterSummary {
        Summary("Transcribe \(\.$file) into a Splashdown note")
    }

    func perform() async throws -> some IntentResult & ReturnsValue<NoteEntity> & ProvidesDialog {
        // The file where it is, when Shortcuts says; otherwise a copy that
        // goes once it's transcribed.
        var copy: URL?
        let path: String
        if let url = file.fileURL, FileManager.default.isReadableFile(atPath: url.path) {
            path = url.path
        } else {
            let folder = FileManager.default.temporaryDirectory
                .appendingPathComponent("splashdown-shortcuts-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let url = folder.appendingPathComponent(file.filename.isEmpty ? "Recording.m4a" : file.filename)
            try file.data.write(to: url)
            copy = folder
            path = url.path
        }
        defer {
            if let copy {
                try? FileManager.default.removeItem(at: copy)
            }
        }
        let answer = try await Splashdown.send(["cmd": "transcribe_file", "path": path])
        guard let note = answer.note else {
            throw SplashdownError(message: "Splashdown didn't say which note.")
        }
        return .result(value: NoteEntity(note), dialog: answer.dialog)
    }
}

@available(macOS 13.0, *)
struct OpenNoteIntent: OpenIntent {
    static var title: LocalizedStringResource = "Open Note"
    static var description = IntentDescription("Shows a note in Splashdown's Notes window.")
    static var openAppWhenRun = false

    @Parameter(title: "Note")
    var target: NoteEntity

    static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$target) in Splashdown")
    }

    func perform() async throws -> some IntentResult {
        _ = try await Splashdown.send(["cmd": "open_note", "path": target.id])
        return .result()
    }
}
