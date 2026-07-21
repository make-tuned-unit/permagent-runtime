// Automations — the phone view of the hub's scheduler (#4). Renders GET
// /schedule/list and drives the command endpoints: run now / pause / unpause /
// stop (kill). Everything acts on the hub; this screen only shows the truth and
// relays taps.
//
// The row model decodes the REAL serialized `ScheduledJob` (flattened by the
// server's EnrichedJob). NOTE: the dispatch brief named #694 fields
// (last_status, retry_count/max_retries, at/every_seconds/tz) — those are NOT in
// the current serialization, so they are not decoded here. The real, present
// signal is: `cron`, `last_run`, `currently_running`, `paused`, `worker_persona`
// (+ EnrichedJob's display_name/description). Status is derived from those.

import SwiftUI

/// One scheduled job. snake_case field names match the server JSON exactly (the
/// APIClient uses a plain JSONDecoder — no key strategy — so property names are
/// the wire keys, per the HomeView/Views convention). Shared with AgentsView.
struct ScheduleJob: Decodable, Identifiable {
    let id: String
    let cron: String
    let last_run: String?
    let currently_running: Bool
    let paused: Bool
    let worker_persona: String?
    let display_name: String?
    let description: String?

    var name: String {
        if let d = display_name, !d.isEmpty { return d }
        return id
    }

    var statusLabel: String {
        if currently_running { return "Running" }
        if paused { return "Paused" }
        return "Scheduled"
    }

    var statusColor: Color {
        if currently_running { return Brand.cyan }
        if paused { return Brand.warning }
        return Brand.textDim
    }
}

struct SchedulesView: View {
    @State private var jobs: [ScheduleJob] = []
    @State private var loaded = false
    @State private var errorText: String?
    @State private var busy: Set<String> = []
    @State private var actionCount = 0

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                if let errorText {
                    HubErrorCard(text: errorText) { await load() }
                }

                if !loaded && jobs.isEmpty && errorText == nil {
                    ProgressView()
                        .tint(Brand.cyan)
                        .padding(.top, 40)
                }

                if loaded && jobs.isEmpty && errorText == nil {
                    GlassCard {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("NO AUTOMATIONS")
                                .font(.brandLabel)
                                .foregroundStyle(Brand.textDim)
                            Text("You haven't set up any scheduled jobs yet. Ask Henry to create one, or add it from the desktop — it'll show here with run and pause controls.")
                                .font(.brandCaption)
                                .foregroundStyle(Brand.textMuted)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                ForEach(jobs) { job in
                    jobCard(job)
                }
            }
            .padding()
        }
        .background(Brand.shell)
        .navigationTitle("Automations")
        .refreshable { await load() }
        .task { await load() }
        .sensoryFeedback(.success, trigger: actionCount)
    }

    private func jobCard(_ job: ScheduleJob) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 8) {
                    if job.currently_running {
                        PulseDot(color: Brand.cyan)
                    } else {
                        Circle().fill(job.statusColor).frame(width: 8, height: 8)
                    }
                    Text(job.name)
                        .font(.brandHeadline)
                        .foregroundStyle(Brand.text)
                        .lineLimit(1)
                    Spacer()
                    Text(job.statusLabel.uppercased())
                        .font(.brandLabel)
                        .foregroundStyle(job.statusColor)
                }

                if let desc = job.description, !desc.isEmpty {
                    Text(desc)
                        .font(.brandCaption)
                        .foregroundStyle(Brand.textMuted)
                        .lineLimit(3)
                }

                HStack(spacing: 10) {
                    Label(job.cron, systemImage: "calendar")
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(Brand.textDim)
                        .lineLimit(1)
                    if let persona = job.worker_persona, !persona.isEmpty {
                        Label(persona, systemImage: "person.fill")
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(Brand.textDim)
                    }
                }

                if let last = RelativeTime.string(from: job.last_run) {
                    Text("Last run \(last)")
                        .font(.caption2)
                        .foregroundStyle(Brand.textDim)
                } else {
                    Text("Not run yet")
                        .font(.caption2)
                        .foregroundStyle(Brand.textDim)
                }

                actions(for: job)
            }
        }
    }

    @ViewBuilder
    private func actions(for job: ScheduleJob) -> some View {
        HStack(spacing: 10) {
            if job.currently_running {
                actionButton(job, verb: "kill", label: "Stop",
                             tint: Brand.deepVoid, fill: Brand.danger)
            } else {
                actionButton(job, verb: "run_now", label: "Run now",
                             tint: Brand.deepVoid, fill: Brand.cyan)
                if job.paused {
                    actionButton(job, verb: "unpause", label: "Resume",
                                 tint: Brand.text, fill: Brand.surface)
                } else {
                    actionButton(job, verb: "pause", label: "Pause",
                                 tint: Brand.textMuted, fill: Brand.surface)
                }
            }
        }
        .padding(.top, 2)
    }

    private func actionButton(
        _ job: ScheduleJob, verb: String, label: String, tint: Color, fill: Color
    ) -> some View {
        Button {
            command(job, verb)
        } label: {
            Text(busy.contains(job.id) ? "…" : label)
                .font(.caption.weight(.semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 9)
                .background(fill)
                .foregroundStyle(tint)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(busy.contains(job.id))
    }

    private func command(_ job: ScheduleJob, _ verb: String) {
        guard !busy.contains(job.id) else { return }
        busy.insert(job.id)
        errorText = nil
        Task {
            do {
                try await APIClient.shared.send("/schedule/\(job.id)/\(verb)")
                actionCount += 1
                await load()
            } catch {
                errorText = "Couldn't \(label(for: verb)) \(job.name) — check the hub, or use the desktop."
            }
            busy.remove(job.id)
        }
    }

    private func label(for verb: String) -> String {
        switch verb {
        case "run_now": return "run"
        case "kill": return "stop"
        case "pause": return "pause"
        case "unpause": return "resume"
        default: return "update"
        }
    }

    private func load() async {
        struct Resp: Decodable { let jobs: [ScheduleJob] }
        do {
            let resp = try await APIClient.shared.get("/schedule/list", as: Resp.self)
            jobs = resp.jobs
            errorText = nil
        } catch {
            errorText = "Couldn't reach the hub — is your Mac awake and on the tailnet?"
        }
        loaded = true
    }
}
