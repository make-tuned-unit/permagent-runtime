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

// ── Chat (v1: send + poll the session reply) ─────────────────────────────────

struct ChatView: View {
    @State private var draft = ""
    @State private var lines: [String] = []
    var body: some View {
        NavigationStack {
            VStack {
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                            GlassCard { Text(line).font(.subheadline).foregroundStyle(Brand.text) }
                        }
                    }
                    .padding()
                }
                HStack(spacing: 8) {
                    TextField("Ask Henry…", text: $draft)
                        .padding(12)
                        .background(Brand.surface)
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                        .foregroundStyle(Brand.text)
                    Button {
                        // v1 scaffold: wire to the hub's reply/session API on
                        // the mini (SSE streaming needs device testing).
                        if !draft.isEmpty { lines.append(draft); draft = "" }
                    } label: {
                        Image(systemName: "arrow.up.circle.fill")
                            .font(.title2)
                            .foregroundStyle(Brand.cyan)
                    }
                }
                .padding()
            }
            .background(Brand.shell)
            .navigationTitle("Henry")
        }
    }
}
