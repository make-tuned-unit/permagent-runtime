// VoiceVAD — the hands-free voice-activity state machine, extracted pure.
//
// This logic used to live inline in VoiceEngine.vadStep, where it read the
// wall clock itself and could only be exercised by talking at a phone. It is
// now a value type with the clock injected per step, so the turn lifecycle is
// unit-testable frame by frame (VoiceVADTests drives simulated 60-second
// conversations through it in milliseconds).
//
// Threshold provenance — these started as a copy of useVoice.ts, but the two
// capture paths do not hear the same numbers. iOS records through
// AVAudioEngine voice processing, whose AGC + noise suppression squash levels
// well below what a browser's getUserMedia delivers; the onset threshold was
// lowered accordingly (0.015 vs the web's 0.025) when the port was made.
// The keepalive floor, however, was copied verbatim at 0.010 — HIGHER,
// relative to onset, than the web runs it. Ordinary sustained speech on the
// phone dips under 0.010 between words and on soft syllables, so the trailing
// -silence window filled up mid-utterance and ended the turn while the user
// was still talking — the reported "orb cuts out at 5-10 seconds". Keepalive
// now preserves the web's onset:keepalive ratio (0.4) at iOS's onset level,
// and the trailing-silence window is widened to ride out a natural
// mid-thought pause, which on the phone (dictation pacing, arm's length)
// routinely exceeds the desktop's conversational 900 ms.

import Foundation

struct VoiceVAD {
    struct Config {
        /// RMS a frame must exceed to open a turn from `.ready`.
        var onset: Float = 0.015
        /// RMS that counts as "still speaking" while listening. 0.4 × onset —
        /// the same ratio useVoice.ts uses (0.010/0.025). The old absolute
        /// copy (0.010) sat at 0.67 × onset and read soft speech as silence;
        /// see the header comment for the premature-cutoff this caused.
        var keepalive: Float = 0.006
        /// Sustained level demanded (twice consecutively) for barge-in. Like
        /// keepalive before it, this was copied VERBATIM from useVoice.ts
        /// (0.05) while onset was lowered for iOS's AGC — leaving barge at
        /// 3.3× onset where the web runs 2× (0.05/0.025). Interrupting took a
        /// raised voice. Restored to the web's ratio at iOS's onset level.
        /// Safe against the agent's own voice bleeding into the mic because
        /// capture runs in `.voiceChat` mode, whose echo cancellation removes
        /// the speaker signal — and two consecutive frames are still required.
        var barge: Float = 0.03
        /// Trailing silence that completes a LONG turn. Wider than the web's
        /// 900 ms so a mid-thought dictation pause (~1.2 s) still survives,
        /// but tighter than the old 1.8 s — last night a finished utterance
        /// sat in Listening for a second too long after the speaker stopped.
        var silenceMs: Double = 1_400
        /// Trailing silence that completes a QUICK turn — one whose VOICED
        /// duration so far is under `quickTurnSpeechMs`. Short conversational
        /// asks ("what's on my board?") are almost always complete when the
        /// speaker stops. 2026-08-21: the 1.1 s window still felt late;
        /// 800 ms hands over without cutting a breath.
        var quickSilenceMs: Double = 800
        /// Voiced duration below which a turn counts as quick.
        var quickTurnSpeechMs: Double = 3_500
        /// Hard cap on one listening turn: a full minute.
        var maxTurnMs: Double = 60_000
        /// Consecutive over-`barge` frames required before interrupting.
        var bargeFrames: Int = 2
        /// Consecutive over-`onset` frames required to open a turn. A single
        /// room-noise spike last night auto-opened ~1.5 s recordings that
        /// came back as empty STT ("No speech detected").
        var onsetFrames: Int = 2
        /// Accumulated frames above keepalive before the turn is "real speech".
        /// Below this, trailing silence uses `abortSilenceMs` so a hiss pop
        /// does not sit in Listening for the full quick window.
        var minCommitMs: Double = 350
        /// Trailing silence that aborts an uncommitted (noise-only) turn.
        var abortSilenceMs: Double = 650

        init() {}
    }

    /// The engine states the VAD distinguishes. `.inactive` covers everything
    /// it never acts on (idle / connecting / failed).
    enum Phase { case ready, listening, thinking, speaking, inactive }

    enum Action: Equatable { case none, beginTurn, endTurn, interrupt }

    let config: Config
    private(set) var heardSpeech = false
    private var lastVoice: TimeInterval = 0
    private var turnStart: TimeInterval = 0
    private var bargeStreak = 0
    private var onsetStreak = 0
    private var lastStep: TimeInterval = 0
    private var voicedAccumMs: Double = 0

    init(config: Config = Config()) {
        self.config = config
    }

    /// Stamp the turn clocks for a turn begun OUTSIDE the VAD (push-to-talk).
    /// Turns the VAD itself opens are stamped inside `step` — do NOT call this
    /// for those, it would discard the onset frame's heard-speech mark.
    ///
    /// Without this stamp, a push-to-talk turn carried whatever `turnStart`
    /// the previous hands-free turn left behind, so the max-turn clause
    /// measured from a stale epoch and could end the turn on the very first
    /// frame after hands-free was re-enabled.
    mutating func noteTurnBegan(at now: TimeInterval) {
        turnStart = now
        lastVoice = now
        lastStep = now
        heardSpeech = false
        bargeStreak = 0
        onsetStreak = 0
        voicedAccumMs = 0
    }

    /// Clear per-turn state when a turn ends for any reason outside `step`
    /// (push-to-talk release, the hands-free toggle ending a live turn).
    mutating func noteTurnEnded() {
        heardSpeech = false
        bargeStreak = 0
        onsetStreak = 0
        voicedAccumMs = 0
    }

    // ── Route-aware presets ─────────────────────────────────────────────────
    //
    // The defaults were calibrated against a HEADSET mic at the mouth. The
    // phone's built-in mic — at arm's length, under .voiceChat's AGC + noise
    // suppression, with the loudspeaker's echo canceller running — delivers
    // ordinary speech well below the 0.015 onset, which is why "Listening"
    // heard a headset but not the bare iPhone (reported 2026-08-06). The
    // built-in preset halves every threshold while preserving the ratios the
    // header comment derives (keepalive = 0.4 × onset, barge = 2 × onset).

    /// The calibrated-default preset (headset / Bluetooth mic).
    static let headsetConfig = Config()

    /// The built-in iPhone mic preset.
    static var builtInMicConfig: Config {
        var c = Config()
        c.onset = 0.008
        c.keepalive = 0.0032
        c.barge = 0.016
        return c
    }

    /// Pick the preset for the session's current input route, by port type
    /// raw values (AVAudioSession.Port.builtInMic == "MicrophoneBuiltIn").
    /// `speakerphone` is the loudspeaker path (no headphones/BT/CarPlay): the
    /// built-in mic is quieter AND, with AEC off so the mic actually works,
    /// barge-in must be higher so TTS from the same speaker is not an interrupt.
    /// Pure so the chooser is unit-testable without an audio session.
    static func configForRoute(inputPortTypes: [String], speakerphone: Bool = false) -> Config {
        if speakerphone { return speakerphoneConfig }
        return inputPortTypes.contains(Self.builtInMicPort) ? builtInMicConfig : headsetConfig
    }

    /// `AVAudioSession.Port.builtInMic.rawValue`. Kept as a string so this
    /// file stays Foundation-only; VoiceVADTests pins it against the SDK.
    static let builtInMicPort = "MicrophoneBuiltIn"

    /// Loudspeaker + built-in mic. Same onset as the built-in preset so
    /// arm's-length speech still opens a turn; barge raised so playback
    /// bleed does not cut the agent off.
    static var speakerphoneConfig: Config {
        var c = builtInMicConfig
        c.barge = 0.055
        c.bargeFrames = 3
        // Room hiss on speakerphone last night sat at ~0.0032 — exactly the
        // built-in keepalive — and held Listening for 30–40 s (e.g. start
        // 23:47:10 → STT 23:47:51). Soft speech is ~0.008–0.010; 0.0055
        // lets speech refresh lastVoice and lets hiss fall through as silence.
        c.keepalive = 0.0055
        return c
    }

    /// Feed one mic frame's RMS. Returns the transition the engine should
    /// perform, if any.
    mutating func step(rms: Float, phase: Phase, now: TimeInterval) -> Action {
        switch phase {
        case .ready:
            bargeStreak = 0
            if rms > config.onset {
                onsetStreak += 1
                if onsetStreak >= config.onsetFrames {
                    onsetStreak = 0
                    heardSpeech = true
                    lastVoice = now
                    turnStart = now
                    lastStep = now
                    voicedAccumMs = 0
                    return .beginTurn
                }
            } else {
                onsetStreak = 0
            }
        case .listening:
            let frameMs = lastStep > 0 ? (now - lastStep) * 1_000 : 85
            lastStep = now
            if rms > config.keepalive {
                heardSpeech = true
                lastVoice = now
                voicedAccumMs += frameMs
            }
            let silentMs = (now - lastVoice) * 1_000
            let turnMs = (now - turnStart) * 1_000
            // Adaptive endpoint: a quick ask hands over fast; a dictation
            // keeps the patient window. Voiced duration (onset → last voice)
            // is the discriminator so the current silence never counts
            // against the classification. Uncommitted (noise-only) turns
            // abort faster so empty STT never sits in Listening.
            let voicedMs = (lastVoice - turnStart) * 1_000
            let committed = voicedAccumMs >= config.minCommitMs
            let window: Double
            if !committed {
                window = config.abortSilenceMs
            } else if voicedMs < config.quickTurnSpeechMs {
                window = config.quickSilenceMs
            } else {
                window = config.silenceMs
            }
            if (heardSpeech && silentMs > window) || turnMs > config.maxTurnMs {
                heardSpeech = false
                onsetStreak = 0
                return .endTurn
            }
        case .speaking, .thinking:
            // Barge-in demands a sustained loud signal so residual TTS bleed
            // can't cut the agent off — and, like the web hook, only fires
            // while actually speaking.
            if rms > config.barge {
                bargeStreak += 1
                if bargeStreak >= config.bargeFrames && phase == .speaking {
                    bargeStreak = 0
                    heardSpeech = false
                    return .interrupt
                }
            } else {
                bargeStreak = 0
            }
        case .inactive:
            break
        }
        return .none
    }
}
