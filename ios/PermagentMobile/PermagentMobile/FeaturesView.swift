// Features — the same off-by-default worker switches as Settings → Features
// on the desktop. Each flag is one `/config/upsert` boolean the daemon
// re-reads on its next tick, so a flip here and a flip on the Mac cannot
// drift: they write the same key. Copy and keys mirror
// `ui/command-center/src/components/settings/features/features.ts`.

import SwiftUI

private struct FeatureRow: Identifiable {
    let key: String
    let label: String
    let what: String
    let effect: String
    var id: String { key }
}

private let featureRows: [FeatureRow] = [
    FeatureRow(
        key: "initiative_enabled",
        label: "Initiative",
        what: "Watches your activity for a terminal command you keep repeating and, once you have gone quiet, proposes automating it on the Decision Inbox. It only ever proposes.",
        effect: "Off by default. Takes effect at the next tick (about a minute), no restart."
    ),
    FeatureRow(
        key: "playbook_enabled",
        label: "Decision Playbook",
        what: "Periodically distills your answered decisions and draft edits into a few provenance-linked hints about how you tend to decide, and recalls them when a roadmap is planned. Hints, never rules.",
        effect: "Off by default. Takes effect at the next tick, no restart."
    ),
    FeatureRow(
        key: "concierge_enabled",
        label: "Concierge",
        what: "Reads your Gmail inbox read-only on the local model, flags what needs you, and proposes an editable reply draft as a Decision-Inbox card. It can never send or change mail.",
        effect: "Off by default. Takes effect at the next tick (up to a few hours), no restart."
    ),
    FeatureRow(
        key: "steward_scan_enabled",
        label: "Steward git-health",
        what: "Sweeps every git repo under your code folders for leftover extra checkouts (worktrees) and merged local branches taking disk space. Files one Decision-Inbox approval per cleanup — proposals only; nothing is deleted until you approve.",
        effect: "Off by default. Takes effect within about 15 minutes, no restart."
    ),
    FeatureRow(
        key: "strix_enabled",
        label: "The Guard (security sweeps)",
        what: "Sweeps one of your own projects per pass for exposed secrets, vulnerable dependencies, and access-control weaknesses, and files a security report as a note on that project. It reports only: it never edits code. Needs Docker and the external strix scanner, and each sweep spends API credits.",
        effect: "Off by default. Takes effect within about 15 minutes, no restart."
    ),
]

struct FeaturesView: View {
    @State private var flags: [String: Bool?] = Dictionary(
        uniqueKeysWithValues: featureRows.map { ($0.key, nil) }
    )
    @State private var errors: [String: String] = [:]
    @State private var gmailToken: Bool?

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                Text("Workers that are off until you switch them on. Each flip is written to the hub and picked up at the next tick — no restart. The same switches live under Settings → Features on the Mac.")
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                    .frame(maxWidth: .infinity, alignment: .leading)

                ForEach(featureRows) { row in
                    RaisedCard {
                        HStack(alignment: .top, spacing: 12) {
                            VStack(alignment: .leading, spacing: 6) {
                                Text(row.label)
                                    .font(.brandHeadline)
                                    .foregroundStyle(ChatSurface.text)
                                Text(row.what)
                                    .font(.brandCaption)
                                    .foregroundStyle(ChatSurface.muted)
                                Text(row.effect)
                                    .font(.caption2)
                                    .foregroundStyle(ChatSurface.dim)
                                if row.key == "concierge_enabled" {
                                    Text(conciergeCopy)
                                        .font(.caption2)
                                        .foregroundStyle(gmailToken == false ? ChatSurface.text : ChatSurface.dim)
                                }
                                if let err = errors[row.key] {
                                    Text(err)
                                        .font(.caption2)
                                        .foregroundStyle(Brand.danger)
                                }
                            }
                            Spacer(minLength: 8)
                            if let value = flags[row.key], let value {
                                Toggle("", isOn: Binding(
                                    get: { value },
                                    set: { save(row.key, $0) }
                                ))
                                .labelsHidden()
                                .tint(ChatSurface.spark)
                            } else {
                                ProgressView().controlSize(.small).tint(ChatSurface.spark)
                            }
                        }
                    }
                }
            }
            .padding()
        }
        .background(ChatSurface.bg.ignoresSafeArea())
        .navigationTitle("Features")
        .task { await load() }
    }

    private var conciergeCopy: String {
        switch gmailToken {
        case true: return "Gmail token present."
        case false: return "Needs a Gmail token: run `permagent integrations connect gmail`. Until then the loop stays idle."
        case nil: return "Checking for a Gmail token…"
        }
    }

    private func load() async {
        await withTaskGroup(of: (String, Bool).self) { group in
            for row in featureRows {
                group.addTask {
                    (row.key, await APIClient.shared.readConfigFlag(row.key))
                }
            }
            for await (key, value) in group {
                flags[key] = value
            }
        }
        let list = try? await APIClient.shared.integrations()
        if let list {
            gmailToken = list.first(where: { $0.provider == "gmail" })?.token_present ?? false
        } else {
            gmailToken = false
        }
    }

    private func save(_ key: String, _ value: Bool) {
        let prev = flags[key] ?? false
        flags[key] = value
        errors[key] = nil
        Task {
            do {
                try await APIClient.shared.upsertConfig(key, value: value)
            } catch {
                flags[key] = prev
                errors[key] = "Couldn't save — is the hub awake?"
            }
        }
    }
}
