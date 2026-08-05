// Past conversations — pick up a thread you already started. The hub owns
// every session; this lists what it has and hands one back to ChatView.
//
// Only real conversations are offered: scheduled-job and worker sessions run
// on the hub by the hundreds and are not threads the user was ever part of, so
// listing them would bury the two or three that matter.

import SwiftUI

struct SessionSummary: Decodable, Identifiable {
    let id: String
    let name: String
    let sessionType: String?
    let updatedAt: String?
    let messageCount: Int?

    enum CodingKeys: String, CodingKey {
        case id, name
        case sessionType = "session_type"
        case updatedAt = "updated_at"
        case messageCount = "message_count"
    }
}

struct ChatHistorySheet: View {
    let currentSessionId: String?
    let onPick: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var sessions: [SessionSummary] = []
    @State private var loaded = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Group {
                if let error {
                    ScrollView { HubErrorCard(text: error) { await load() }.padding() }
                } else if loaded && sessions.isEmpty {
                    VStack(spacing: 8) {
                        Text("No earlier conversations")
                            .font(.brandHeadline)
                            .foregroundStyle(Brand.text)
                        Text("Threads you start here will be listed for you to return to.")
                            .font(.brandCaption)
                            .foregroundStyle(Brand.textMuted)
                            .multilineTextAlignment(.center)
                    }
                    .padding(32)
                } else {
                    ScrollView {
                        VStack(spacing: 10) {
                            ForEach(sessions) { s in
                                Button { onPick(s.id) } label: { row(s) }
                                    .buttonStyle(.plain)
                            }
                        }
                        .padding()
                    }
                }
            }
            .background(Brand.shell)
            .navigationTitle("Past conversations")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") { dismiss() }
                }
            }
            .task { await load() }
        }
    }

    private func row(_ s: SessionSummary) -> some View {
        GlassCard {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(s.name.isEmpty ? "Untitled conversation" : s.name)
                        .font(.brandCaption)
                        .foregroundStyle(Brand.text)
                        .lineLimit(1)
                    HStack(spacing: 6) {
                        if let when = RelativeTime.string(from: s.updatedAt) {
                            Text(when)
                                .font(.system(.caption2, design: .monospaced))
                                .foregroundStyle(Brand.textDim)
                        }
                        if let n = s.messageCount, n > 0 {
                            Text("\(n) message\(n == 1 ? "" : "s")")
                                .font(.caption2)
                                .foregroundStyle(Brand.textDim)
                        }
                    }
                }
                Spacer(minLength: 0)
                if s.id == currentSessionId {
                    Text("CURRENT")
                        .font(.brandLabel)
                        .foregroundStyle(Brand.cyan)
                } else {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .foregroundStyle(Brand.textDim)
                }
            }
        }
    }

    private func load() async {
        struct Response: Decodable { let sessions: [SessionSummary]? }
        do {
            let r = try await APIClient.shared.get("/api/sessions", as: Response.self)
            let all = r.sessions ?? []
            withAnimation(Motion.spring) {
                // User-facing threads only, most recent first, and never an
                // empty shell (a session minted but never spoken to).
                sessions = all
                    .filter { ($0.sessionType ?? "user") == "user" }
                    .filter { ($0.messageCount ?? 0) > 0 }
                    .sorted { ($0.updatedAt ?? "") > ($1.updatedAt ?? "") }
                    .prefix(30)
                    .map { $0 }
                loaded = true
                error = nil
            }
        } catch {
            self.error = "Couldn't load your conversations."
        }
    }
}
