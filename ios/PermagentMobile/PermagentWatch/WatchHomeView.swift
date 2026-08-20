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

struct WatchChatView: View {
    @EnvironmentObject private var relay: WatchRelay
    @State private var draft = ""
    @FocusState private var speaking: Bool

    var body: some View {
        ZStack {
            ChatSurface.bg.ignoresSafeArea()
            VStack(spacing: 8) {
                MobiusView(size: 88, glow: relay.chatBusy ? 1 : 0.6)
                if relay.chatThinking {
                    ThinkingDots()
                }
                if !relay.chatText.isEmpty {
                    Text(relay.chatText)
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.text)
                        .multilineTextAlignment(.center)
                        .lineLimit(6)
                } else if let notice = relay.notice {
                    Text(notice)
                        .font(.brandCaption)
                        .foregroundStyle(Brand.warning)
                        .multilineTextAlignment(.center)
                } else {
                    Text("Speak, then send. The orb stays with you.")
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.muted)
                        .multilineTextAlignment(.center)
                }
                TextField("Speak", text: $draft)
                    .focused($speaking)
                    .font(.brandCaption)
                    .onSubmit(send)
                Button(relay.chatBusy ? "Listening…" : "Send") { send() }
                    .disabled(relay.chatBusy || draft.trimmingCharacters(in: .whitespaces).isEmpty)
                    .tint(ChatSurface.spark)
            }
            .padding(.horizontal, 6)
        }
        .navigationTitle(relay.agentName)
        .onAppear { speaking = true }
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        draft = ""
        speaking = false
        relay.chat(text)
    }
}

struct WatchNoteView: View {
    @EnvironmentObject private var relay: WatchRelay
    @StateObject private var recorder = WatchRecorder()
    @State private var projectSpoken = ""
    @State private var denied = false

    var body: some View {
        ZStack {
            ChatSurface.bg.ignoresSafeArea()
            ScrollView {
                VStack(spacing: 8) {
                    if recorder.isRecording {
                        Text(timeLabel)
                            .font(.brandHeadline)
                            .foregroundStyle(ChatSurface.spark)
                            .monospacedDigit()
                        Button("Stop") { recorder.stop() }
                            .tint(Brand.danger)
                    } else if relay.noteBusy {
                        ProgressView().tint(ChatSurface.spark)
                        Text("Transcribing on your Mac…")
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
                            Text("Project: \(project.name)")
                                .font(.brandCaption)
                                .foregroundStyle(ChatSurface.spark)
                            Button(relay.noteBusy ? "Saving…" : "Save") { relay.saveNote() }
                                .disabled(relay.noteBusy)
                                .tint(ChatSurface.spark)
                        } else {
                            Text("Say the project name")
                                .font(.brandCaption)
                                .foregroundStyle(ChatSurface.muted)
                            TextField("Project", text: $projectSpoken)
                                .font(.brandCaption)
                                .onSubmit {
                                    relay.resolveProject(projectSpoken)
                                    projectSpoken = ""
                                }
                            if !relay.ambiguousProjects.isEmpty {
                                ForEach(relay.ambiguousProjects, id: \.id) { p in
                                    Button(p.name) { relay.resolvedProject = p }
                                        .font(.brandCaption)
                                }
                            }
                        }
                    } else {
                        Text("Dictate a note. Whisper on your Mac transcribes it; then say the project.")
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.muted)
                            .multilineTextAlignment(.center)
                        if denied {
                            Text("Microphone access is off in Settings.")
                                .font(.brandCaption)
                                .foregroundStyle(Brand.danger)
                        }
                        Button("Record") { start() }
                            .tint(ChatSurface.spark)
                    }
                    if let notice = relay.notice, relay.noteSaved == nil {
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
            recorder.onFinish = { url in
                if let url { relay.sendRecording(url) }
            }
        }
    }

    private var timeLabel: String {
        let s = Int(recorder.elapsed)
        return String(format: "%d:%02d", s / 60, s % 60)
    }

    private func start() {
        Task {
            guard await recorder.requestPermission() else { denied = true; return }
            denied = false
            try? recorder.start()
        }
    }
}
