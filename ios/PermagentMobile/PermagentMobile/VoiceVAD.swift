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
// now preserves the web's onset:keepalive ratio (0.4) at iOS's onset level.
//
// ENDPOINT WINDOWS (2026-08-25). Measured against the morning's session, the
// client holds `listening` for 800 ms (quick asks) or 1400 ms (anything with
// more than 3.5 s of voiced audio) after the speaker stops — before the
// daemon's clock even starts. Current practice puts a FIXED hangover at
// 300–800 ms, so the quick window comes down to 500 ms (OpenAI Realtime's
// `server_vad` default) and the noise-abort to 500 ms. The long window stays
// at 1400 ms on purpose: the agents that run 500 ms on long turns all pair the
// timer with a semantic end-of-turn model, and without one a shorter window
// cuts people off mid-thought — see the note on `silenceMs`. All three are
// overridable at runtime via `applyingDefaults` so they can be tuned by ear.

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
        /// Trailing silence that completes a LONG (dictation-length) turn.
        ///
        /// DELIBERATELY NOT CUT on 2026-08-25. It is 2–3× the 300–800 ms that
        /// shipped voice agents use, and it is the largest client-side share
        /// of "the app sits on listening too long" — but every one of those
        /// agents buys the short window back with a semantic end-of-turn model
        /// (LiveKit's Qwen2.5-0.5B EOU, Pipecat's Smart Turn v3), and we have
        /// no such signal. With a fixed timer alone, dropping this cuts people
        /// off mid-thought: `testNaturalPausesDoNotEndTheTurn` is the
        /// regression guard for exactly that reported bug, and a 1.2 s pause
        /// after four seconds of speech must survive. A false endpoint costs
        /// far more than 900 ms of latency. Tune by ear via `DefaultsKey`
        /// below; see docs/research/VOICE_LATENCY_AND_ORB_2026-08-25.md §3.
        var silenceMs: Double = 1_400
        /// Trailing silence that completes a QUICK turn — one whose VOICED
        /// duration so far is under `quickTurnSpeechMs`. Short conversational
        /// asks ("what's on my board?") are almost always complete when the
        /// speaker stops, and there is no mid-thought pause to protect.
        /// 500 ms is OpenAI Realtime's `server_vad` `silence_duration_ms`
        /// default and sits inside the 300–800 ms band LiveKit and Pipecat
        /// both land in. Was 800 ms.
        var quickSilenceMs: Double = 500
        /// Voiced duration below which a turn counts as quick. Held at 3.5 s:
        /// raising it would hand four- and five-second utterances — which DO
        /// contain mid-thought pauses — to the tight window above.
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
        /// Nothing was said, so there is no pause to protect. Was 650 ms.
        var abortSilenceMs: Double = 500

        init() {}
    }

    // ── Tuning knob ─────────────────────────────────────────────────────────
    //
    // The endpoint windows are the one part of this file worth turning by ear
    // on a real phone in a real room, so they are overridable at runtime
    // without a rebuild. Pure and clamped, so the override is unit-testable
    // and a fat-fingered value cannot wedge a turn open or cut every sentence
    // in half.

    /// UserDefaults keys for the three endpoint windows, in milliseconds.
    enum DefaultsKey {
        static let silenceMs = "voice.vad.silenceMs"
        static let quickSilenceMs = "voice.vad.quickSilenceMs"
        static let quickTurnSpeechMs = "voice.vad.quickTurnSpeechMs"
    }

    /// Accepted range for an overridden trailing-silence window. The floor is
    /// below anything research recommends but still long enough that a single
    /// inter-word gap cannot end a turn; the ceiling is the old default, so
    /// the knob can restore previous behaviour but not make it worse.
    static let silenceOverrideRange: ClosedRange<Double> = 200...1_800

    /// Accepted range for the quick/long classifier.
    static let quickTurnOverrideRange: ClosedRange<Double> = 1_000...20_000

    /// Apply any `voice.vad.*` overrides present in `defaults` on top of
    /// `base`, clamping each to its accepted range. An absent or non-numeric
    /// key leaves the compiled default alone.
    static func applyingDefaults(_ base: Config, defaults: UserDefaults) -> Config {
        var c = base
        func read(_ key: String, _ range: ClosedRange<Double>) -> Double? {
            guard defaults.object(forKey: key) != nil else { return nil }
            let v = defaults.double(forKey: key)
            guard v.isFinite, v > 0 else { return nil }
            return min(range.upperBound, max(range.lowerBound, v))
        }
        if let v = read(DefaultsKey.silenceMs, silenceOverrideRange) { c.silenceMs = v }
        if let v = read(DefaultsKey.quickSilenceMs, silenceOverrideRange) { c.quickSilenceMs = v }
        if let v = read(DefaultsKey.quickTurnSpeechMs, quickTurnOverrideRange) {
            c.quickTurnSpeechMs = v
        }
        // A quick window longer than the long window would invert the whole
        // point of the two tiers; hold the invariant regardless of input.
        c.quickSilenceMs = min(c.quickSilenceMs, c.silenceMs)
        return c
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
    static func configForRoute(
        inputPortTypes: [String],
        speakerphone: Bool = false,
        defaults: UserDefaults = .standard
    ) -> Config {
        let base: Config
        if speakerphone {
            base = speakerphoneConfig
        } else {
            base = inputPortTypes.contains(Self.builtInMicPort) ? builtInMicConfig : headsetConfig
        }
        return applyingDefaults(base, defaults: defaults)
    }

    /// `AVAudioSession.Port.builtInMic.rawValue`. Kept as a string so this
    /// file stays Foundation-only; VoiceVADTests pins it against the SDK.
    static let builtInMicPort = "MicrophoneBuiltIn"

    /// Loudspeaker + built-in mic. Same onset as the built-in preset so
    /// arm's-length speech still opens a turn; barge raised so playback
    /// bleed does not cut the agent off.
    static var speakerphoneConfig: Config {
        var c = builtInMicConfig
        // TTS bleed measured around 0.035 RMS. Keep headroom above it, then
        // let VoiceAudioRoute's playback-aware echo gate decide. The previous
        // 0.055 floor rejected ordinary interruption before that gate ran.
        c.barge = 0.04
        c.bargeFrames = 3
        // Room hiss on speakerphone last night sat at ~0.0032 — exactly the
        // built-in keepalive — and held Listening for 30–40 s (e.g. start
        // 23:47:10 → STT 23:47:51). Soft speech is ~0.008–0.010; 0.0055
        // lets speech refresh lastVoice and lets hiss fall through as silence.
        c.keepalive = 0.0055
        return c
    }

    /// Feed one mic frame's RMS. `voiceLike` is the spectral veto from
    /// `VoiceSpectrum.looksLikeVoice` — fail OPEN (`true`) when the analyser
    /// has nothing to judge, so a missing FFT cannot kill the orb.
    /// Kitchen music is loud enough to sit on keepalive but spectrally flat;
    /// those frames must not refresh `lastVoice` or they ride to `maxTurnMs`.
    mutating func step(
        rms: Float,
        phase: Phase,
        now: TimeInterval,
        voiceLike: Bool = true
    ) -> Action {
        switch phase {
        case .ready:
            bargeStreak = 0
            if rms > config.onset && voiceLike {
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
            if rms > config.keepalive && voiceLike {
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

/// Spectral voice/transient discriminator — port of
/// `ui/command-center/src/hooks/vadSpectrum.ts`.
///
/// Band AVERAGES, not dB sums. The web's first version summed analyser bytes
/// and required the low bins to hold 55% of that sum; on a dB scale the
/// non-voice bins swamp the voice bins and real speech scores ~0.3, so the
/// gate rejected essentially all speech. Comparing averages keeps the tilt
/// meaningful. Fails OPEN whenever there is nothing to judge.
enum VoiceSpectrum {
    /// How much louder (byte-dB) the voice band must be than the bright band.
    /// Close to 1.0: this is a VETO on obvious broadband, not a speech test.
    static let voiceTilt: Float = 1.12
    /// ~0–1.2 kHz at fftSize 64 on a 16 kHz frame.
    static let lowFraction: Float = 0.16
    /// Bright band start (~3 kHz+).
    static let highFraction: Float = 0.38
    static let fftSize = 64

    static func looksLikeVoice(_ data: [Float]) -> Bool {
        let n = data.count
        if n < 8 { return true }
        let lowEnd = max(1, Int((Float(n) * lowFraction).rounded()))
        let highStart = max(lowEnd + 1, Int((Float(n) * highFraction).rounded()))
        if highStart >= n { return true }
        var low: Float = 0
        for i in 0..<lowEnd { low += data[i] }
        var high: Float = 0
        for i in highStart..<n { high += data[i] }
        let lowAvg = low / Float(lowEnd)
        let highAvg = high / Float(n - highStart)
        if lowAvg <= 1 { return true }
        if highAvg <= 1 { return true }
        return lowAvg >= highAvg * voiceTilt
    }

    /// Map a 16 kHz PCM frame to 32 byte-scale bins (fftSize 64). Short
    /// frames return `[]`, which `looksLikeVoice` admits (fail-open).
    static func byteBins(samples: UnsafePointer<Float>, count: Int) -> [Float] {
        let nfft = fftSize
        guard count >= nfft else { return [] }
        let start = count - nfft
        var windowed = [Float](repeating: 0, count: nfft)
        for i in 0..<nfft {
            let hann = 0.5 - 0.5 * cos(2 * Float.pi * Float(i) / Float(nfft - 1))
            windowed[i] = samples[start + i] * hann
        }
        var bins = [Float](repeating: 0, count: nfft / 2)
        for k in 0..<(nfft / 2) {
            var re: Float = 0
            var im: Float = 0
            for n in 0..<nfft {
                let ang = 2 * Float.pi * Float(k) * Float(n) / Float(nfft)
                re += windowed[n] * Foundation.cos(ang)
                im -= windowed[n] * Foundation.sin(ang)
            }
            let mag = (re * re + im * im).squareRoot() / Float(nfft)
            let db = 20 * log10(mag + 1e-12)
            bins[k] = max(0, min(255, (255 / 70) * (db + 100)))
        }
        return bins
    }
}
