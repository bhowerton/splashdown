// Recording what the computer plays, for meeting notes: a Core Audio process
// tap on every app but this one (so Splashdown's own sounds stay out),
// wrapped in a private aggregate device, read as mono Float32 and handed to
// Rust as it arrives. macOS 14.2+; the first start asks the user for
// permission to record this computer's audio (NSAudioCaptureUsageDescription).

import AudioToolbox
import CoreAudio
import Foundation

/// Mono samples at `rate` Hz, called on an audio thread. The pointer is only
/// valid during the call; Rust copies it.
public typealias SplashdownAudioCallback = @convention(c) (
    UnsafeMutableRawPointer?, UnsafePointer<Float>?, Int32, Double
) -> Void

final class SystemTap {
    var tap = AudioObjectID(kAudioObjectUnknown)
    var aggregate = AudioObjectID(kAudioObjectUnknown)
    var proc: AudioDeviceIOProcID?

    func stop() {
        if let proc {
            AudioDeviceStop(aggregate, proc)
            AudioDeviceDestroyIOProcID(aggregate, proc)
        }
        proc = nil
        if aggregate != kAudioObjectUnknown {
            AudioHardwareDestroyAggregateDevice(aggregate)
            aggregate = AudioObjectID(kAudioObjectUnknown)
        }
        if #available(macOS 14.2, *), tap != kAudioObjectUnknown {
            AudioHardwareDestroyProcessTap(tap)
            tap = AudioObjectID(kAudioObjectUnknown)
        }
    }
}

private func address(_ selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
}

/// This process as a Core Audio object, to leave out of the tap.
private func ownProcessObject() -> AudioObjectID? {
    var pid = getpid()
    var object = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    var where_ = address(kAudioHardwarePropertyTranslatePIDToProcessObject)
    let status = AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &where_,
        UInt32(MemoryLayout<pid_t>.size), &pid, &size, &object)
    return status == noErr && object != kAudioObjectUnknown ? object : nil
}

private func defaultOutputUID() -> String? {
    var device = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    var where_ = address(kAudioHardwarePropertyDefaultSystemOutputDevice)
    guard AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &where_, 0, nil, &size, &device) == noErr
    else { return nil }
    var uid: Unmanaged<CFString>?
    size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
    where_ = address(kAudioDevicePropertyDeviceUID)
    guard AudioObjectGetPropertyData(device, &where_, 0, nil, &size, &uid) == noErr,
          let uid
    else { return nil }
    return uid.takeRetainedValue() as String
}

/// Starts recording the computer's sound. Returns a handle for
/// `splashdown_tap_stop`, or nil with `code` set: OS_TOO_OLD before macOS
/// 14.2, FAILED (with the Core Audio status in `detail`) otherwise.
@_cdecl("splashdown_tap_start")
public func splashdown_tap_start(
    _ ctx: UnsafeMutableRawPointer?,
    _ callback: SplashdownAudioCallback,
    _ code: UnsafeMutablePointer<Int32>,
    _ detail: UnsafeMutablePointer<Int32>
) -> UnsafeMutableRawPointer? {
    guard #available(macOS 14.2, *) else {
        code.pointee = OS_TOO_OLD
        return nil
    }
    let fail = { (status: OSStatus, recorder: SystemTap) -> UnsafeMutableRawPointer? in
        recorder.stop()
        code.pointee = FAILED
        detail.pointee = status
        return nil
    }
    let recorder = SystemTap()

    // 1. A tap on everything but us, still playing through the speakers.
    let excluded = ownProcessObject().map { [$0] } ?? []
    let description = CATapDescription(stereoGlobalTapButExcludeProcesses: excluded)
    description.uuid = UUID()
    description.name = "Splashdown meeting"
    description.isPrivate = true
    description.muteBehavior = .unmuted
    var status = AudioHardwareCreateProcessTap(description, &recorder.tap)
    guard status == noErr else { return fail(status, recorder) }

    // 2. A private aggregate device around it, clocked by the output device.
    var properties: [String: Any] = [
        kAudioAggregateDeviceNameKey: "Splashdown meeting",
        kAudioAggregateDeviceUIDKey: UUID().uuidString,
        kAudioAggregateDeviceIsPrivateKey: true,
        kAudioAggregateDeviceIsStackedKey: false,
        kAudioAggregateDeviceTapAutoStartKey: true,
        kAudioAggregateDeviceTapListKey: [
            [
                kAudioSubTapUIDKey: description.uuid.uuidString,
                kAudioSubTapDriftCompensationKey: true,
            ]
        ],
    ]
    if let output = defaultOutputUID() {
        properties[kAudioAggregateDeviceMainSubDeviceKey] = output
        properties[kAudioAggregateDeviceSubDeviceListKey] = [[kAudioSubDeviceUIDKey: output]]
    }
    status = AudioHardwareCreateAggregateDevice(properties as CFDictionary, &recorder.aggregate)
    guard status == noErr else { return fail(status, recorder) }

    // 3. The tap's format: its sample rate (the samples are Float32).
    var format = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    var where_ = address(kAudioTapPropertyFormat)
    status = AudioObjectGetPropertyData(recorder.tap, &where_, 0, nil, &size, &format)
    guard status == noErr else { return fail(status, recorder) }
    let rate = format.mSampleRate

    // 4. Read it: mix every channel of every buffer down to mono.
    status = AudioDeviceCreateIOProcIDWithBlock(&recorder.proc, recorder.aggregate, nil) {
        _, input, _, _, _ in
        let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
        var mono: [Float] = []
        var sources = 0
        for buffer in buffers {
            guard let data = buffer.mData else { continue }
            let channels = Int(max(buffer.mNumberChannels, 1))
            let count = Int(buffer.mDataByteSize) / MemoryLayout<Float>.size
            let frames = count / channels
            let samples = data.assumingMemoryBound(to: Float.self)
            if mono.count < frames {
                mono += [Float](repeating: 0, count: frames - mono.count)
            }
            for frame in 0..<frames {
                for channel in 0..<channels {
                    mono[frame] += samples[frame * channels + channel]
                }
            }
            sources += channels
        }
        guard sources > 0, !mono.isEmpty else { return }
        let scale = 1 / Float(sources)
        for i in mono.indices { mono[i] *= scale }
        mono.withUnsafeBufferPointer {
            callback(ctx, $0.baseAddress, Int32(mono.count), rate)
        }
    }
    guard status == noErr, let proc = recorder.proc else { return fail(status, recorder) }
    status = AudioDeviceStart(recorder.aggregate, proc)
    guard status == noErr else { return fail(status, recorder) }

    code.pointee = OK
    return Unmanaged.passRetained(recorder).toOpaque()
}

/// Stops recording and frees everything `splashdown_tap_start` made. No
/// callback runs after it returns.
@_cdecl("splashdown_tap_stop")
public func splashdown_tap_stop(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    let recorder = Unmanaged<SystemTap>.fromOpaque(handle).takeRetainedValue()
    recorder.stop()
}
