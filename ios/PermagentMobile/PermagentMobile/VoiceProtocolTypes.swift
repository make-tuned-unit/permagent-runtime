import Foundation

enum VoiceProtocolCapability {
    static let helloV1 = "hello_v1"
    static let turnTelemetryV2 = "turn_telemetry_v2"
    static let feedbackPlayback = "feedback_playback"
}

struct VoiceHelloInput: Codable, Equatable {
    let port_type: String?
    let port_name: String?
    let uid: String?
}

struct VoiceHelloMessage: Codable, Equatable {
    let type = "hello"
    let protocol_version: Int
    let client: String?
    let app_version: String?
    let build_number: String?
    let git_sha: String?
    let git_dirty: Bool?
    let capabilities: [String]?
    let audio_revision: Int?
    let inputs: [VoiceHelloInput]?
    let capture_sample_rate: Int?
    let capture_channels: Int?
    let wire_sample_rate: Int?
    let wire_channels: Int?
    let wire_encoding: String?
    let io_buffer_duration_ms: Int?
}

enum VoiceTurnTrigger: String, Codable, Equatable {
    case userTap = "user_tap"
    case vad
    case barge
    case unknown
}

enum VoiceTurnEndReason: String, Codable, Equatable {
    case silence
    case maxTurn = "max_turn"
    case userTap = "user_tap"
    case barge
    case noiseAbort = "noise_abort"
    case routeChange = "route_change"
    case interruption
    case clientError = "client_error"
    case unknown
}

enum VoiceEnrollmentControlAction: Equatable {
    case beginEnroll
    case skipEnroll
    case clearEnroll
}

func voiceEndReasonForEnrollmentAction(_ action: VoiceEnrollmentControlAction) -> VoiceTurnEndReason {
    switch action {
    case .beginEnroll, .skipEnroll, .clearEnroll:
        return .interruption
    }
}

struct VoiceStartMessage: Codable, Equatable {
    let type = "start"
    let sample_rate: Int
    let turn_id: String?
    let trigger: VoiceTurnTrigger?
    let audio_revision: Int?
}

struct VoiceStopMessage: Codable, Equatable {
    let type = "stop"
    let turn_id: String?
    let audio_revision: Int?
    let trigger: VoiceTurnTrigger?
    let end_reason: VoiceTurnEndReason?
    let capture_duration_ms: Int?
    let preroll_duration_ms: Int?
    let vad_noise_floor_rms: Float?
    let vad_onset_rms: Float?
    let vad_keepalive_rms: Float?
    let vad_committed: Bool?
    let vad_voiced_ms: Int?
    let vad_trailing_silence_ms: Int?
    let vad_silence_window_ms: Int?
    let vad_max_turn_ms: Int?
    let vad_max_keepalive_gap_ms: Int?
    let capture_frames: Int?
    let voiced_frames: Int?
    let dropped_input_samples: Int?
    let wire_samples_sent: Int?
    let send_failures: Int?
}

enum VoiceClientWireMessage: Codable, Equatable {
    case hello(VoiceHelloMessage)
    case start(VoiceStartMessage)
    case stop(VoiceStopMessage)
    case feedbackCancel(VoiceFeedbackCancelMessage)

    private enum CodingKeys: String, CodingKey {
        case type
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "hello":
            self = .hello(try VoiceHelloMessage(from: decoder))
        case "start":
            self = .start(try VoiceStartMessage(from: decoder))
        case "stop":
            self = .stop(try VoiceStopMessage(from: decoder))
        case "feedback_cancel":
            self = .feedbackCancel(try VoiceFeedbackCancelMessage(from: decoder))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "unsupported client wire type"
            )
        }
    }

    func encode(to encoder: Encoder) throws {
        switch self {
        case .hello(let hello):
            try hello.encode(to: encoder)
        case .start(let start):
            try start.encode(to: encoder)
        case .stop(let stop):
            try stop.encode(to: encoder)
        case .feedbackCancel(let feedbackCancel):
            try feedbackCancel.encode(to: encoder)
        }
    }
}

enum VoiceTurnOutcome: String, Codable, Equatable {
    case captureRejectedMalformed = "capture_rejected_malformed"
    case captureRejectedShort = "capture_rejected_short"
    case emptyStt = "empty_stt"
    case sttBusy = "stt_busy"
}

enum VoiceTurnOutcomeReason: String, Codable, Equatable {
    case malformedPcm = "malformed_pcm"
    case shortCapture = "short_capture"
    case zeroPcm = "zero_pcm"
    case nearSilentPcm = "near_silent_pcm"
    case finiteSignalNoWords = "finite_signal_no_words"
    case sttBusy = "stt_busy"
}

struct VoiceTurnOutcomeMessage: Codable, Equatable {
    let type = "turn_outcome"
    let outcome: VoiceTurnOutcome
    let reason: VoiceTurnOutcomeReason
}

struct VoiceFeedbackStartMessage: Codable, Equatable {
    let type = "feedback_start"
    let turn_id: String?
}

struct VoiceFeedbackEndMessage: Codable, Equatable {
    let type = "feedback_end"
    let turn_id: String?
    let sample_rate: Int?
    let cancelled: Bool?
}

struct VoiceFeedbackCancelMessage: Codable, Equatable {
    let type = "feedback_cancel"
    let turn_id: String?
}

struct VoiceIdleMessage: Codable, Equatable {
    let type = "idle"
}

enum VoiceServerWireMessage: Codable, Equatable {
    case turnOutcome(VoiceTurnOutcomeMessage)
    case feedbackStart(VoiceFeedbackStartMessage)
    case feedbackEnd(VoiceFeedbackEndMessage)
    case idle(VoiceIdleMessage)

    private enum CodingKeys: String, CodingKey {
        case type
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "turn_outcome":
            self = .turnOutcome(try VoiceTurnOutcomeMessage(from: decoder))
        case "feedback_start":
            self = .feedbackStart(try VoiceFeedbackStartMessage(from: decoder))
        case "feedback_end":
            self = .feedbackEnd(try VoiceFeedbackEndMessage(from: decoder))
        case "idle":
            self = .idle(try VoiceIdleMessage(from: decoder))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "unsupported server wire type"
            )
        }
    }

    func encode(to encoder: Encoder) throws {
        switch self {
        case .turnOutcome(let message):
            try message.encode(to: encoder)
        case .feedbackStart(let message):
            try message.encode(to: encoder)
        case .feedbackEnd(let message):
            try message.encode(to: encoder)
        case .idle(let message):
            try message.encode(to: encoder)
        }
    }
}

enum VoiceCapturePolicy {
    static let automaticMaxTurnMS: Double = 15_000
    static let userMaxTurnMS: Double = 60_000
    static let automaticRearmQuietMS: Double = 500

    static func maxTurnMS(trigger: VoiceTurnTrigger?) -> Double {
        switch trigger {
        case .vad, .barge:
            return automaticMaxTurnMS
        case .userTap, .unknown, nil:
            return userMaxTurnMS
        }
    }
}

enum VoiceFeedbackStartDisposition: Equatable {
    case accept
    case cancel
}

func voiceFeedbackStartDisposition(
    feedbackTurnID: String?,
    expectedTurnID: String?,
    activeListeningTurnID: String?
) -> VoiceFeedbackStartDisposition {
    guard let feedbackTurnID, !feedbackTurnID.isEmpty else { return .cancel }
    guard feedbackTurnID == expectedTurnID else { return .cancel }
    if let activeListeningTurnID, activeListeningTurnID != feedbackTurnID {
        return .cancel
    }
    return .accept
}

enum VoiceAutomaticQuietRearmer {
    static func shouldRearm(
        waiting: Bool,
        lastVoiceLikeAt: TimeInterval?,
        now: TimeInterval,
        quietMS: Double
    ) -> Bool {
        guard waiting, now.isFinite, quietMS >= 0 else { return false }
        guard let lastVoiceLikeAt else { return true }
        return (now - lastVoiceLikeAt) * 1_000.0 >= quietMS
    }
}

/// Playback completions must represent audible drain, not merely that the
/// player accepted/consumed a buffer. VoiceView maps this contract to
/// AVAudioPlayerNode's `.dataPlayedBack` callback without making the
/// Foundation-only test target import AVFoundation.
enum VoicePlaybackDrainPolicy {
    static let waitsForPlayback = true
}

/// Why a live reply is being interrupted. A user tap is an explicit
/// cancellation and must close the hub stream immediately. A VAD barge-in is
/// only a local signal; route-level echo filtering decides whether it reaches
/// cancellation at all.
enum VoiceReplyInterruptSource: Equatable {
    case explicitUser
    case automaticBargeIn
}

enum VoiceReplyCancellationPlan: Equatable {
    case immediate
    case localOnly
}

enum VoiceReplyCancellationPolicy {
    static func plan(
        source: VoiceReplyInterruptSource,
        replyEnded: Bool
    ) -> VoiceReplyCancellationPlan {
        switch source {
        case .explicitUser:
            return .immediate
        case .automaticBargeIn:
            return replyEnded ? .localOnly : .immediate
        }
    }
}

/// Deterministic reply-frame fence used by VoiceEngine across interruption
/// and reconnect. Late binary data from a canceled socket must never revive
/// `.speaking` or leak into the next capture.
struct VoiceReplyLifecycle: Equatable {
    enum Phase: Equatable {
        case idle
        case active
        case complete
    }

    private(set) var phase: Phase = .idle
    private(set) var generation = 0

    var acceptsAudio: Bool { phase == .active }

    mutating func beginTurn() {
        generation &+= 1
        phase = .active
    }

    mutating func requestCancellation(
        source: VoiceReplyInterruptSource,
        replyEnded: Bool
    ) -> VoiceReplyCancellationPlan {
        let plan = VoiceReplyCancellationPolicy.plan(
            source: source,
            replyEnded: replyEnded
        )
        switch plan {
        case .immediate, .localOnly:
            phase = .complete
        }
        return plan
    }

    mutating func receiveReplyEnd() {
        guard phase == .active else { return }
        phase = .complete
    }

    mutating func receiveTerminalWithoutReply() {
        guard phase == .active else { return }
        phase = .complete
    }

    mutating func invalidate() {
        generation &+= 1
        phase = .idle
    }
}

/// Identifies one AVAudioPlayerNode scheduling lifetime. A completion from a
/// prior route/reconnect generation must not drain a new reply's counter.
struct VoicePlaybackEpoch: Equatable {
    private(set) var value = 0

    mutating func advance() {
        value &+= 1
    }

    func accepts(_ callbackValue: Int) -> Bool {
        callbackValue == value
    }
}

/// A turn can finish without a reply while the socket is still usable. Keep
/// this distinction explicit so the UI can give the user a recoverable,
/// terminal explanation instead of leaving an apparently-live listening
/// screen with no evidence that the turn ended.
enum VoiceTurnFeedback: Equatable {
    case none
    case emptyCapture
    case connectionLost
    case error(String)

    var message: String? {
        switch self {
        case .none: return nil
        case .emptyCapture: return "I didn’t catch that — try again."
        case .connectionLost: return "Connection lost — reconnecting to your hub."
        case .error(let detail): return detail.isEmpty ? "Voice stopped — try again." : detail
        }
    }

    var isRecoverable: Bool {
        switch self {
        case .none: return false
        case .emptyCapture, .connectionLost, .error: return true
        }
    }
}

struct VoiceTranscriptBuffer {
    private(set) var partial = ""
    private(set) var final = ""

    var displayText: String { final.isEmpty ? partial : final }

    mutating func reset() {
        partial = ""
        final = ""
    }

    mutating func acceptPartial(_ text: String) {
        guard final.isEmpty else { return }
        partial = text
    }

    mutating func acceptFinal(_ text: String) {
        final = text
        partial = ""
    }
}

struct VoiceReplyWordTiming: Decodable, Equatable {
    let word: String
    let startMS: Int
    let endMS: Int
    let rangeLocation: Int?
    let rangeLength: Int?

    private enum CodingKeys: String, CodingKey {
        case word, text, token
        case startMS = "start_ms"
        case endMS = "end_ms"
        case start, end
        case startTimeMS = "start_time_ms"
        case endTimeMS = "end_time_ms"
        case range
        case utf16Start = "utf16_start"
        case utf16Length = "utf16_length"
        case startUTF16 = "start_utf16"
        case endUTF16 = "end_utf16"
    }

    private struct RangeObject: Decodable {
        let location: Int?
        let length: Int?
        let start: Int?
        let end: Int?
    }

    init(word: String, startMS: Int, endMS: Int, rangeLocation: Int? = nil, rangeLength: Int? = nil) {
        self.word = word
        self.startMS = startMS
        self.endMS = endMS
        self.rangeLocation = rangeLocation
        self.rangeLength = rangeLength
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        word = (try? c.decode(String.self, forKey: .word))
            ?? (try? c.decode(String.self, forKey: .text))
            ?? (try? c.decode(String.self, forKey: .token))
            ?? ""
        startMS = (try? c.decode(Int.self, forKey: .startMS))
            ?? (try? c.decode(Int.self, forKey: .startTimeMS))
            ?? (try? c.decode(Int.self, forKey: .start))
            ?? 0
        endMS = (try? c.decode(Int.self, forKey: .endMS))
            ?? (try? c.decode(Int.self, forKey: .endTimeMS))
            ?? (try? c.decode(Int.self, forKey: .end))
            ?? startMS
        if let start = (try? c.decode(Int.self, forKey: .startUTF16)) ?? (try? c.decode(Int.self, forKey: .utf16Start)) {
            rangeLocation = start
            rangeLength = (try? c.decode(Int.self, forKey: .endUTF16)).map { max(0, $0 - start) } ?? (try? c.decode(Int.self, forKey: .utf16Length))
        } else if let object = try? c.decode(RangeObject.self, forKey: .range) {
            rangeLocation = object.location ?? object.start
            if let length = object.length {
                rangeLength = length
            } else if let start = object.start, let end = object.end {
                rangeLength = max(0, end - start)
            } else {
                rangeLength = nil
            }
        } else if let values = try? c.decode([Int].self, forKey: .range), values.count >= 2 {
            rangeLocation = values[0]; rangeLength = values[1]
        } else {
            rangeLocation = nil
            rangeLength = nil
        }
    }
}

struct VoiceReplySegment: Decodable, Equatable {
    let segmentID: String
    let text: String
    let sampleRate: Int
    let durationMS: Int
    let words: [VoiceReplyWordTiming]

    private enum CodingKeys: String, CodingKey {
        case segmentID = "segment_id"
        case text
        case sampleRate = "sample_rate"
        case durationMS = "duration_ms"
        case words
        case wordTimings = "word_timings"
        case timings
    }

    init(
        segmentID: String,
        text: String,
        sampleRate: Int,
        durationMS: Int,
        words: [VoiceReplyWordTiming] = []
    ) {
        self.segmentID = segmentID
        self.text = text
        self.sampleRate = sampleRate
        self.durationMS = durationMS
        self.words = words
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        segmentID = try c.decode(VoiceWireSegmentID.self, forKey: .segmentID).value
        text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        sampleRate = try c.decodeIfPresent(Int.self, forKey: .sampleRate) ?? 24_000
        durationMS = try c.decodeIfPresent(Int.self, forKey: .durationMS) ?? 0
        words = (try? c.decode([VoiceReplyWordTiming].self, forKey: .words))
            ?? (try? c.decode([VoiceReplyWordTiming].self, forKey: .wordTimings))
            ?? (try? c.decode([VoiceReplyWordTiming].self, forKey: .timings))
            ?? []
    }

    var durationSeconds: Double { Double(max(0, durationMS)) / 1_000.0 }

    /// If the hub supplies timing without ranges, infer each token's UTF-16
    /// range by searching forward through the segment text.
    func displayRanges() -> [NSRange] {
        let nsText = text as NSString
        var searchLocation = 0
        return words.map { timing in
            if let location = timing.rangeLocation,
               let length = timing.rangeLength,
               location >= 0,
               length >= 0,
               location + length <= nsText.length {
                searchLocation = max(searchLocation, location + length)
                return NSRange(location: location, length: length)
            }
            let needle = timing.word as NSString
            guard needle.length > 0, searchLocation <= nsText.length else {
                return NSRange(location: 0, length: 0)
            }
            let range = nsText.range(
                of: needle as String,
                options: [],
                range: NSRange(location: searchLocation, length: nsText.length - searchLocation)
            )
            if range.location != NSNotFound {
                searchLocation = range.location + range.length
            }
            return range.location == NSNotFound ? NSRange(location: 0, length: 0) : range
        }
    }

    /// Timing is optional. When supplied, it must be internally consistent;
    /// malformed metadata disables highlighting while preserving readable text.
    var hasValidTimings: Bool {
        guard durationMS > 0, !words.isEmpty else { return false }
        let ranges = displayRanges()
        guard ranges.count == words.count else { return false }
        var previousEnd = 0
        for (index, timing) in words.enumerated() {
            guard timing.startMS >= 0, timing.endMS > timing.startMS,
                  timing.endMS <= durationMS else { return false }
            let range = ranges[index]
            guard range.location >= 0, range.length > 0,
                  NSMaxRange(range) <= (text as NSString).length,
                  range.location >= previousEnd else { return false }
            previousEnd = NSMaxRange(range)
        }
        return true
    }
}

struct VoiceWireSegmentID: Decodable, Equatable {
    let value: String

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let string = try? c.decode(String.self) {
            value = string
        } else if let unsigned = try? c.decode(UInt64.self) {
            value = String(unsigned)
        } else if let signed = try? c.decode(Int64.self) {
            value = String(signed)
        } else {
            throw DecodingError.typeMismatch(
                String.self,
                DecodingError.Context(
                    codingPath: decoder.codingPath,
                    debugDescription: "segment_id must be a string or integer"
                )
            )
        }
    }
}

/// Pure framing guard used by VoiceEngine: one metadata frame must precede one
/// PCM frame, and a broken sequence disables timing without losing audio/text.
struct VoiceReplySegmentQueue {
    private(set) var pendingCount = 0
    private(set) var timingDisabled = false
    private var seen = Set<String>()
    private var lastNumericID: Int?

    mutating func reset() {
        pendingCount = 0
        timingDisabled = false
        seen.removeAll(keepingCapacity: true)
        lastNumericID = nil
    }

    mutating func acceptMetadata(segmentID: String) -> Bool {
        if seen.contains(segmentID) { timingDisabled = true }
        if let number = Int(segmentID), let lastNumericID, number <= lastNumericID {
            timingDisabled = true
        }
        if let number = Int(segmentID) { lastNumericID = number }
        seen.insert(segmentID)
        pendingCount += 1
        return !timingDisabled
    }

    mutating func consumePCM() -> Bool {
        guard pendingCount > 0 else {
            timingDisabled = true
            return false
        }
        pendingCount -= 1
        return !timingDisabled
    }
}

/// Re-find ordered segment text in the authoritative reply. Searching from
/// the prior match keeps repeated segments deterministic and returns nil when
/// the server text cannot safely support a highlight.
func voiceReplyGlobalRanges(segmentTexts: [String], in authoritative: String) -> [NSRange]? {
    let nsText = authoritative as NSString
    var cursor = 0
    var result: [NSRange] = []
    for text in segmentTexts {
        let needle = text as NSString
        guard needle.length > 0, cursor <= nsText.length else { return nil }
        let range = nsText.range(of: text, options: [], range: NSRange(location: cursor, length: nsText.length - cursor))
        guard range.location != NSNotFound else { return nil }
        result.append(range)
        cursor = NSMaxRange(range)
    }
    return result
}

/// Selects a word at a playback-relative position. VoiceEngine uses this pure
/// helper from a cancellable display task, so highlighting follows playback
/// rather than message-delivery time.
func voiceReplyWordIndex(at elapsedMS: Int, in segment: VoiceReplySegment) -> Int? {
    guard segment.hasValidTimings else { return nil }
    let clamped = max(0, elapsedMS)
    if let index = segment.words.firstIndex(where: {
        clamped >= $0.startMS && clamped < max($0.endMS, $0.startMS + 1)
    }) {
        return index
    }
    return nil
}
