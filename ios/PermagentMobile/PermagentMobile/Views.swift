// v1 surfaces — supervision from anywhere. Each view is a thin renderer over
// the hub's existing API; no device-local state beyond the pairing token.

import SwiftUI

// ── Pairing ──────────────────────────────────────────────────────────────────

struct PairingView: View {
    @EnvironmentObject var session: HubSession
    @State private var url = ""
    @State private var failed = false
    var body: some View {
        VStack(spacing: 24) {
            Spacer()
            Text("PERMAGENT")
                .font(.system(.title2, design: .monospaced).weight(.bold))
                .foregroundStyle(Brand.ribbon)
            Text("Pair with your hub")
                .font(.title3.weight(.semibold))
                .foregroundStyle(Brand.text)
            Text("On your Mac: Settings → Devices → copy the pairing URL, then paste it here. Both devices must be on your tailnet.")
                .font(.footnote)
                .foregroundStyle(Brand.textMuted)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)
            GlassCard {
                TextField("http://your-mac.tailnet.ts.net:3001/ui/#token=…", text: $url)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(Brand.text)
            }
            .padding(.horizontal, 24)
            if failed {
                Text("That doesn't look like a pairing URL — copy it fresh from Settings → Devices.")
                    .font(.caption)
                    .foregroundStyle(Brand.danger)
                    .padding(.horizontal, 32)
            }
            Button {
                Task { failed = !(await session.pair(from: url)) }
            } label: {
                Text("Connect")
                    .font(.body.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 14)
                    .background(Brand.ribbon)
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .foregroundStyle(Brand.deepVoid)
            }
            .padding(.horizontal, 24)
            Spacer()
            Spacer()
        }
    }
}

// ── Decisions (the phone's killer surface) ───────────────────────────────────

struct DecisionItem: Decodable, Identifiable {
    let id: String
    let kind: String
    let title: String?
    let detail: String?
}

struct InboxView: View {
    @State private var items: [DecisionItem] = []
    var body: some View {
        NavigationStack {
            List(items) { d in
                GlassCard {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(d.kind.replacingOccurrences(of: "_", with: " ").uppercased())
                            .font(.system(.caption2, design: .monospaced).weight(.semibold))
                            .foregroundStyle(Brand.cyan)
                        Text(d.title ?? "Decision")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(Brand.text)
                        if let detail = d.detail, !detail.isEmpty {
                            Text(detail).font(.caption).foregroundStyle(Brand.textMuted).lineLimit(3)
                        }
                    }
                }
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Brand.shell)
            .navigationTitle("Decisions")
            .refreshable { await load() }
            .task { await load() }
        }
    }
    func load() async {
        struct Resp: Decodable { let decisions: [DecisionItem] }
        if let resp = try? await APIClient.shared.get("/api/decisions", as: Resp.self) {
            items = resp.decisions
        }
    }
}

// ── Goals ────────────────────────────────────────────────────────────────────

struct ActiveGoal: Decodable, Identifiable {
    let id: String
    let title: String
    let state: String
}

struct GoalsView: View {
    @State private var goals: [ActiveGoal] = []
    var body: some View {
        NavigationStack {
            List(goals) { g in
                HStack(spacing: 10) {
                    Circle()
                        .fill(g.state == "in_progress" ? Brand.cyan : Brand.textDim)
                        .frame(width: 7, height: 7)
                    Text(g.title).font(.subheadline).foregroundStyle(Brand.text)
                    Spacer()
                    Text(g.state).font(.system(.caption2, design: .monospaced)).foregroundStyle(Brand.textMuted)
                }
                .listRowBackground(Color.clear)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Brand.shell)
            .navigationTitle("In Flight")
            .refreshable { await load() }
            .task { await load() }
        }
    }
    func load() async {
        struct Resp: Decodable { let goals: [ActiveGoal] }
        if let resp = try? await APIClient.shared.get("/api/goals/active", as: Resp.self) {
            goals = resp.goals
        }
    }
}

// ── Chat (real: send to the hub's /reply, stream Henry's answer) ─────────────

struct ChatBubble: Identifiable {
    let id = UUID()
    let role: String   // "user" | "assistant"
    var text: String
}

struct ChatView: View {
    @State private var draft = ""
    @State private var messages: [ChatBubble] = []
    @State private var sending = false
    @State private var sessionId = MobileSession.chatSessionId()

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 10) {
                            if messages.isEmpty {
                                Text("Ask Henry to do something on your hub — open a site in the desktop browser, dispatch a goal, check the Brain. It runs on your Mac; you watch it here.")
                                    .font(.caption)
                                    .foregroundStyle(Brand.textMuted)
                                    .padding(.top, 48)
                                    .padding(.horizontal, 4)
                            }
                            ForEach(messages) { bubble($0) }
                        }
                        .padding()
                    }
                    .onChange(of: messages.count) { _, _ in
                        if let last = messages.last {
                            withAnimation { proxy.scrollTo(last.id, anchor: .bottom) }
                        }
                    }
                }
                composer
            }
            .background(Brand.shell)
            .navigationTitle("Henry")
        }
    }

    private func bubble(_ m: ChatBubble) -> some View {
        HStack {
            if m.role == "user" { Spacer(minLength: 44) }
            Text(m.text.isEmpty ? "…" : m.text)
                .font(.subheadline)
                .foregroundStyle(m.role == "user" ? Brand.deepVoid : Brand.text)
                .padding(12)
                .background(m.role == "user" ? Brand.cyan : Brand.surface)
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            if m.role != "user" { Spacer(minLength: 44) }
        }
        .id(m.id)
    }

    private var composer: some View {
        HStack(spacing: 8) {
            TextField("Ask Henry…", text: $draft, axis: .vertical)
                .lineLimit(1...4)
                .padding(12)
                .background(Brand.surface)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .foregroundStyle(Brand.text)
            Button {
                send()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.title2)
                    .foregroundStyle(canSend ? Brand.cyan : Brand.textDim)
            }
            .disabled(!canSend)
        }
        .padding()
    }

    private var canSend: Bool {
        !sending && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !sending else { return }
        draft = ""
        sending = true
        messages.append(ChatBubble(role: "user", text: text))
        messages.append(ChatBubble(role: "assistant", text: ""))
        let idx = messages.count - 1
        Task {
            do {
                for try await delta in APIClient.shared.replyStream(text, sessionId: sessionId) {
                    if idx < messages.count { messages[idx].text += delta }
                }
                if idx < messages.count && messages[idx].text.isEmpty {
                    messages[idx].text = "Done — check your desktop."
                }
            } catch {
                if idx < messages.count {
                    messages[idx].text = "⚠️ Couldn't reach Henry — is your Mac awake and on the tailnet?"
                }
            }
            sending = false
        }
    }
}
