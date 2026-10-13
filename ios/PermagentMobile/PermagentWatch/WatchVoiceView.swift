// The Chat button's destination. With a direct-voice credential the watch
// talks to the hub itself (WatchVoiceClient); without one, or after direct
// voice gives up, it is the phone-relay chat (WatchChatView) as before.

import SwiftUI

struct WatchChatEntry: View {
    @ObservedObject private var voice = WatchVoiceProvisioning.shared
    @State private var useRelay = false

    var body: some View {
        if let credential = voice.credential, credential.isUsable, !useRelay {
            WatchVoiceView(credential: credential) { useRelay = true }
        } else {
            WatchChatView()
        }
    }
}

struct WatchVoiceView: View {
    let credential: WatchVoiceCredential
    var useRelay: () -> Void

    @EnvironmentObject private var relay: WatchRelay
    @Environment(\.dismiss) private var dismiss
    @StateObject private var client = WatchVoiceClient()

    var body: some View {
        ZStack {
            ChatSurface.bg.ignoresSafeArea()
            ScrollView {
                VStack(spacing: 6) {
                    Button { client.orbTapped() } label: {
                        VoiceOrbView(level: orbLevel, speaking: client.phase == .speaking,
                                     thinking: thinking, listening: client.phase == .listening)
                            .frame(width: 84, height: 84)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(orbHint)

                    Text(client.phase.label)
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.muted)
                        .accessibilityIdentifier("watchVoiceState")

                    if !client.transcript.isEmpty {
                        Text(client.transcript)
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.muted)
                            .multilineTextAlignment(.center)
                            .lineLimit(4)
                            .accessibilityIdentifier("watchVoiceTranscript")
                    }
                    if !client.reply.isEmpty {
                        Text(client.reply)
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.text)
                            .multilineTextAlignment(.center)
                            .accessibilityIdentifier("watchVoiceReply")
                    }
                    if let toast = client.toast {
                        Text(toast)
                            .font(.brandCaption)
                            .foregroundStyle(Brand.warning)
                            .multilineTextAlignment(.center)
                            .accessibilityIdentifier("watchVoiceToast")
                    }
                    if client.directUnavailable {
                        Button("Retry") { client.retry() }
                            .font(.brandCaption)
                            .tint(ChatSurface.spark)
                        if relay.paired {
                            Button("Use iPhone") {
                                client.finish()
                                useRelay()
                            }
                            .font(.brandCaption)
                            .tint(ChatSurface.ember)
                        }
                    }

                    Button {
                        client.finish()
                        dismiss()
                    } label: {
                        Label("Finish", systemImage: "stop.fill")
                            .font(.brandHeadline)
                            .foregroundStyle(ChatSurface.onSpark)
                            .frame(maxWidth: .infinity, minHeight: 40)
                            .background(ChatSurface.spark, in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Finish conversation")
                }
                .padding(.horizontal, 6)
            }
        }
        .navigationTitle(relay.agentName)
        .onAppear { client.start(credential: credential) }
        .onDisappear { client.finish() }
    }

    private var thinking: Bool {
        client.phase == .transcribing || client.phase == .thinking || client.phase == .connecting
    }

    private var orbLevel: Double {
        switch client.phase {
        case .listening: return client.level
        case .speaking: return 0.5
        case .transcribing, .thinking: return 0.2
        default: return 0
        }
    }

    private var orbHint: String {
        switch client.phase {
        case .ready: return "Talk"
        case .listening: return "Send now"
        case .speaking: return "Stop speaking"
        case .failed: return "Reconnect"
        default: return "Orb"
        }
    }
}
