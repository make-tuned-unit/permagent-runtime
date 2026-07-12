// Home — the key-info surface (Jesse's rule 2026-07-11): one glance answers
// "is my system healthy, what needs me, what's happening". Pull-to-refresh +
// live /events nudges. Every number is the hub's truth; nothing is computed
// on-device.

import SwiftUI

struct HubSnapshot {
    var healthy: Bool?
    var decisionsPending: Int?
    var goalsActive: Int?
    var activity: [ActivityRow] = []
}

struct ActivityRow: Decodable, Identifiable {
    let id: String
    let ts: String
    let kind: String
    let title: String
}

struct HomeView: View {
    @EnvironmentObject var session: HubSession
    @State private var snap = HubSnapshot()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 14) {
                    // Hub health — the always-on machine, at a glance.
                    GlassCard {
                        HStack(spacing: 10) {
                            Circle()
                                .fill(snap.healthy == false ? Brand.danger : Brand.cyan)
                                .frame(width: 9, height: 9)
                                .shadow(color: Brand.cyanGlow, radius: snap.healthy == false ? 0 : 6)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(snap.healthy == false ? "Hub unreachable" : "Hub online")
                                    .font(.brandHeadline)
                                    .foregroundStyle(Brand.text)
                                Text(snap.healthy == false
                                     ? "Check that your Mac is awake and on the tailnet."
                                     : "Your Mac is holding the fort — Brain, models, and memory all live.")
                                    .font(.brandCaption)
                                    .foregroundStyle(Brand.textMuted)
                            }
                            Spacer()
                        }
                    }

                    // The two numbers that matter.
                    HStack(spacing: 14) {
                        statTile("Decisions", value: snap.decisionsPending, accent: Brand.violet,
                                 hint: "waiting for you")
                        statTile("In flight", value: snap.goalsActive, accent: Brand.cyan,
                                 hint: "goals running")
                    }

                    // Recent activity (the hub's durable journal).
                    GlassCard {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("RECENT ACTIVITY")
                                .font(.brandLabel)
                                .foregroundStyle(Brand.textDim)
                            if snap.activity.isEmpty {
                                Text("All quiet. Henry will note goal moves, decisions, and Librarian passes here.")
                                    .font(.brandCaption)
                                    .foregroundStyle(Brand.textMuted)
                            } else {
                                ForEach(snap.activity.prefix(8)) { row in
                                    HStack(alignment: .top, spacing: 8) {
                                        Text(icon(for: row.kind))
                                        VStack(alignment: .leading, spacing: 1) {
                                            Text(row.title)
                                                .font(.brandCaption)
                                                .foregroundStyle(Brand.text)
                                                .lineLimit(2)
                                            Text(row.kind.replacingOccurrences(of: "_", with: " "))
                                                .font(.system(.caption2, design: .monospaced))
                                                .foregroundStyle(Brand.textDim)
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // The companion superpower, said plainly.
                    GlassCard {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("REMOTE HANDS")
                                .font(.brandLabel)
                                .foregroundStyle(Brand.textDim)
                            Text("Anything you ask Henry here happens on the hub — open a site in the desktop browser, launch a project terminal, dispatch a goal. Your desktop shows it live.")
                                .font(.brandCaption)
                                .foregroundStyle(Brand.textMuted)
                        }
                    }
                }
                .padding()
            }
            .background(Brand.shell)
            .navigationTitle("Permagent")
            .refreshable { await load() }
            .task { await load() }
        }
    }

    private func statTile(_ label: String, value: Int?, accent: Color, hint: String) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 4) {
                Text(value.map(String.init) ?? "–")
                    .font(.brandDisplay)
                    .monospacedDigit()               // tabular figures — counts don't jitter on refresh
                    .contentTransition(.numericText()) // roll the digit when it changes
                    .foregroundStyle(accent)
                Text(label.uppercased())
                    .font(.brandLabel)
                    .foregroundStyle(Brand.text)
                Text(hint)
                    .font(.caption2)
                    .foregroundStyle(Brand.textDim)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func icon(for kind: String) -> String {
        switch kind {
        case let k where k.contains("decision"): return "🔔"
        case let k where k.contains("goal"): return "⚒️"
        case let k where k.contains("librarian"): return "📚"
        case let k where k.contains("fail"): return "⚠️"
        default: return "·"
        }
    }

    private func load() async {
        struct Status: Decodable { let status: String }
        // The inbox returns { items, summary:{ total_pending } } — the count
        // lives under summary (was read at top level, so it always showed "–").
        struct Summary: Decodable { let total_pending: Int? }
        struct Decisions: Decodable { let summary: Summary? }
        struct Goals: Decodable { struct G: Decodable { let id: String }; let goals: [G] }
        struct Activity: Decodable { let items: [ActivityRow]? }

        snap.healthy = (try? await APIClient.shared.get("/status", as: Status.self)) != nil
        if let d = try? await APIClient.shared.get("/api/decisions", as: Decisions.self) {
            withAnimation(Motion.spring) { snap.decisionsPending = d.summary?.total_pending }
        }
        if let g = try? await APIClient.shared.get("/api/goals/active", as: Goals.self) {
            withAnimation(Motion.spring) { snap.goalsActive = g.goals.count }
        }
        // The durable journal (#619); tolerant of the route not being live yet.
        if let a = try? await APIClient.shared.get("/api/activity", as: Activity.self) {
            snap.activity = a.items ?? []
        }
    }
}
