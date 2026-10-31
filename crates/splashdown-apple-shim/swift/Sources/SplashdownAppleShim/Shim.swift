// C ABI over Apple's on-device models.
//
// Every async entry point reports back exactly once through `SplashdownCallback`
// with a status code and a UTF-8 string (the result, or an error message).
// The string is only valid during the callback; the Rust side copies it.

import AVFAudio
import Foundation
import FoundationModels
import Speech

public typealias SplashdownCallback = @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<CChar>?) -> Void

// Status codes shared with ../src/lib.rs.
let OK: Int32 = 0
let NOT_ENABLED: Int32 = 1        // Apple Intelligence is off
let NOT_ELIGIBLE: Int32 = 2       // this Mac can't run it
let NOT_READY: Int32 = 3          // model or assets still downloading
let OS_TOO_OLD: Int32 = 4         // needs macOS 26
let UNSUPPORTED_LANGUAGE: Int32 = 6
let NEEDS_DOWNLOAD: Int32 = 7     // speech assets supported but not installed
let DOWNLOADING: Int32 = 8
let TOO_LONG: Int32 = 10
let GUARDRAIL: Int32 = 11
let REFUSED: Int32 = 12
let FAILED: Int32 = 19

/// Carries the Rust context pointer across the `Task` boundary.
struct Reply: @unchecked Sendable {
    let ctx: UnsafeMutableRawPointer?
    let callback: SplashdownCallback

    func send(_ status: Int32, _ text: String) {
        text.withCString { callback(ctx, status, $0) }
    }
}

// MARK: - Language model

@_cdecl("splashdown_apple_llm_status")
public func splashdown_apple_llm_status() -> Int32 {
    guard #available(macOS 26.0, *) else { return OS_TOO_OLD }
    switch SystemLanguageModel.default.availability {
    case .available:
        return OK
    case .unavailable(.appleIntelligenceNotEnabled):
        return NOT_ENABLED
    case .unavailable(.deviceNotEligible):
        return NOT_ELIGIBLE
    case .unavailable(.modelNotReady):
        return NOT_READY
    case .unavailable:
        return FAILED
    }
}

@_cdecl("splashdown_apple_llm_complete")
public func splashdown_apple_llm_complete(
    _ system: UnsafePointer<CChar>,
    _ user: UnsafePointer<CChar>,
    _ temperature: Double,
    _ maxTokens: Int32,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    let system = String(cString: system)
    let user = String(cString: user)
    guard #available(macOS 26.0, *) else {
        reply.send(OS_TOO_OLD, "Apple's on-device model needs macOS 26 or later")
        return
    }
    Task {
        do {
            // Our prompts transform the user's own words, which the default
            // guardrails sometimes refuse.
            let model = SystemLanguageModel(useCase: .general, guardrails: .permissiveContentTransformations)
            let session = LanguageModelSession(model: model, instructions: system)
            let options = GenerationOptions(temperature: temperature, maximumResponseTokens: Int(maxTokens))
            let response = try await session.respond(to: user, options: options)
            reply.send(OK, response.content)
        } catch let error as LanguageModelSession.GenerationError {
            switch error {
            case .exceededContextWindowSize:
                reply.send(TOO_LONG, "too long for the on-device model")
            case .guardrailViolation:
                reply.send(GUARDRAIL, "blocked by the on-device model's safety guardrails")
            case .refusal:
                reply.send(REFUSED, "the on-device model declined")
            case .unsupportedLanguageOrLocale:
                reply.send(UNSUPPORTED_LANGUAGE, "language not supported by the on-device model")
            case .assetsUnavailable:
                reply.send(NOT_READY, "the on-device model isn't ready yet")
            default:
                reply.send(FAILED, error.localizedDescription)
            }
        } catch {
            reply.send(FAILED, error.localizedDescription)
        }
    }
}

// MARK: - Speech

/// The transcriber for `locale` (or the user's locale), or the status code
/// explaining why there isn't one.
@available(macOS 26.0, *)
func transcriber(_ locale: UnsafePointer<CChar>?) async -> Result<SpeechTranscriber, Status> {
    guard SpeechTranscriber.isAvailable else {
        return .failure(Status(NOT_ELIGIBLE, "this Mac can't run Apple's speech model"))
    }
    let wanted = locale.map { Locale(identifier: String(cString: $0)) } ?? Locale.current
    guard let supported = await SpeechTranscriber.supportedLocale(equivalentTo: wanted) else {
        return .failure(Status(UNSUPPORTED_LANGUAGE, "\(wanted.identifier) isn't supported by Apple's speech model"))
    }
    return .success(SpeechTranscriber(locale: supported, preset: .transcription))
}

struct Status: Error {
    let code: Int32
    let message: String
    init(_ code: Int32, _ message: String) {
        self.code = code
        self.message = message
    }
}

/// Reports whether speech assets for the locale are installed. On success
/// the text is the locale identifier used.
@_cdecl("splashdown_apple_stt_status")
public func splashdown_apple_stt_status(
    _ locale: UnsafePointer<CChar>?,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    guard #available(macOS 26.0, *) else {
        reply.send(OS_TOO_OLD, "Apple's speech model needs macOS 26 or later")
        return
    }
    let localeCopy = locale.map { strdup($0) }
    Task {
        defer { localeCopy.flatMap { free($0) } }
        switch await transcriber(localeCopy.map { UnsafePointer($0!) }) {
        case .failure(let s):
            reply.send(s.code, s.message)
        case .success(let t):
            let id = t.selectedLocales.first?.identifier ?? ""
            switch await AssetInventory.status(forModules: [t]) {
            case .installed: reply.send(OK, id)
            case .supported: reply.send(NEEDS_DOWNLOAD, id)
            case .downloading: reply.send(DOWNLOADING, id)
            case .unsupported: reply.send(UNSUPPORTED_LANGUAGE, id)
            @unknown default: reply.send(FAILED, id)
            }
        }
    }
}

/// Downloads and installs speech assets for the locale (from Apple).
@_cdecl("splashdown_apple_stt_prepare")
public func splashdown_apple_stt_prepare(
    _ locale: UnsafePointer<CChar>?,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    guard #available(macOS 26.0, *) else {
        reply.send(OS_TOO_OLD, "Apple's speech model needs macOS 26 or later")
        return
    }
    let localeCopy = locale.map { strdup($0) }
    Task {
        defer { localeCopy.flatMap { free($0) } }
        switch await transcriber(localeCopy.map { UnsafePointer($0!) }) {
        case .failure(let s):
            reply.send(s.code, s.message)
        case .success(let t):
            do {
                if let request = try await AssetInventory.assetInstallationRequest(supporting: [t]) {
                    try await request.downloadAndInstall()
                }
                reply.send(OK, "")
            } catch {
                reply.send(FAILED, error.localizedDescription)
            }
        }
    }
}

/// Transcribes an audio file (16 kHz mono WAV from the Rust side).
@_cdecl("splashdown_apple_transcribe")
public func splashdown_apple_transcribe(
    _ path: UnsafePointer<CChar>,
    _ locale: UnsafePointer<CChar>?,
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownCallback
) {
    let reply = Reply(ctx: ctx, callback: callback)
    let url = URL(fileURLWithPath: String(cString: path))
    guard #available(macOS 26.0, *) else {
        reply.send(OS_TOO_OLD, "Apple's speech model needs macOS 26 or later")
        return
    }
    let localeCopy = locale.map { strdup($0) }
    Task {
        defer { localeCopy.flatMap { free($0) } }
        let module: SpeechTranscriber
        switch await transcriber(localeCopy.map { UnsafePointer($0!) }) {
        case .failure(let s):
            reply.send(s.code, s.message)
            return
        case .success(let t):
            module = t
        }
        do {
            let file = try AVAudioFile(forReading: url)
            let analyzer = SpeechAnalyzer(modules: [module])
            // Collect results while the analyzer runs.
            let collector = Task {
                var parts: [String] = []
                for try await result in module.results {
                    let text = String(result.text.characters).trimmingCharacters(in: .whitespaces)
                    if !text.isEmpty { parts.append(text) }
                }
                return parts.joined(separator: " ")
            }
            if let end = try await analyzer.analyzeSequence(from: file) {
                try await analyzer.finalizeAndFinish(through: end)
            } else {
                await analyzer.cancelAndFinishNow()
            }
            reply.send(OK, try await collector.value)
        } catch {
            reply.send(FAILED, error.localizedDescription)
        }
    }
}
