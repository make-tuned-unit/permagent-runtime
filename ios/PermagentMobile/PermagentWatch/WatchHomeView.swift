import SwiftUI

struct WatchHomeView: View {
    @EnvironmentObject private var relay: WatchRelay
    @ObservedObject private var route = AppRoute.shared
    @State private var dest: Dest?

    enum Dest: Hashable { case chat, note }

    var body: some View {
        NavigationStack {
            ZStack {
                ChatSurface.bg.ignoresSafeArea()
                VStack(spacing: 8) {
                    if let notice = relay.notice, !relay.paired {
                        Text(notice)
                            .font(.brandCaption)
                            .foregroundStyle(Brand.warning)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 8)
                    }

                    watchButton(
                        title: "Chat",
                        subtitle: relay.agentName,
                        systemImage: "bubble.left.and.bubble.right.fill",
                        accent: ChatSurface.spark
                    ) {
                        dest = .chat
                    }
                    watchButton(
                        title: "Dictate",
                        subtitle: "Voice note",
                        systemImage: "mic.fill",
                        accent: ChatSurface.ember
                    ) {
                        dest = .note
                    }
                }
                .padding(.horizontal, 4)
            }
            .navigationDestination(item: $dest) { d in
                switch d {
                case .chat: WatchChatView()
                case .note: WatchNoteView()
                }
            }
            .onAppear { openChatIfAsked() }
            .onChange(of: route.showWatchChat) { _, on in
                if on { openChatIfAsked() }
            }
        }
    }

    private func openChatIfAsked() {
        guard route.showWatchChat else { return }
        dest = .chat
        route.showWatchChat = false
    }

    private func watchButton(
        title: String,
        subtitle: String,
        systemImage: String,
        accent: Color,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                ZStack {
                    Circle()
                        .fill(accent.opacity(0.16))
                    Image(systemName: systemImage)
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(accent)
                }
                .frame(width: 42, height: 42)

                VStack(alignment: .leading, spacing: 1) {
                    Text(title)
                        .font(.brandHeadline)
                        .foregroundStyle(ChatSurface.text)
                        .lineLimit(1)
                        .minimumScaleFactor(0.85)
                    Text(subtitle)
                        .font(.system(size: 11, weight: .regular))
                        .foregroundStyle(ChatSurface.muted)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .frame(maxWidth: .infinity, minHeight: 66)
            .background(ChatSurface.raised, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .strokeBorder(accent.opacity(0.22), lineWidth: 1)
            }
        }
        .buttonStyle(WatchActionButtonStyle())
        .disabled(!relay.paired)
        .opacity(relay.paired ? 1 : 0.45)
    }
}

private struct WatchActionButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .opacity(configuration.isPressed ? 0.78 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

// MARK: - Chat: tap the bubble, orb is already listening.

struct WatchChatView: View {
    @EnvironmentObject private var relay: WatchRelay
    @StateObject private var recorder = WatchRecorder()
    @State private var denied = false
    @State private var listenError: String?
    @State private var active = false

    var body: some View {
        ZStack {
            ChatSurface.bg.ignoresSafeArea()
            VStack(spacing: 6) {
                WatchOrbButton(
                    level: orbLevel,
                    speaking: false,
                    thinking: relay.chatThinking || (relay.chatBusy && !recorder.isRecording),
                    listening: recorder.isRecording,
                    enabled: recorder.isRecording
                ) { if recorder.isRecording { recorder.stop() } }

                Text(statusLine)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                    .multilineTextAlignment(.center)

                if !relay.chatText.isEmpty {
                    Text(relay.chatText)
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.text)
                        .multilineTextAlignment(.center)
                        .lineLimit(6)
                } else if denied {
                    Text("Microphone access is off in Settings.")
                        .font(.brandCaption)
                        .foregroundStyle(Brand.danger)
                        .multilineTextAlignment(.center)
                } else if let listenError {
                    Text(listenError)
                        .font(.brandCaption)
                        .foregroundStyle(Brand.warning)
                        .multilineTextAlignment(.center)
                } else if let notice = relay.notice {
                    Text(notice)
                        .font(.brandCaption)
                        .foregroundStyle(Brand.warning)
                        .multilineTextAlignment(.center)
                }
            }
            .padding(.horizontal, 6)
        }
        .navigationTitle(relay.agentName)
        .onAppear {
            active = true
            armRecorder()
            beginListening()
        }
        .onDisappear {
            active = false
            recorder.cancel()
        }
        .onChange(of: relay.chatBusy) { _, busy in
            if !busy && active { beginListening() }
        }
    }

    private var orbLevel: Double {
        if recorder.isRecording { return Double(recorder.level) }
        if relay.chatThinking { return 0.2 }
        return 0
    }

    private var statusLine: String {
        if recorder.isRecording { return "Listening…" }
        if relay.chatThinking || relay.chatBusy { return "Thinking…" }
        return "Tap the orb to send"
    }

    private func armRecorder() {
        recorder.endpoint = .chat
        recorder.onFinish = { [weak recorder] url in
            guard let recorder, recorder.heardSpeech, let url else {
                if active { beginListening() }
                return
            }
            relay.sendRecording(url, kind: "chat")
        }
    }

    private func beginListening() {
        guard !recorder.isRecording, !relay.chatBusy else { return }
        Task {
            guard await recorder.requestPermission() else { denied = true; return }
            guard !recorder.isRecording, !relay.chatBusy else { return }
            denied = false
            listenError = nil
            do {
                try recorder.start()
            } catch {
                listenError = "Couldn't start the microphone."
            }
        }
    }
}

// MARK: - Note: listening first, project after, tap a name to save.

struct WatchNoteView: View {
    @EnvironmentObject private var relay: WatchRelay
    @StateObject private var recorder = WatchRecorder()
    @State private var denied = false
    @State private var listenError: String?
    @State private var active = false

    var body: some View {
        ZStack {
            ChatSurface.bg.ignoresSafeArea()
            ScrollView {
                VStack(spacing: 8) {
                    if recorder.isRecording {
                        DictationWaveform(level: Double(recorder.level))
                            .frame(height: 46)

                        Text(timeLabel)
                            .font(.system(size: 24, weight: .semibold, design: .rounded))
                            .foregroundStyle(ChatSurface.text)
                            .monospacedDigit()

                        Button {
                            recorder.stop()
                        } label: {
                            Label("Finish", systemImage: "stop.fill")
                                .font(.brandHeadline)
                                .foregroundStyle(ChatSurface.onSpark)
                                .frame(maxWidth: .infinity, minHeight: 44)
                                .background(ChatSurface.spark, in: Capsule())
                        }
                        .buttonStyle(WatchActionButtonStyle())
                        .accessibilityLabel("Finish dictating")
                    } else if relay.noteBusy {
                        ProgressView()
                            .tint(ChatSurface.spark)
                        Text(relay.noteTranscript.isEmpty ? "Transcribing…" : "Saving…")
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.muted)
                    } else if let saved = relay.noteSaved {
                        Text(saved)
                            .font(.brandCaption)
                            .foregroundStyle(Brand.success)
                            .multilineTextAlignment(.center)
                    } else if !relay.noteTranscript.isEmpty {
                        Text(relay.noteTranscript)
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.text)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        if let project = relay.resolvedProject {
                            Text("Saving to \(project.name)…")
                                .font(.brandCaption)
                                .foregroundStyle(ChatSurface.spark)
                        } else {
                            Text("Choose a project")
                                .font(.brandCaption)
                                .foregroundStyle(ChatSurface.muted)
                            ForEach(relay.projects, id: \.id) { p in
                                Button(p.name) { relay.saveNote(to: p) }
                                    .font(.brandCaption)
                                    .tint(ChatSurface.spark)
                            }
                            if relay.projects.isEmpty, let notice = relay.notice {
                                Text(notice)
                                    .font(.brandCaption)
                                    .foregroundStyle(Brand.warning)
                            }
                        }
                    } else if denied {
                        Text("Microphone access is off in Settings.")
                            .font(.brandCaption)
                            .foregroundStyle(Brand.danger)
                    } else if let listenError {
                        Text(listenError)
                            .font(.brandCaption)
                            .foregroundStyle(Brand.warning)
                    } else if let notice = relay.notice {
                        Text(notice)
                            .font(.brandCaption)
                            .foregroundStyle(Brand.warning)
                            .multilineTextAlignment(.center)
                    }
                }
                .padding(.horizontal, 4)
            }
        }
        .navigationTitle("Dictate")
        .onAppear {
            active = true
            armRecorder()
            beginListening()
        }
        .onDisappear {
            active = false
            recorder.cancel()
        }
        .onChange(of: relay.noteSaved) { _, saved in
            if saved != nil, active {
                Task {
                    try? await Task.sleep(for: .seconds(1.4))
                    guard active else { return }
                    relay.prepareNextNote()
                    beginListening()
                }
            }
        }
    }

    private var timeLabel: String {
        let s = Int(recorder.elapsed)
        return String(format: "%d:%02d", s / 60, s % 60)
    }

    private func armRecorder() {
        recorder.endpoint = .note
        recorder.onFinish = { [weak recorder] url in
            guard let recorder, recorder.heardSpeech, let url else {
                if active { beginListening() }
                return
            }
            relay.sendRecording(url, kind: "note")
        }
    }

    private func beginListening() {
        guard !recorder.isRecording, !relay.noteBusy else { return }
        Task {
            guard await recorder.requestPermission() else { denied = true; return }
            guard !recorder.isRecording, !relay.noteBusy else { return }
            denied = false
            listenError = nil
            do {
                try recorder.start()
            } catch {
                listenError = "Couldn't start the microphone."
            }
        }
    }
}

/// A recorder meter, deliberately not an Orb: Notes is one-way dictation.
private struct DictationWaveform: View {
    var level: Double

    private let weights: [Double] = [0.42, 0.72, 1, 0.78, 0.5]

    var body: some View {
        HStack(alignment: .center, spacing: 6) {
            ForEach(Array(weights.enumerated()), id: \.offset) { index, weight in
                Capsule()
                    .fill(index == 2 ? ChatSurface.spark : ChatSurface.spark.opacity(0.62))
                    .frame(width: 5, height: barHeight(weight))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(.easeOut(duration: 0.12), value: level)
        .accessibilityHidden(true)
    }

    private func barHeight(_ weight: Double) -> CGFloat {
        let live = max(0.12, min(1, level * 3.4))
        return 8 + (34 * live * weight)
    }
}

/// The conversation orb as the only control: tap to stop a live turn.
private struct WatchOrbButton: View {
    var level: Double
    var speaking: Bool
    var thinking: Bool
    var listening: Bool = false
    var enabled: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VoiceOrbView(level: level, speaking: speaking, thinking: thinking, listening: listening)
                .frame(width: 96, height: 96)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(enabled ? "Stop listening" : "Orb")
    }
}
