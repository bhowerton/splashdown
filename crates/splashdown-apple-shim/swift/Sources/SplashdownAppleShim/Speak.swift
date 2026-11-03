// Reading text aloud with the system's voices ("read this to me", the
// Ask panel's speaker button). Plain synchronous calls: the reading goes on
// in the background, and Rust asks whether it's still going.

import AVFAudio
import Foundation

/// Knows which utterance is being read, so a reading that was stopped for
/// a new one doesn't count as the new one finishing.
private final class Watcher: NSObject, AVSpeechSynthesizerDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var current: AVSpeechUtterance?

    func started(_ utterance: AVSpeechUtterance?) {
        lock.lock()
        current = utterance
        lock.unlock()
    }

    var busy: Bool {
        lock.lock()
        defer { lock.unlock() }
        return current != nil
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        ended(utterance)
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        ended(utterance)
    }

    private func ended(_ utterance: AVSpeechUtterance) {
        lock.lock()
        if current === utterance {
            current = nil
        }
        lock.unlock()
    }
}

private let watcher = Watcher()
private let synthesizer: AVSpeechSynthesizer = {
    let synthesizer = AVSpeechSynthesizer()
    synthesizer.delegate = watcher
    return synthesizer
}()

/// Novelty voices ("Bells", "Bad News") and the user's Personal Voice
/// aren't for reading text back.
private func readable(_ voice: AVSpeechSynthesisVoice) -> Bool {
    if #available(macOS 14.0, *) {
        return !voice.voiceTraits.contains(.isNoveltyVoice) && !voice.voiceTraits.contains(.isPersonalVoice)
    }
    return true
}

private func speaks(_ voice: AVSpeechSynthesisVoice, _ language: String) -> Bool {
    voice.language == language || voice.language.hasPrefix(language + "-")
}

/// The chosen voice when it speaks `language` (or the language isn't
/// known); otherwise the best voice installed for the language: premium
/// before enhanced before the rest, then one for the user's own region.
private func pickVoice(_ id: String, _ language: String) -> AVSpeechSynthesisVoice? {
    if !id.isEmpty, let chosen = AVSpeechSynthesisVoice(identifier: id),
       language.isEmpty || speaks(chosen, language) {
        return chosen
    }
    if language.isEmpty {
        return nil
    }
    let region = Locale.current.region?.identifier ?? ""
    let fallback = AVSpeechSynthesisVoice(language: language)
    let score = { (voice: AVSpeechSynthesisVoice) -> Int in
        var score = voice.quality.rawValue * 10
        if !region.isEmpty && voice.language.hasSuffix("-" + region) { score += 5 }
        if voice.identifier == fallback?.identifier { score += 1 }
        return score
    }
    let voices = AVSpeechSynthesisVoice.speechVoices().filter { speaks($0, language) && readable($0) }
    return voices.max { score($0) < score($1) } ?? fallback
}

/// Reads `text` aloud, stopping anything being read. `voice` is a voice's
/// identifier ("" for none chosen), `language` the text's language ("de",
/// "" when unknown), `speed` 1.0 for the voice's usual pace.
@_cdecl("splashdown_speak")
public func splashdown_speak(
    _ text: UnsafePointer<CChar>,
    _ voice: UnsafePointer<CChar>,
    _ language: UnsafePointer<CChar>,
    _ speed: Double
) {
    let utterance = AVSpeechUtterance(string: String(cString: text))
    utterance.voice = pickVoice(String(cString: voice), String(cString: language))
    // The rate runs from 0 to 1 with 0.5 as usual, and 1 is far too fast to
    // follow, so faster speeds go up more gently than slower ones go down.
    let usual = AVSpeechUtteranceDefaultSpeechRate
    let rate = speed >= 1 ? usual + Float(speed - 1) * 0.25 : usual * Float(speed)
    utterance.rate = min(max(rate, AVSpeechUtteranceMinimumSpeechRate), AVSpeechUtteranceMaximumSpeechRate)
    synthesizer.stopSpeaking(at: .immediate)
    watcher.started(utterance)
    synthesizer.speak(utterance)
}

/// Stops reading at once.
@_cdecl("splashdown_speak_stop")
public func splashdown_speak_stop() {
    watcher.started(nil)
    synthesizer.stopSpeaking(at: .immediate)
}

/// Whether something is still being read.
@_cdecl("splashdown_speaking")
public func splashdown_speaking() -> Bool {
    watcher.busy
}

/// The voices that can read text, one per line: identifier, name,
/// language and quality (1 usual, 2 enhanced, 3 premium), tab-separated.
/// Replies at once.
@_cdecl("splashdown_voices")
public func splashdown_voices(_ ctx: UnsafeMutableRawPointer?, _ callback: SplashdownCallback) {
    let lines = AVSpeechSynthesisVoice.speechVoices()
        .filter(readable)
        .map { "\($0.identifier)\t\($0.name)\t\($0.language)\t\($0.quality.rawValue)" }
    Reply(ctx: ctx, callback: callback).send(OK, lines.joined(separator: "\n"))
}
