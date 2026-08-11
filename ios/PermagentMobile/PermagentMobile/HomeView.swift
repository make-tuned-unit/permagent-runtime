// Home — the key-info surface (Jesse's rule 2026-07-11): one glance answers
// "is my system healthy, what needs me, what's happening". Pull-to-refresh +
// live /events nudges. Every number is the hub's truth; nothing is computed
// on-device. Every card is a DOOR, not a poster: tiles jump to their section,
// activity rows land where the event lives, to-dos push the full list.

import SwiftUI

struct HubSnapshot {
    var healthy: Bool?
    var decisionsPending: Int?
    var goalsActive: Int?
    var activity: [ActivityRow] = []
    var todos: [DueCard] = []
}

struct ActivityRow: Decodable, Identifiable {
    let id: String
    let ts: String
    let kind: String
    let title: String
}

/// Springy press affordance shared by every tappable card — the card gives
/// slightly under the finger, so "this is a door" needs no chevron lecture.
struct PressableCard: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .opacity(configuration.isPressed ? 0.92 : 1)
            .animation(.spring(response: 0.28, dampingFraction: 0.7), value: configuration.isPressed)
    }
}

struct HomeView: View {
    @ObservedObject private var identity = AgentIdentity.shared
    @EnvironmentObject var session: HubSession
    @Binding var tab: AppTab
    @State private var snap = HubSnapshot()
    @State private var taps = 0

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 14) {
                    // Hub health — the always-on machine, at a glance.
                    RaisedCard {
                        HStack(spacing: 10) {
                            Circle()
                                .fill(snap.healthy == false ? Brand.danger : ChatSurface.spark)
                                .frame(width: 9, height: 9)
                                .shadow(color: Brand.cyanGlow, radius: snap.healthy == false ? 0 : 6)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(snap.healthy == false ? "Hub unreachable" : "Hub online")
                                    .font(.brandHeadline)
                                    .foregroundStyle(ChatSurface.text)
                                Text(snap.healthy == false
                                     ? "Check that your Mac is awake and on the tailnet."
                                     : "Your Mac is holding the fort — Brain, models, and memory all live.")
                                    .font(.brandCaption)
                                    .foregroundStyle(ChatSurface.muted)
                            }
                            Spacer()
                        }
                    }

                    // The two numbers that matter — each one a door.
                    HStack(spacing: 14) {
                        statTile("Decisions", value: snap.decisionsPending, accent: Brand.violet,
                                 hint: "waiting for you", destination: .decisions)
                        statTile("In flight", value: snap.goalsActive, accent: ChatSurface.spark,
                                 hint: "goals running", destination: .goals)
                    }

                    // To-dos — the desktop dashboard's due list, in your pocket.
                    NavigationLink {
                        TodosView()
                    } label: {
                        RaisedCard {
                            VStack(alignment: .leading, spacing: 10) {
                                HStack {
                                    Text("TO-DOS")
                                        .font(.brandLabel).tracking(0.88)
                                        .foregroundStyle(ChatSurface.dim)
                                    Spacer()
                                    if !snap.todos.isEmpty {
                                        Text("\(snap.todos.count)")
                                            .font(.manrope(12))
                                            .monospacedDigit()
                                            .foregroundStyle(ChatSurface.spark)
                                            .contentTransition(.numericText())
                                    }
                                    Image(systemName: "chevron.right")
                                        .font(.caption2)
                                        .foregroundStyle(ChatSurface.dim)
                                }
                                if snap.todos.isEmpty {
                                    Text("Nothing due. Dated cards from any project land here.")
                                        .font(.brandCaption)
                                        .foregroundStyle(ChatSurface.muted)
                                } else {
                                    ForEach(snap.todos.prefix(3)) { todo in
                                        HStack(spacing: 8) {
                                            Circle()
                                                .fill(dueBucket(todo.dueDate).accent)
                                                .frame(width: 5, height: 5)
                                            Text(todo.title)
                                                .font(.brandCaption)
                                                .foregroundStyle(ChatSurface.text)
                                                .lineLimit(1)
                                            Spacer(minLength: 6)
                                            Text(dueLabel(todo.dueDate))
                                                .font(.inter(11, weight: .semibold))
                                                .foregroundStyle(dueBucket(todo.dueDate).accent)
                                        }
                                    }
                                }
                            }
                        }
                    }
                    .buttonStyle(PressableCard())

                    // Recent activity (the hub's durable journal). Rows land
                    // where the event lives.
                    RaisedCard {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("RECENT ACTIVITY")
                                .font(.brandLabel).tracking(0.88)
                                .foregroundStyle(ChatSurface.dim)
                            if snap.activity.isEmpty {
                                Text("All quiet. \(identity.nameCapitalized) will note goal moves, decisions, and Librarian passes here.")
                                    .font(.brandCaption)
                                    .foregroundStyle(ChatSurface.muted)
                            } else {
                                ForEach(snap.activity.prefix(8)) { row in
                                    activityRow(row)
                                }
                            }
                        }
                    }

                    // The companion superpower, said plainly — and a door to it.
                    Button {
                        taps += 1
                        tab = .chat
                    } label: {
                        RaisedCard {
                            VStack(alignment: .leading, spacing: 6) {
                                Text("REMOTE HANDS")
                                    .font(.brandLabel).tracking(0.88)
                                    .foregroundStyle(ChatSurface.dim)
                                Text("Anything you ask \(identity.name) here happens on the hub — open a site in the desktop browser, launch a project terminal, dispatch a goal. Your desktop shows it live.")
                                    .font(.brandCaption)
                                    .foregroundStyle(ChatSurface.muted)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    .buttonStyle(PressableCard())
                }
                .padding()
            }
            .background(ChatSurface.bg.ignoresSafeArea())
            .toolbar(.hidden, for: .navigationBar)
            .safeAreaInset(edge: .top, spacing: 0) { pageHeader }
            .refreshable { await load() }
            .task { await load() }
            // Tactile: navigation taps land with a light tap.
            .sensoryFeedback(.impact(weight: .light), trigger: taps)
        }
    }

    /// In-page title — the tab hides the system bar in favor of this.
    private var pageHeader: some View {
        HStack {
            Text("Permagent")
                .font(.brandTitle)
                .foregroundStyle(ChatSurface.text)
            Spacer()
        }
        .padding(.horizontal, 18)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .background(ChatSurface.bg)
    }

    private func statTile(_ label: String, value: Int?, accent: Color, hint: String,
                          destination: AppTab) -> some View {
        Button {
            taps += 1
            tab = destination
        } label: {
            RaisedCard {
                VStack(alignment: .leading, spacing: 4) {
                    Text(value.map(String.init) ?? "–")
                        .font(.brandDisplay)
                        .tracking(-0.64)                 // tokens.ts display -0.02em at 32pt
                        .monospacedDigit()               // tabular figures — counts don't jitter on refresh
                        .contentTransition(.numericText()) // roll the digit when it changes
                        .foregroundStyle(accent)
                    Text(label.uppercased())
                        .font(.brandLabel).tracking(0.88)
                        .foregroundStyle(ChatSurface.text)
                    HStack(spacing: 3) {
                        Text(hint)
                            .font(.brandMicro)
                            .foregroundStyle(ChatSurface.dim)
                        Image(systemName: "chevron.right")
                            .font(.system(size: 8, weight: .semibold))
                            .foregroundStyle(ChatSurface.dim)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .buttonStyle(PressableCard())
        .accessibilityLabel("\(label): \(value.map(String.init) ?? "unknown"), \(hint). Opens \(label).")
    }

    /// Where an activity event "lives": decision events land in Decisions,
    /// goal events in In Flight; everything else stays informational.
    private func destination(for kind: String) -> AppTab? {
        if kind.contains("decision") { return .decisions }
        if kind.contains("goal") || kind.contains("dispatch") { return .goals }
        if kind.contains("automation") || kind.contains("schedule") { return .control }
        return nil
    }

    @ViewBuilder
    private func activityRow(_ row: ActivityRow) -> some View {
        let dest = destination(for: row.kind)
        let icon = icon(for: row.kind)
        let content = HStack(alignment: .top, spacing: 8) {
            Image(systemName: icon.symbol)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(icon.tint)
                .frame(width: 16, height: 16)
            VStack(alignment: .leading, spacing: 1) {
                Text(row.title)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.text)
                    .lineLimit(2)
                Text(row.kind.replacingOccurrences(of: "_", with: " "))
                    .font(.jetbrainsMono(11))
                    .foregroundStyle(ChatSurface.dim)
            }
            Spacer(minLength: 0)
            if dest != nil {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(ChatSurface.dim)
            }
        }
        .contentShape(Rectangle())

        if let dest {
            Button {
                taps += 1
                tab = dest
            } label: { content }
            .buttonStyle(.plain)
        } else {
            content
        }
    }

    /// Activity glyphs speak the brand accents: violet for decisions (the
    /// established Decisions color), spark for goal motion, danger for
    /// failures — SF Symbols, not emoji, matching the desktop's icon voice.
    private func icon(for kind: String) -> (symbol: String, tint: Color) {
        switch kind {
        case let k where k.contains("decision"): return ("bell.fill", Brand.violet)
        case let k where k.contains("goal") || k.contains("dispatch"): return ("hammer.fill", ChatSurface.spark)
        case let k where k.contains("librarian"): return ("books.vertical.fill", ChatSurface.muted)
        case let k where k.contains("fail") || k.contains("error"): return ("exclamationmark.triangle.fill", Brand.danger)
        default: return ("circle.fill", ChatSurface.dim)
        }
    }

    private func load() async {
        // The inbox returns { items, summary:{ total_pending } } — the count
        // lives under summary (was read at top level, so it always showed "–").
        struct Summary: Decodable { let total_pending: Int? }
        struct Decisions: Decodable { let summary: Summary? }
        struct Goals: Decodable { struct G: Decodable { let id: String }; let goals: [G] }
        struct Activity: Decodable { let items: [ActivityRow]? }

        // Reachability is the HTTP STATUS, not the body shape. `GET /status`
        // answers 200 with the bare string `ok` — not JSON — so decoding it as
        // `{ status: String }` always threw, `try?` swallowed it, and the phone
        // reported "Hub unreachable" while every other call on the same
        // connection succeeded and rendered live hub data (reported 2026-08-04,
        // with Recent Activity populated behind the red banner).
        //
        // `send` checks 2xx and discards the body, which is exactly the question
        // being asked. Never reintroduce a decode here: a health check that
        // fails on payload shape cannot distinguish "hub is down" from "hub
        // answered in a format I didn't expect", and those need opposite fixes.
        snap.healthy = ((try? await APIClient.shared.send("/status", method: "GET")) != nil)
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
        if let due = try? await APIClient.shared.get("/api/cards/due", as: [DueCard].self) {
            withAnimation(Motion.spring) { snap.todos = due }
        }
    }
}
