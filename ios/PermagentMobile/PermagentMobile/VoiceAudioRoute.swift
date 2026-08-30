// VoiceAudioRoute — speakerphone vs headphones capture policy, extracted pure.
//
// VoiceEngine used to force `.speaker` and enable voice-processing AEC on
// every route. That pair is how iOS echo-cancels a loudspeaker sitting next
// to the built-in mic — and it cancelled the *user* too ("Listening", orb
// dead). The same forced-speaker override stole AirPods that were already
// connected. The decisions live here so a unit test can name both bugs
// without opening a microphone.

import AVFoundation
import Foundation

enum VoiceTransport {
    /// URLSessionWebSocketTask defaults to a 1 MiB incoming message. A 15.7 s
    /// Float32 TTS chunk in the 2026-08-28 session was ~1.5 MiB and iOS closed
    /// the otherwise healthy conversation with code 1009. Eight MiB carries
    /// about 87 seconds at 24 kHz mono while still bounding one server frame.
    static let maximumIncomingMessageBytes = 8 * 1024 * 1024
}

enum VoiceAudioRoute {
    /// Output ports that mean "do not use the loudspeaker". Taken from the
    /// SDK so a rename cannot silently drop AirPods back onto speakerphone.
    static var externalOutputPorts: Set<String> {
        [
            AVAudioSession.Port.headphones.rawValue,
            AVAudioSession.Port.bluetoothA2DP.rawValue,
            AVAudioSession.Port.bluetoothHFP.rawValue,
            AVAudioSession.Port.bluetoothLE.rawValue,
            AVAudioSession.Port.carAudio.rawValue,
        ]
    }

    /// `.default` plus route-aware AEC. `.voiceChat` ducks TTS and used to
    /// win the receiver. `.videoChat` turns on *session*-level AEC, which
    /// muted the near-end mic on speaker AND dropped Bluetooth HFP — the
    /// 2026-08-21 rebuild that switched to it connected to /voice but never
    /// sent a recording start (hub: three sockets, zero `Recording started`).
    static let sessionMode: AVAudioSession.Mode = .default

    enum OutputOverride: Equatable {
        /// Built-in loudspeaker (speakerphone).
        case speaker
        /// Leave the current route alone (headphones / BT / CarPlay).
        case none
    }

    struct Policy: Equatable {
        /// No headset/car output — capture is the phone's own speaker+mic.
        var speakerphone: Bool
    /// AEC. MUST be false on speakerphone (it mutes the near-end mic)
    /// and true on a headset (playback is in the ears).
    ///
    /// N2 (kitchen-voice): one-way dictation uses `.spokenAudio` for Voice
    /// Isolation. The orb speakerphone path stays VP-off until a real iPhone
    /// measurement shows setVoiceProcessingEnabled(true) still yields RMS
    /// above onset (the 2026-08-21 mute). Do not flip this flag without that.
        var voiceProcessing: Bool
        var outputOverride: OutputOverride
    }

    static func isExternalOutput(_ outputPortTypes: [String]) -> Bool {
        outputPortTypes.contains { externalOutputPorts.contains($0) }
    }

    static func policy(outputPortTypes: [String]) -> Policy {
        let headset = isExternalOutput(outputPortTypes)
        return Policy(
            speakerphone: !headset,
            voiceProcessing: headset,
            outputOverride: headset ? .none : .speaker
        )
    }

    /// MicPipe is built for one input format; voice processing also re-formats
    /// the node. Either change without a graph rebuild drops every subsequent
    /// tap buffer (rms 0 — "Listening" again).
    ///
    /// A live rate of 0 is the session still settling after setCategory /
    /// speaker override — NOT a real format change. Tearing the graph down
    /// in that window is how a working tap became a dead one: startAudio
    /// then threw unusableInputFormat, the error was swallowed, and the orb
    /// stayed on LISTENING with no mic.
    static func mustRebuildGraph(
        voiceProcessing: Bool,
        wantVoiceProcessing: Bool,
        captureSampleRate: Double,
        captureChannels: UInt32,
        liveSampleRate: Double,
        liveChannels: UInt32
    ) -> Bool {
        guard liveSampleRate > 0, liveChannels > 0 else { return false }
        return voiceProcessing != wantVoiceProcessing
            || captureSampleRate != liveSampleRate
            || captureChannels != liveChannels
    }

    /// Speakerphone runs without AEC, so TTS from the same speaker looks like
    /// speech. Ignore barge-in only when the mic is NOT clearly louder than
    /// the expected acoustic echo — a blanket ignore made him uninterruptible.
    /// Best practice (2026 duplex agents): stop immediately on a real barge,
    /// never on his own echo. Headphones keep AEC and never ignore.
    static let speakerPlaybackBargeFloor: Float = 0.05
    /// The playback tap is measured before loudspeaker → room → microphone
    /// attenuation. Live traces put 0.2 playback near 0.03 mic echo; comparing
    /// the two values directly required an impossible shout over the digital
    /// source rather than over the sound actually reaching the microphone.
    static let speakerPlaybackToMicGain: Float = 0.15

    static func ignoreBargeIn(
        speakerphone: Bool,
        playbackRms: Float,
        micRms: Float = 0
    ) -> Bool {
        guard speakerphone else { return false }
        guard playbackRms > speakerPlaybackBargeFloor else { return false }
        let expectedEcho = playbackRms * speakerPlaybackToMicGain
        let floor = max(speakerPlaybackBargeFloor, expectedEcho * 1.75)
        return micRms <= floor
    }
}
