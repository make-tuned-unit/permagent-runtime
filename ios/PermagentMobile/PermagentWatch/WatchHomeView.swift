import SwiftUI

struct WatchHomeView: View {
    @EnvironmentObject private var relay: WatchRelay
    @State private var dest: Dest?

    enum Dest: Hashable { case chat, note }

    var body: some View {
        NavigationStack {
            ZStack {
                ChatSurface.bg.ignoresSafeArea()
                VStack(spacing: 10) {
                    MobiusView(size: 72, glow: 0.85)
                        .padding(.top, 4)

                    Text(relay.paired ? relay.agentName : "Permagent")
                        .font(.brandHeadline)
                        .foregroundStyle(ChatSurface.text)
                        .lineLimit(1)

                    if let notice = relay.notice, !relay.paired {
                        Text(notice)
                            .font(.brandCaption)
                            .foregroundStyle(Brand.warning)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 8)
                    }

                    HStack(spacing: 10) {
                        watchButton(title: "Chat", systemImage: "bubble.left.fill") {
                            dest = .chat
                        }
                        watchButton(title: "Note", systemImage: "mic.fill") {
                            dest = .note
                        }
                    }
                    .padding(.top, 4)
                }
                .padding(.horizontal, 8)
            }
            .navigationDestination(item: $dest) { d in
                switch d {
                case .chat: WatchChatView()
                case .note: WatchNoteView()
                }
            }
        }
    }

    private func watchButton(title: String, systemImage: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 4) {
                Image(systemName: systemImage)
                    .font(.system(size: 16, weight: .semibold))
                Text(title)
                    .font(.brandCaption)
            }
            .foregroundStyle(ChatSurface.onSpark)
            .frame(maxWidth: .infinity, minHeight: 52)
            .background(ChatSurface.ribbon, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(!relay.paired && dest == nil)
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
                    WatchOrbButton(
                        level: recorder.isRecording ? Double(recorder.level) : (relay.noteBusy ? 0.2 : 0),
                        speaking: false,
                        thinking: relay.noteBusy,
                        enabled: recorder.isRecording
                    ) { if recorder.isRecording { recorder.stop() } }

                    if recorder.isRecording {
                        Text(timeLabel)
                            .font(.brandHeadline)
                            .foregroundStyle(ChatSurface.spark)
                            .monospacedDigit()
                        Text("Listening…")
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.muted)
                    } else if relay.noteBusy {
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
        .navigationTitle("Note")
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

/// The conversation orb as the only control: tap to stop a live turn.
private struct WatchOrbButton: View {
    var level: Double
    var speaking: Bool
    var thinking: Bool
    var enabled: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VoiceOrbView(level: level, speaking: speaking, thinking: thinking)
                .frame(width: 96, height: 96)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(enabled ? "Stop listening" : "Orb")
    }
}
