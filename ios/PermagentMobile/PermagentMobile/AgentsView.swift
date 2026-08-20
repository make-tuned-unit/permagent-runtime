// Agents at work — "what are my agents doing, from my pocket".
//
// Same merged surface as Settings → Agents on the desktop:
//   • WORKING NOW     — running jobs from GET /schedule/list
//   • BACKGROUND      — GET /api/agents/roster workers, each with the same
//                       on/off gate the Mac writes (`/config/upsert`)
//   • DISPATCH        — the dispatch roster (availability + engine)
//
// A missing or malformed `gate` renders no switch — never a toggle that
// claims off for a key the daemon does not read.

import SwiftUI

struct AgentsView: View {
    @ObservedObject private var identity = AgentIdentity.shared
    @State private var running: [ScheduleJob] = []
    @State private var workers: [RosterWorker] = []
    @State private var dispatch: [DispatchPersona] = []
    @State private var loaded = false
    @State private var errorText: String?
    @State private var busy: Set<String> = []
    @State private var stopCount = 0
    @State private var gateEnabled: [String: Bool] = [:]

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                if let errorText {
                    HubErrorCard(text: errorText) { await load() }
                }

                RaisedCard {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 8) {
                            Text("WORKING NOW")
                                .font(.brandLabel).tracking(0.88)
                                .foregroundStyle(ChatSurface.dim)
                            if !running.isEmpty {
                                Text("\(running.count)")
                                    .font(.brandLabel)
                                    .foregroundStyle(ChatSurface.spark)
                            }
                            Spacer()
                        }
                        if running.isEmpty {
                            Text(loaded
                                 ? "Nothing running right now. When an automation fires, it appears here — with a Stop button."
                                 : "Checking…")
                                .font(.brandCaption)
                                .foregroundStyle(ChatSurface.muted)
                        } else {
                            ForEach(running) { job in runningRow(job) }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                RaisedCard {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("BACKGROUND AGENTS")
                            .font(.brandLabel).tracking(0.88)
                            .foregroundStyle(ChatSurface.dim)
                        if workers.isEmpty {
                            Text(loaded
                                 ? "No workers configured. \(identity.nameCapitalized) runs solo until you add background agents on the desktop."
                                 : "Loading…")
                                .font(.brandCaption)
                                .foregroundStyle(ChatSurface.muted)
                        } else {
                            ForEach(workers) { worker in workerRow(worker) }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                if !dispatch.isEmpty {
                    RaisedCard {
                        VStack(alignment: .leading, spacing: 12) {
                            Text("DISPATCH")
                                .font(.brandLabel).tracking(0.88)
                                .foregroundStyle(ChatSurface.dim)
                            ForEach(dispatch) { persona in dispatchRow(persona) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                RaisedCard {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("REMOTE HANDS")
                            .font(.brandLabel).tracking(0.88)
                            .foregroundStyle(ChatSurface.dim)
                        Text("These agents run on your Mac. A switch here is the same key as Settings → Agents on the desktop — flip either, both update. Stop a job here and it halts on the hub.")
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.muted)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding()
        }
        .background(ChatSurface.bg.ignoresSafeArea())
        .navigationTitle("Agents at work")
        .refreshable { await load() }
        .task { await load() }
        .task { await subscribeLive() }
        .sensoryFeedback(.impact(weight: .medium), trigger: stopCount)
    }

    private func runningRow(_ job: ScheduleJob) -> some View {
        HStack(spacing: 10) {
            PulseDot(color: ChatSurface.spark)
            VStack(alignment: .leading, spacing: 2) {
                Text(job.name)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.text)
                    .lineLimit(1)
                if let last = RelativeTime.string(from: job.last_run) {
                    Text("started \(last)")
                        .font(.caption2)
                        .foregroundStyle(ChatSurface.dim)
                }
            }
            Spacer()
            Button {
                stop(job)
            } label: {
                Text(busy.contains(job.id) ? "…" : "Stop")
                    .font(.caption.weight(.semibold))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .background(Brand.danger)
                    .foregroundStyle(Brand.onDanger)
                    .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(busy.contains(job.id))
        }
    }

    private func workerRow(_ w: RosterWorker) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(liveDot(w.live_state))
                .frame(width: 8, height: 8)
                .padding(.top, 6)
            VStack(alignment: .leading, spacing: 2) {
                Text(w.display_name)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.text)
                Text(liveCaption(w))
                    .font(.caption2)
                    .foregroundStyle(ChatSurface.muted)
                    .lineLimit(2)
            }
            Spacer(minLength: 8)
            if let gate = w.gate {
                Toggle("", isOn: Binding(
                    get: { gateEnabled[gate.config_key] ?? gate.enabled },
                    set: { flip(gate.config_key, $0) }
                ))
                .labelsHidden()
                .tint(ChatSurface.spark)
            }
        }
    }

    private func dispatchRow(_ p: DispatchPersona) -> some View {
        HStack(spacing: 10) {
            Circle()
                .fill(availabilityDot(p.availability))
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 2) {
                Text(p.display_name)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.text)
                Text(availabilityCaption(p))
                    .font(.caption2)
                    .foregroundStyle(ChatSurface.muted)
                    .lineLimit(1)
            }
            Spacer()
            if !p.engineLabel.isEmpty {
                Text(p.engineLabel)
                    .font(.jetbrainsMono(11))
                    .foregroundStyle(ChatSurface.dim)
            }
            if let gate = p.gate {
                Toggle("", isOn: Binding(
                    get: { gateEnabled[gate.config_key] ?? gate.enabled },
                    set: { flip(gate.config_key, $0) }
                ))
                .labelsHidden()
                .tint(ChatSurface.spark)
            }
        }
    }

    private func liveDot(_ state: LiveState?) -> Color {
        guard let state else { return Brand.textDim }
        switch state.kind {
        case .ok: return Brand.success
        case .unavailable: return Brand.warning
        case .notQueryable: return Brand.textDim
        }
    }

    private func liveCaption(_ w: RosterWorker) -> String {
        if let state = w.live_state {
            switch state.kind {
            case .ok(let value) where !value.isEmpty: return value
            case .unavailable(let reason) where !reason.isEmpty: return reason
            default: break
            }
        }
        return w.what_it_does
    }

    private func availabilityDot(_ a: Availability?) -> Color {
        guard let a else { return Brand.textDim }
        switch a.kind {
        case .available: return Brand.success
        case .unavailable, .probeFailed: return Brand.warning
        }
    }

    private func availabilityCaption(_ p: DispatchPersona) -> String {
        if let a = p.availability {
            switch a.kind {
            case .available: return p.role
            case .unavailable(let r): return r.isEmpty ? p.role : r
            case .probeFailed(let r): return r.isEmpty ? "Probe failed" : r
            }
        }
        return p.role
    }

    private func flip(_ key: String, _ value: Bool) {
        let prev = gateEnabled[key] ?? false
        gateEnabled[key] = value
        errorText = nil
        Task {
            do {
                try await APIClient.shared.upsertConfig(key, value: value)
            } catch {
                gateEnabled[key] = prev
                errorText = "Couldn't update that switch — is the hub awake?"
            }
        }
    }

    private func stop(_ job: ScheduleJob) {
        guard !busy.contains(job.id) else { return }
        busy.insert(job.id)
        errorText = nil
        Task {
            do {
                try await APIClient.shared.send("/schedule/\(job.id)/kill")
                stopCount += 1
                await load()
            } catch {
                errorText = "Couldn't stop \(job.name) — check the hub, or use the desktop."
            }
            busy.remove(job.id)
        }
    }

    private func load() async {
        struct SchedulesResp: Decodable { let jobs: [ScheduleJob] }
        let jobs = (try? await APIClient.shared.get("/schedule/list", as: SchedulesResp.self))?.jobs
        let roster = try? await APIClient.shared.agentRoster()

        if let jobs { running = jobs.filter { $0.currently_running } }
        if let roster {
            workers = roster.workers
            dispatch = roster.dispatch_roster
            for w in roster.workers {
                if let g = w.gate { gateEnabled[g.config_key] = g.enabled }
            }
            for p in roster.dispatch_roster {
                if let g = p.gate { gateEnabled[g.config_key] = g.enabled }
            }
        }
        loaded = true
        errorText = (jobs == nil && roster == nil)
            ? "Couldn't reach the hub — is your Mac awake and on the tailnet?"
            : nil
    }

    /// Live refresh over /events. Reloads only on work-state events so a chat
    /// stream (message_received / stream_chunk) doesn't thrash the roster.
    private func subscribeLive() async {
        guard let stream = await APIClient.shared.eventStream() else { return }
        let triggers: Set<String> = [
            "agent_state_changed", "goal_state_changed",
            "task_started", "task_completed", "task_failed",
        ]
        for await event in stream where triggers.contains(event.type) {
            await load()
        }
    }
}
