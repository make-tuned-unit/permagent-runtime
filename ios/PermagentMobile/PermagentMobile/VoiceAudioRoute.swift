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

    /// `.videoChat` is speaker+mic. `.voiceChat` ducks TTS and used to win
    /// the receiver; `.default` plus forced speaker plus AEC muted the mic.
    static let sessionMode: AVAudioSession.Mode = .videoChat

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
    static func mustRebuildGraph(
        voiceProcessing: Bool,
        wantVoiceProcessing: Bool,
        captureSampleRate: Double,
        captureChannels: UInt32,
        liveSampleRate: Double,
        liveChannels: UInt32
    ) -> Bool {
        voiceProcessing != wantVoiceProcessing
            || captureSampleRate != liveSampleRate
            || captureChannels != liveChannels
    }

    /// Speakerphone runs without AEC, so TTS from the same speaker looks like
    /// speech. Ignore barge-in while playback is actually coming out.
    static let speakerPlaybackBargeFloor: Float = 0.05

    static func ignoreBargeIn(speakerphone: Bool, playbackRms: Float) -> Bool {
        speakerphone && playbackRms > speakerPlaybackBargeFloor
    }
}
