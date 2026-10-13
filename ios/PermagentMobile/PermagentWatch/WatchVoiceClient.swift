// Standalone spoken conversation from the wrist: the watch speaks the hub's
// `/voice` WebSocket itself, so the phone may be in a pocket or switched off
// (Astra ruling 2026-10-13, slice "Wi-Fi wrist conversation").
//
// It mirrors the iPhone client in VoiceView.swift, reduced to tap-to-talk +
// silence-to-submit (protocol: crates/goose-server/src/routes/voice.rs):
//   connect  wss://<gateway>/voice?session_id=<conversation>&client=watch_voice
//            Authorization: Bearer <watch voice token>   (TLS pinned by fingerprint)
//   server → {"type":"ready"}
//   client → {"type":"hello",…,"client":"watch_voice","wire_encoding":"pcm_f32le"}
//            {"type":"start","sample_rate":16000,"turn_id":"watch-N","trigger":"user_tap"}
//            binary: Float32 LE mono PCM @ 16 kHz
//            {"type":"stop","turn_id":"watch-N","end_reason":"silence",…}
//   server → transcript_partial / transcript / reply_start /
//            audio_segment (+ one binary Float32 LE @ 24 kHz frame each) /
//            reply_text / reply_end | turn_outcome | idle | error
//
// States: Connecting → Ready → Listening → Transcribing → Thinking →
// Speaking → Ready. The mic reopens only after `reply_end` AND the last
// reply buffer has been rendered. A dropped socket never resubmits a turn
// the hub may already be running; it reconnects and waits for a tap.

import AVFoundation
import CryptoKit
import Foundation
import os

private let watchVoiceLog = Logger(subsystem: "ai.permagent.watch", category: "voice")

@MainActor
final class WatchVoiceClient: ObservableObject {
    @Published private(set) var phase: WatchVoicePhase = .idle
    @Published private(set) var transcript = ""
    @Published private(set) var reply = ""
    @Published private(set) var toast: String?
    @Published private(set) var level: Double = 0
    /// Direct voice gave up (timeouts / refused). The view offers the relay.
    @Published private(set) var directUnavailable = false

    private var credential: WatchVoiceCredential?
    private var active = false
    private var autoListenOnReady = false

    // Socket
    private var urlSession: URLSession?
    private var socket: URLSessionWebSocketTask?
    /// Bumps on every connect and close; callbacks from an older socket are dropped.
    private var epoch = 0
    private var reconnects = 0
    private var connectTimer: Task<Void, Never>?
    /// An `error` frame that arrived before `ready` (e.g. the hub's voice
    /// models are not loaded): the close that follows is not transient.
    private var preReadyError: String?

    // Audio
    private var audioReady = false
    private var engine: AVAudioEngine?
    private var player: AVAudioPlayerNode?
    private var pipe: WatchMicPipe?
    private var tapInstalled = false
    private let playbackFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 24_000,
                                               channels: 1, interleaved: false)!
    private var gate = WatchReplyGate()
    private var playbackGeneration = 0
    private var replyMuted = false
    private var observers: [NSObjectProtocol] = []

    // Turn
    private var turnSeq = 0
    private var listeningTurnID: String?
    private var submittedTurnID: String?
    private var replyAuthoritative = false
    private var heardSpeech = false
    private var spokenFor: TimeInterval = 0
    private var silentFor: TimeInterval = 0
    private var captureElapsed: TimeInterval = 0
    private var wireSamplesSent = 0
    private var captureTimer: Task<Void, Never>?
    private var progressTimer: Task<Void, Never>?
    private var lastProgress = Date()
    private var stopSentAt = Date()

    /// No speech at all for this long ends the listen (as an empty turn)
    /// instead of streaming silence for the full 45 s.
    private static let noSpeechTimeout: TimeInterval = 12
    /// dBFS above which a 16 kHz frame counts as voiced.
    private static let voicedDB: Float = -42

    // MARK: Public

    func start(credential: WatchVoiceCredential) {
        guard !active else { return }
        active = true
        self.credential = credential
        directUnavailable = false
        reconnects = 0
        toast = nil
        transcript = ""
        reply = ""
        autoListenOnReady = true
        phase = .connecting
        Task { await self.prepareAudioThenConnect() }
    }

    /// The explicit Finish control (and leaving the screen): close the socket
    /// (the hub cancels whatever this turn was still synthesizing), release
    /// the mic and the audio session.
    func finish() {
        guard active else { return }
        active = false
        epoch += 1
        cancelTimers()
        closeSocket()
        removeTap()
        dropPlayback()
        listeningTurnID = nil
        submittedTurnID = nil
        teardownAudio()
        phase = .idle
        level = 0
    }

    /// The orb: Ready → talk, Listening → send now, Speaking → stop talking.
    func orbTapped() {
        switch phase {
        case .ready:
            beginListening()
        case .listening:
            endTurn(reason: "user_tap")
        case .speaking:
            // Local barge: silence the rest of this reply. The turn still
            // completes on reply_end; the mic does not reopen by itself.
            replyMuted = true
            dropPlayback()
            maybeFinishTurn()
        case .failed:
            retry()
        default:
            break
        }
    }

    func retry() {
        guard let credential else { return }
        finish()
        start(credential: credential)
    }

    // MARK: Audio session + graph

    private func prepareAudioThenConnect() async {
        let epochAtStart = epoch
        #if DEBUG
        let injecting = Self.injection != nil
        #else
        let injecting = false
        #endif
        if !injecting {
            guard await AVAudioApplication.requestRecordPermission() else {
                fail("Microphone access is off in Settings.")
                return
            }
        }
        guard active, epoch == epochAtStart else { return }
        do {
            let session = AVAudioSession.sharedInstance()
            // 0x4 = allowBluetoothHFP (named allowBluetooth before watchOS
            // 11): AirPods carry both the mic and the reply when connected,
            // otherwise the watch's own mic and speaker.
            try session.setCategory(.playAndRecord, mode: .voiceChat, policy: .default,
                                    options: AVAudioSession.CategoryOptions(rawValue: 0x4))
            // watchOS: the asynchronous activation is the one that also
            // grants networking during an audio session (TN3135).
            let activated = try await session.activate(options: [])
            guard activated else { throw WatchVoiceError.sessionRefused }
            guard active, epoch == epochAtStart else { return }
            try buildGraph(withInput: !injecting)
            observeAudioSession()
            audioReady = true
        } catch {
            watchVoiceLog.error("watch_voice_audio_setup_failed error=\(String(describing: error), privacy: .public)")
            fail("Couldn't start audio on the watch.")
            return
        }
        connect()
    }

    private func buildGraph(withInput: Bool) throws {
        let engine = AVAudioEngine()
        let player = AVAudioPlayerNode()
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: playbackFormat)
        if withInput {
            let format = engine.inputNode.outputFormat(forBus: 0)
            guard let pipe = WatchMicPipe(inputFormat: format) else { throw WatchVoiceError.inputFormat }
            self.pipe = pipe
        }
        engine.prepare()
        try engine.start()
        player.play()
        self.engine = engine
        self.player = player
    }

    private func rebuildGraph() {
        let hadTap = tapInstalled
        removeTap()
        dropPlayback()
        engine?.stop()
        engine = nil
        player = nil
        pipe = nil
        do {
            #if DEBUG
            try buildGraph(withInput: Self.injection == nil)
            #else
            try buildGraph(withInput: true)
            #endif
            if hadTap, phase == .listening { installTap() }
        } catch {
            fail("Audio route failed — tap to reconnect.")
        }
    }

    private func teardownAudio() {
        for o in observers { NotificationCenter.default.removeObserver(o) }
        observers.removeAll()
        engine?.stop()
        engine = nil
        player = nil
        pipe = nil
        audioReady = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func observeAudioSession() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification,
                                            object: nil, queue: .main) { [weak self] note in
            let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt ?? 0
            MainActor.assumeIsolated { self?.routeChanged(raw) }
        })
        observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification,
                                            object: nil, queue: .main) { [weak self] note in
            let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt ?? 0
            MainActor.assumeIsolated { self?.interrupted(raw) }
        })
    }

    private func routeChanged(_ raw: UInt) {
        guard active, let reason = AVAudioSession.RouteChangeReason(rawValue: raw),
              reason == .oldDeviceUnavailable || reason == .newDeviceAvailable else { return }
        watchVoiceLog.info("watch_voice_route_change reason=\(raw, privacy: .public) phase=\(self.phase.label, privacy: .public)")
        // AirPods in or out: submit what was said so far, then rebuild the
        // graph for the new input/output format.
        if phase == .listening { endTurn(reason: "route_change") }
        if phase == .speaking { toast = "Audio moved — reply text is on screen." }
        rebuildGraph()
        maybeFinishTurn()
    }

    private func interrupted(_ raw: UInt) {
        guard active, let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
        switch type {
        case .began:
            if phase == .listening { endTurn(reason: "interruption") }
            dropPlayback()
            engine?.pause()
            maybeFinishTurn()
        case .ended:
            try? engine?.start()
            player?.play()
        @unknown default:
            break
        }
    }

    // MARK: Socket

    private func connect() {
        guard active, let credential, let url = credential.voiceURL() else {
            fail("This watch isn't set up for direct voice yet.")
            return
        }
        epoch += 1
        let myEpoch = epoch
        closeSocket()
        preReadyError = nil
        phase = .connecting
        var request = URLRequest(url: url, timeoutInterval: WatchVoiceLimits.connect)
        if credential.queryTokenAuth != true {
            request.setValue("Bearer \(credential.voiceToken)", forHTTPHeaderField: "Authorization")
        }
        let pin = WatchVoicePinning(fingerprint: WatchVoiceCredential.normalizedFingerprint(credential.certFingerprint))
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = WatchVoiceLimits.connect
        let session = URLSession(configuration: config, delegate: pin, delegateQueue: nil)
        let task = session.webSocketTask(with: request)
        task.maximumMessageSize = 8 * 1024 * 1024
        urlSession = session
        socket = task
        task.resume()
        watchVoiceLog.info("watch_voice_connect host=\(url.host ?? "?", privacy: .public) attempt=\(self.reconnects, privacy: .public)")
        receiveLoop(task, epoch: myEpoch)
        connectTimer?.cancel()
        connectTimer = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(WatchVoiceLimits.connect))
            guard !Task.isCancelled, let self, self.epoch == myEpoch, self.phase == .connecting else { return }
            self.lostConnection(why: "The hub didn't answer within 8 s.")
        }
    }

    private nonisolated func receiveLoop(_ task: URLSessionWebSocketTask, epoch: Int) {
        task.receive { [weak self] result in
            // Copy the Sendable payload out before the hop.
            let outcome: Result<URLSessionWebSocketTask.Message, Error> = result
            Task { @MainActor [weak self] in
                guard let self, epoch == self.epoch, task === self.socket else { return }
                switch outcome {
                case .success(let message):
                    self.handle(message)
                    self.receiveLoop(task, epoch: epoch)
                case .failure(let error):
                    watchVoiceLog.info("watch_voice_socket_failed error=\(error.localizedDescription, privacy: .public)")
                    self.lostConnection(why: nil)
                }
            }
        }
    }

    private func closeSocket() {
        connectTimer?.cancel()
        connectTimer = nil
        socket?.cancel(with: .goingAway, reason: nil)
        socket = nil
        urlSession?.invalidateAndCancel()
        urlSession = nil
    }

    /// Socket dropped, refused or timed out. Never resubmits a turn the hub
    /// may already be running; reconnects a bounded number of times.
    private func lostConnection(why: String?) {
        guard active else { return }
        if phase == .connecting, let refused = preReadyError {
            // The hub answered and said no; retrying in 1.5 s won't change it.
            fail(refused)
            return
        }
        epoch += 1
        closeSocket()
        removeTap()
        dropPlayback()
        cancelTurnTimers()
        if listeningTurnID != nil {
            listeningTurnID = nil
            toast = "Connection dropped while listening — say it again."
        }
        if submittedTurnID != nil {
            submittedTurnID = nil
            toast = "Connection dropped — your last message may still reach the hub. Not resending it."
        }
        level = 0
        if reconnects < WatchVoiceLimits.reconnectAttempts {
            reconnects += 1
            phase = .connecting
            autoListenOnReady = false
            let myEpoch = epoch
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: .seconds(1.5))
                guard let self, self.active, self.epoch == myEpoch else { return }
                self.connect()
            }
        } else {
            directUnavailable = true
            phase = .failed(why ?? "Can't reach your hub on Wi-Fi.")
            toast = why ?? "Can't reach your hub on Wi-Fi."
        }
    }

    private func fail(_ message: String) {
        epoch += 1
        closeSocket()
        cancelTimers()
        removeTap()
        directUnavailable = true
        toast = message
        phase = .failed(message)
    }

    // MARK: Server frames

    private struct ServerFrame: Decodable {
        let type: String
        let text: String?
        let message: String?
        let outcome: String?
        let reason: String?
        let turn_id: String?
    }

    private func handle(_ message: URLSessionWebSocketTask.Message) {
        switch message {
        case .data(let data):
            playChunk(data)
        case .string(let text):
            guard let data = text.data(using: .utf8),
                  let frame = try? JSONDecoder().decode(ServerFrame.self, from: data) else { return }
            handle(frame)
        @unknown default:
            break
        }
    }

    private func handle(_ frame: ServerFrame) {
        switch frame.type {
        case "ready":
            connectTimer?.cancel()
            reconnects = 0
            directUnavailable = false
            phase = .ready
            sendHello()
            watchVoiceLog.info("watch_voice_state ready")
            if autoListenOnReady {
                autoListenOnReady = false
                beginListening()
            }
        case "transcript_partial":
            if phase == .listening || phase == .transcribing, let t = frame.text, !t.isEmpty {
                transcript = t
            }
        case "transcript":
            guard submittedTurnID != nil else { return }
            transcript = frame.text ?? ""
            progressed()
            if phase == .transcribing { setPhase(.thinking) }
        case "reply_start":
            guard submittedTurnID != nil else { return }
            progressed()
            if phase == .transcribing { setPhase(.thinking) }
        case "audio_segment", "reply_segment":
            guard submittedTurnID != nil else { return }
            progressed()
            if !replyAuthoritative, let t = frame.text, !t.isEmpty {
                reply = reply.isEmpty ? t : reply + " " + t
            }
        case "reply_text":
            guard submittedTurnID != nil else { return }
            progressed()
            reply = frame.text ?? reply
            replyAuthoritative = true
        case "reply_end":
            guard submittedTurnID != nil else { return }
            progressed()
            gate.endReceived()
            watchVoiceLog.info("watch_voice_reply_end pending=\(self.gate.pendingBuffers, privacy: .public)")
            maybeFinishTurn()
        case "turn_outcome":
            guard submittedTurnID != nil || phase == .listening else { return }
            let outcome = (frame.outcome ?? "").lowercased()
            let note: String
            switch outcome {
            case "empty_stt", "capture_rejected_short": note = "Didn't catch that — tap to talk."
            case "disconnected", "capture_disconnected": note = "Connection hiccup — tap to talk."
            default: note = frame.reason ?? frame.message ?? "That turn stopped — tap to talk."
            }
            endWithoutReply(note)
        case "idle":
            // Legacy reset (empty capture, rejected speaker). After reply_end
            // the playback gate owns the turn, so ignore it there.
            guard !gate.replyEnded, submittedTurnID != nil || phase == .listening else { return }
            endWithoutReply("Didn't catch that — tap to talk.")
        case "error":
            let msg = frame.message ?? "Voice stopped — try again."
            if submittedTurnID != nil || phase == .listening {
                endWithoutReply(msg)
            } else {
                if phase == .connecting { preReadyError = msg }
                toast = msg
            }
        default:
            break // voice_print, navigate, clipboard, wake_*: not used on the watch
        }
    }

    // MARK: Turn

    private func beginListening() {
        guard active, phase == .ready, socket != nil, listeningTurnID == nil, submittedTurnID == nil else { return }
        turnSeq &+= 1
        let id = "watch-\(turnSeq)"
        listeningTurnID = id
        transcript = ""
        reply = ""
        toast = nil
        replyAuthoritative = false
        replyMuted = false
        gate.reset()
        heardSpeech = false
        spokenFor = 0
        silentFor = 0
        captureElapsed = 0
        wireSamplesSent = 0
        sendJSON(["type": "start", "sample_rate": 16_000, "turn_id": id, "trigger": "user_tap"])
        setPhase(.listening)
        #if DEBUG
        if let injection = Self.injection {
            if injectedTurnsLeft == nil { injectedTurnsLeft = injection.turns }
            let clip = injection.clips[(injection.turns - (injectedTurnsLeft ?? 1)) % injection.clips.count]
            injectedTurnsLeft = max(0, (injectedTurnsLeft ?? 1) - 1)
            startInjected(clip, turnID: id)
        } else {
            installTap()
        }
        #else
        installTap()
        #endif
        captureTimer?.cancel()
        captureTimer = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(WatchVoiceLimits.recording))
            guard !Task.isCancelled, let self, self.listeningTurnID == id else { return }
            self.endTurn(reason: "max_turn")
        }
    }

    private func installTap() {
        guard !tapInstalled, let engine, let pipe else { return }
        let input = engine.inputNode
        input.installTap(onBus: 0, bufferSize: 1_600, format: input.outputFormat(forBus: 0),
                         block: Self.makeTapBlock(pipe: pipe, client: self))
        tapInstalled = true
    }

    private func removeTap() {
        guard tapInstalled else { return }
        engine?.inputNode.removeTap(onBus: 0)
        tapInstalled = false
        #if DEBUG
        injectTask?.cancel()
        injectTask = nil
        #endif
    }

    /// Built outside the main actor: the tap fires on a real-time audio
    /// thread, and a closure formed in a @MainActor method would inherit
    /// main-actor isolation and trip the Swift 6 executor check (4bbe4bee).
    private nonisolated static func makeTapBlock(pipe: WatchMicPipe,
                                                 client: WatchVoiceClient) -> AVAudioNodeTapBlock {
        { [weak client] buffer, _ in
            guard let (data, rms) = pipe.convert(buffer) else { return }
            Task { @MainActor [weak client] in client?.micFrame(data, rms: rms) }
        }
    }

    fileprivate func micFrame(_ data: Data, rms: Float) {
        guard phase == .listening, listeningTurnID != nil, !data.isEmpty else { return }
        let samples = data.count / MemoryLayout<Float>.size
        wireSamplesSent += samples
        socket?.send(.data(data)) { _ in }
        let dt = Double(samples) / 16_000
        captureElapsed += dt
        let db = rms > 0 ? 20 * log10(rms) : -120
        level = Double(min(1, max(0, (db + 55) / 40)))
        if db > Self.voicedDB {
            heardSpeech = true
            spokenFor += dt
            silentFor = 0
        } else {
            silentFor += dt
        }
        let endpoint = WatchVoiceLimits.endpoint
        if endpoint.shouldStop(heardSpeech: heardSpeech, spokenFor: spokenFor,
                               silentFor: silentFor, elapsed: captureElapsed) {
            endTurn(reason: captureElapsed >= endpoint.maxDuration ? "max_turn" : "silence")
        } else if !heardSpeech, captureElapsed >= Self.noSpeechTimeout {
            endTurn(reason: "silence")
        }
    }

    private func endTurn(reason: String) {
        guard phase == .listening, let id = listeningTurnID else { return }
        removeTap()
        captureTimer?.cancel()
        level = 0
        listeningTurnID = nil
        submittedTurnID = id
        gate.reset()
        let stop: [String: Any] = [
            "type": "stop", "turn_id": id, "trigger": "user_tap", "end_reason": reason,
            "capture_duration_ms": Int(captureElapsed * 1_000),
            "wire_samples_sent": wireSamplesSent,
        ]
        let myEpoch = epoch
        sendJSON(stop) { [weak self] failed in
            guard failed, let self, self.epoch == myEpoch else { return }
            self.lostConnection(why: nil)
        }
        stopSentAt = Date()
        setPhase(.transcribing)
        progressed()
        armProgressWatchdog(turnID: id)
    }

    private func armProgressWatchdog(turnID: String) {
        progressTimer?.cancel()
        progressTimer = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard let self, self.submittedTurnID == turnID else { return }
                let stalled = Date().timeIntervalSince(self.lastProgress) > WatchVoiceLimits.noProgress
                let overlong = Date().timeIntervalSince(self.stopSentAt) > WatchVoiceLimits.turn
                if stalled || overlong {
                    watchVoiceLog.error("watch_voice_turn_timeout stalled=\(stalled, privacy: .public) overlong=\(overlong, privacy: .public)")
                    // A fresh socket cancels the hub's handler for this turn,
                    // and stale frames from it are dropped by the epoch check.
                    self.submittedTurnID = nil
                    self.dropPlayback()
                    self.reconnects = 0
                    self.lostConnection(why: nil)
                    self.toast = stalled ? "The hub stopped responding — tap to try again."
                                         : "That reply ran too long — tap to try again."
                    return
                }
            }
        }
    }

    private func progressed() { lastProgress = Date() }

    private func maybeFinishTurn() {
        guard submittedTurnID != nil, gate.complete else { return }
        let resume = !replyMuted
        submittedTurnID = nil
        cancelTurnTimers()
        setPhase(.ready)
        guard active, resume else { return }
        #if DEBUG
        if Self.injection != nil, (injectedTurnsLeft ?? 0) == 0 { return }
        #endif
        // Back-and-forth: the reply finished playing, listen for the answer.
        beginListening()
    }

    private func endWithoutReply(_ note: String) {
        removeTap()
        listeningTurnID = nil
        submittedTurnID = nil
        cancelTurnTimers()
        dropPlayback()
        level = 0
        toast = note
        setPhase(.ready)
    }

    private func setPhase(_ next: WatchVoicePhase) {
        guard phase != next else { return }
        watchVoiceLog.info("watch_voice_state \(self.phase.label, privacy: .public) -> \(next.label, privacy: .public)")
        NSLog("permagent-watchvoice: state %@ -> %@", phase.label, next.label)
        phase = next
    }

    private func cancelTurnTimers() {
        captureTimer?.cancel()
        captureTimer = nil
        progressTimer?.cancel()
        progressTimer = nil
    }

    private func cancelTimers() {
        cancelTurnTimers()
        connectTimer?.cancel()
        connectTimer = nil
    }

    // MARK: Playback

    private func playChunk(_ data: Data) {
        guard submittedTurnID != nil, !replyMuted else { return }
        let n = data.count / MemoryLayout<Float>.size
        guard n > 0, let player,
              let buffer = AVAudioPCMBuffer(pcmFormat: playbackFormat, frameCapacity: AVAudioFrameCount(n)),
              let channel = buffer.floatChannelData?[0] else { return }
        buffer.frameLength = AVAudioFrameCount(n)
        data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            memcpy(channel, base, n * MemoryLayout<Float>.size)
        }
        progressed()
        gate.scheduled()
        if phase != .speaking { setPhase(.speaking) }
        player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack,
                              completionHandler: Self.makeRenderedHandler(client: self,
                                                                          generation: playbackGeneration))
        if !player.isPlaying { player.play() }
    }

    private nonisolated static func makeRenderedHandler(client: WatchVoiceClient,
                                                        generation: Int)
        -> @Sendable (AVAudioPlayerNodeCompletionCallbackType) -> Void {
        { [weak client] _ in
            Task { @MainActor [weak client] in client?.bufferRendered(generation: generation) }
        }
    }

    private func bufferRendered(generation: Int) {
        guard generation == playbackGeneration else { return }
        gate.rendered()
        progressed()
        maybeFinishTurn()
    }

    private func dropPlayback() {
        playbackGeneration &+= 1
        gate.dropAudio()
        player?.stop()
        player?.play()
    }

    // MARK: Send

    private func sendHello() {
        let info = Bundle.main.infoDictionary ?? [:]
        sendJSON([
            "type": "hello",
            "protocol_version": 1,
            "client": "watch_voice",
            "app_version": info["CFBundleShortVersionString"] as? String ?? "",
            "build_number": info["CFBundleVersion"] as? String ?? "",
            "capabilities": ["hello_v1"],
            "wire_sample_rate": 16_000,
            "wire_channels": 1,
            "wire_encoding": "pcm_f32le",
        ])
    }

    private func sendJSON(_ object: [String: Any], completion: (@MainActor (Bool) -> Void)? = nil) {
        guard let data = try? JSONSerialization.data(withJSONObject: object),
              let text = String(data: data, encoding: .utf8), let socket else {
            completion?(true)
            return
        }
        socket.send(.string(text), completionHandler: Self.makeSendHandler(completion))
    }

    private nonisolated static func makeSendHandler(_ completion: (@MainActor (Bool) -> Void)?)
        -> @Sendable (Error?) -> Void {
        { error in
            let failed = error != nil
            Task { @MainActor in completion?(failed) }
        }
    }

    // MARK: DEBUG audio injection

    #if DEBUG
    private var injectTask: Task<Void, Never>?
    private var injectedTurnsLeft: Int?

    /// Simulator rehearsal (the watch sim has no usable mic): launch with
    /// `SIMCTL_CHILD_PERMAGENT_INJECT_WAV=/abs/a.wav[,/abs/b.wav…]` (16 kHz
    /// mono PCM16) and optionally `SIMCTL_CHILD_PERMAGENT_INJECT_TURNS=2`.
    /// Listen N streams clip N (cycling) in real time through the same
    /// VAD/stop path as the mic. Release builds contain none of this.
    private static let injection: (clips: [[Float]], turns: Int)? = {
        let env = ProcessInfo.processInfo.environment
        guard let list = env["PERMAGENT_INJECT_WAV"] else { return nil }
        let clips = list.split(separator: ",").map(String.init).compactMap { path -> [Float]? in
            guard path.hasPrefix("/"), let data = FileManager.default.contents(atPath: path) else { return nil }
            return WatchWAV.pcm16MonoSamples(data)
        }
        guard !clips.isEmpty else { return nil }
        let turns = Int(env["PERMAGENT_INJECT_TURNS"] ?? "") ?? clips.count
        NSLog("permagent-inject: watch voice will stream INJECTED audio (%d clip(s), %d turn(s))",
              clips.count, turns)
        return (clips, max(1, turns))
    }()

    private func startInjected(_ samples: [Float], turnID: String) {
        injectTask?.cancel()
        injectTask = Task { @MainActor [weak self] in
            let chunk = 1_600
            var index = 0
            while index < samples.count {
                guard !Task.isCancelled, let self, self.listeningTurnID == turnID else { return }
                let slice = Array(samples[index..<min(samples.count, index + chunk)])
                index += chunk
                let rms = sqrt(slice.reduce(0) { $0 + $1 * $1 } / Float(max(1, slice.count)))
                let data = slice.withUnsafeBufferPointer { Data(buffer: $0) }
                self.micFrame(data, rms: rms)
                try? await Task.sleep(for: .milliseconds(100))
            }
            // Clip over without the VAD firing: submit like a tap.
            guard let self, self.listeningTurnID == turnID else { return }
            self.endTurn(reason: "silence")
        }
    }
    #endif
}

enum WatchVoiceError: Error {
    case sessionRefused
    case inputFormat
}

/// Mic buffers in the hardware format → 16 kHz mono Float32 wire frames.
/// Used only from the audio tap thread.
final class WatchMicPipe: @unchecked Sendable {
    private let converter: AVAudioConverter
    private let inFormat: AVAudioFormat
    private let outFormat: AVAudioFormat

    init?(inputFormat: AVAudioFormat) {
        guard inputFormat.sampleRate > 0, inputFormat.channelCount > 0,
              let out = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000,
                                      channels: 1, interleaved: false),
              let converter = AVAudioConverter(from: inputFormat, to: out) else { return nil }
        self.converter = converter
        self.inFormat = inputFormat
        self.outFormat = out
    }

    func convert(_ buffer: AVAudioPCMBuffer) -> (Data, Float)? {
        guard buffer.frameLength > 0, buffer.format.sampleRate == inFormat.sampleRate else { return nil }
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * outFormat.sampleRate
                                         / inFormat.sampleRate) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: capacity) else { return nil }
        nonisolated(unsafe) var fed = false
        // The converter calls this block synchronously, on this thread.
        nonisolated(unsafe) let input = buffer
        var error: NSError?
        let status = converter.convert(to: out, error: &error) { _, inputStatus in
            if fed { inputStatus.pointee = .noDataNow; return nil }
            fed = true
            inputStatus.pointee = .haveData
            return input
        }
        guard status != .error, out.frameLength > 0, let ch = out.floatChannelData?[0] else { return nil }
        let n = Int(out.frameLength)
        var sum: Float = 0
        for i in 0..<n { sum += ch[i] * ch[i] }
        return (Data(bytes: ch, count: n * MemoryLayout<Float>.size), sqrt(sum / Float(n)))
    }
}

/// Pins the LAN gateway's self-signed leaf by SHA-256. With no fingerprint
/// (plain-HTTP loopback rehearsal) it defers to the system.
final class WatchVoicePinning: NSObject, URLSessionDelegate, @unchecked Sendable {
    private let fingerprint: String?

    init(fingerprint: String?) { self.fingerprint = fingerprint }

    func urlSession(_ session: URLSession, didReceive challenge: URLAuthenticationChallenge,
                    completionHandler: @escaping @Sendable (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        guard let fingerprint else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        guard let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
              let leaf = chain.first else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        let der = SecCertificateCopyData(leaf) as Data
        let digest = SHA256.hash(data: der).map { String(format: "%02x", $0) }.joined()
        if digest == fingerprint {
            completionHandler(.useCredential, URLCredential(trust: trust))
        } else {
            watchVoiceLog.error("watch_voice_pin_mismatch")
            completionHandler(.cancelAuthenticationChallenge, nil)
        }
    }
}

#if DEBUG
enum WatchWAV {
    /// Float samples from a 16 kHz mono PCM16 WAV (the rehearsal clips).
    static func pcm16MonoSamples(_ data: Data) -> [Float]? {
        let bytes = [UInt8](data)
        guard bytes.count > 44, String(bytes: bytes[0..<4], encoding: .ascii) == "RIFF" else { return nil }
        func u32(_ i: Int) -> Int { Int(bytes[i]) | Int(bytes[i+1]) << 8 | Int(bytes[i+2]) << 16 | Int(bytes[i+3]) << 24 }
        func u16(_ i: Int) -> Int { Int(bytes[i]) | Int(bytes[i+1]) << 8 }
        var i = 12
        var rate = 0, channels = 0, bits = 0
        while i + 8 <= bytes.count {
            let id = String(bytes: bytes[i..<i+4], encoding: .ascii) ?? ""
            let size = u32(i + 4)
            let body = i + 8
            if id == "fmt " {
                channels = u16(body + 2); rate = u32(body + 4); bits = u16(body + 14)
            } else if id == "data" {
                guard rate == 16_000, channels == 1, bits == 16 else { return nil }
                let end = min(bytes.count, body + size)
                var out: [Float] = []
                out.reserveCapacity((end - body) / 2)
                var j = body
                while j + 1 < end {
                    out.append(Float(Int16(bitPattern: UInt16(u16(j)))) / 32_768)
                    j += 2
                }
                return out
            }
            i = body + size + (size & 1)
        }
        return nil
    }
}
#endif
