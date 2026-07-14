// Agents at work — "what are my agents doing, from my pocket" (#1).
//
// The dispatch brief specified GET /api/runs, but that endpoint does NOT exist
// on the hub (no route, no desktop RunRoster — verified by grep). Rather than
// call a fake endpoint, this screen is composed from REAL routes that together
// answer the same question honestly:
//   • WORKING NOW  — running jobs from GET /schedule/list (currently_running),
//                    each interruptible via POST /schedule/{id}/kill (the Stop).
//   • BACKGROUND AGENTS — the worker roster from GET /api/agent/workers, with
//                    each worker's live availability + engine.
// It live-updates over the hub's /events WebSocket (work-state events only, so a
// chat stream doesn't churn it). If a unified /api/runs (worker|schedule|session
// with live status) is added later, this screen can swap to it.

import SwiftUI

/// A background worker persona from GET /api/agent/workers (the daemon returns a
/// JSON object keyed by worker key → this shape). Only the fields this screen
/// renders are decoded; extra keys (traits, tone, nickname…) are ignored. Field
/// names are the snake_case wire keys (plain JSONDecoder, no key strategy).
struct AgentWorker: Decodable {
    let display_name: String
    let role: String
    let engine: String
    let available: Bool
    let reason: String?

    var engineLabel: String { engine.replacingOccurrences(of: "_", with: " ") }
}

/// One roster entry — the map key is stable and unique, so it's the identity.
struct WorkerEntry: Identifiable {
    let id: String
    let worker: AgentWorker
}

struct AgentsView: View {
    @State private var running: [ScheduleJob] = []
    @State private var workers: [WorkerEntry] = []
    @State private var loaded = false
    @State private var errorText: String?
    @State private var busy: Set<String> = []
    @State private var stopCount = 0

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                if let errorText {
                    HubErrorCard(text: errorText) { await load() }
                }

                // Working now — the interruptible, live rows.
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 8) {
                            Text("WORKING NOW")
                                .font(.brandLabel)
                                .foregroundStyle(Brand.textDim)
                            if !running.isEmpty {
                                Text("\(running.count)")
                                    .font(.brandLabel)
                                    .foregroundStyle(Brand.cyan)
                            }
                            Spacer()
                        }
                        if running.isEmpty {
                            Text(loaded
                                 ? "Nothing running right now. When an automation fires, it appears here — with a Stop button."
                                 : "Checking…")
                                .font(.brandCaption)
                                .foregroundStyle(Brand.textMuted)
                        } else {
                            ForEach(running) { job in runningRow(job) }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                // Background agents — the roster (who your agents are + readiness).
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("BACKGROUND AGENTS")
                            .font(.brandLabel)
                            .foregroundStyle(Brand.textDim)
                        if workers.isEmpty {
                            Text(loaded
                                 ? "No workers configured. Henry runs solo until you add background agents on the desktop."
                                 : "Loading…")
                                .font(.brandCaption)
                                .foregroundStyle(Brand.textMuted)
                        } else {
                            ForEach(workers) { entry in workerRow(entry.worker) }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                GlassCard {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("REMOTE HANDS")
                            .font(.brandLabel)
                            .foregroundStyle(Brand.textDim)
                        Text("These agents run on your Mac. Stop a job here and it halts on the hub — your desktop updates live.")
                            .font(.brandCaption)
                            .foregroundStyle(Brand.textMuted)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding()
        }
        .background(Brand.shell)
        .navigationTitle("Agents at work")
        .refreshable { await load() }
        .task { await load() }
        .task { await subscribeLive() }
        .sensoryFeedback(.impact(weight: .medium), trigger: stopCount)
    }

    private func runningRow(_ job: ScheduleJob) -> some View {
        HStack(spacing: 10) {
            PulseDot(color: Brand.cyan)
            VStack(alignment: .leading, spacing: 2) {
                Text(job.name)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.text)
                    .lineLimit(1)
                if let last = RelativeTime.string(from: job.last_run) {
                    Text("started \(last)")
                        .font(.caption2)
                        .foregroundStyle(Brand.textDim)
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
                    .foregroundStyle(Brand.deepVoid)
                    .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(busy.contains(job.id))
        }
    }

    private func workerRow(_ w: AgentWorker) -> some View {
        HStack(spacing: 10) {
            Circle()
                .fill(w.available ? Brand.success : Brand.textDim)
                .frame(width: 8, height: 8)
                .shadow(color: w.available ? Brand.success.opacity(0.6) : .clear, radius: 5)
            VStack(alignment: .leading, spacing: 2) {
                Text(w.display_name)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.text)
                Text(w.available ? w.role : (w.reason ?? "Unavailable"))
                    .font(.caption2)
                    .foregroundStyle(w.available ? Brand.textMuted : Brand.warning)
                    .lineLimit(1)
            }
            Spacer()
            Text(w.engineLabel)
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(Brand.textDim)
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
        let workerMap = try? await APIClient.shared.get("/api/agent/workers", as: [String: AgentWorker].self)

        if let jobs { running = jobs.filter { $0.currently_running } }
        if let workerMap {
            workers = workerMap
                .sorted { $0.key < $1.key }
                .map { WorkerEntry(id: $0.key, worker: $0.value) }
        }
        loaded = true
        errorText = (jobs == nil && workerMap == nil)
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
