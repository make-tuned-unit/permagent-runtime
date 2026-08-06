// Past conversations — the sidebar every chat app has, in a sheet.
//
// The bar is Claude/ChatGPT: one tap to get here, conversations grouped by
// when you had them, searchable, swipe to delete, and the one you are in
// marked. Returning to a thread is not a recovery move, it is the normal way
// to use the app, so nothing here is behind a confirmation.
//
// Only real conversations are offered: the hub also runs scheduled jobs and
// worker sessions by the hundreds, which were never threads the user was part
// of, and empty shells (a session minted but never spoken to).

import SwiftUI

struct SessionSummary: Decodable, Identifiable, Equatable {
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

    /// The hub auto-titles most threads from their content. "New Chat" is its
    /// placeholder, which is no more use to a person than a blank.
    var displayTitle: String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty || trimmed.caseInsensitiveCompare("New Chat") == .orderedSame {
            return "Untitled conversation"
        }
        return trimmed
    }
}

/// When a conversation happened, in the buckets people actually think in.
enum ChatAge: String, CaseIterable {
    case today = "TODAY"
    case yesterday = "YESTERDAY"
    case week = "PREVIOUS 7 DAYS"
    case earlier = "EARLIER"
}

func chatAge(_ raw: String?, now: Date = Date()) -> ChatAge {
    guard let date = parseHubDate(raw) else { return .earlier }
    let cal = Calendar.current
    if cal.isDateInToday(date) { return .today }
    if cal.isDateInYesterday(date) { return .yesterday }
    if let weekAgo = cal.date(byAdding: .day, value: -7, to: now), date >= weekAgo { return .week }
    return .earlier
}

/// Tolerant of both shapes the hub emits (RFC3339 with or without fractional
/// seconds, and SQLite's "yyyy-MM-dd HH:mm:ss" in UTC).
func parseHubDate(_ raw: String?) -> Date? {
    guard let raw, !raw.isEmpty else { return nil }
    let iso = ISO8601DateFormatter()
    iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let d = iso.date(from: raw) { return d }
    iso.formatOptions = [.withInternetDateTime]
    if let d = iso.date(from: raw) { return d }
    let df = DateFormatter()
    df.locale = Locale(identifier: "en_US_POSIX")
    df.timeZone = TimeZone(identifier: "UTC")
    df.dateFormat = "yyyy-MM-dd HH:mm:ss"
    return df.date(from: raw)
}

struct ChatHistorySheet: View {
    let currentSessionId: String?
    let onPick: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var sessions: [SessionSummary] = []
    @State private var loaded = false
    @State private var error: String?
    @State private var query = ""

    private var filtered: [SessionSummary] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !q.isEmpty else { return sessions }
        return sessions.filter { $0.displayTitle.lowercased().contains(q) }
    }

    private var grouped: [(ChatAge, [SessionSummary])] {
        let byAge = Dictionary(grouping: filtered) { chatAge($0.updatedAt) }
        return ChatAge.allCases.compactMap { age in
            guard let rows = byAge[age], !rows.isEmpty else { return nil }
            return (age, rows)
        }
    }

    var body: some View {
        NavigationStack {
            Group {
                if let error {
                    ScrollView { HubErrorCard(text: error) { await load() }.padding() }
                } else if !loaded {
                    ProgressView().tint(Brand.cyan).frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if sessions.isEmpty {
                    emptyState("No conversations yet",
                               "Threads you start will be listed here for you to return to.")
                } else if filtered.isEmpty {
                    emptyState("No matches", "Nothing here matches “\(query)”.")
                } else {
                    List {
                        ForEach(grouped, id: \.0) { age, rows in
                            Section {
                                ForEach(rows) { s in
                                    Button { onPick(s.id) } label: { row(s) }
                                        .listRowBackground(Brand.surface)
                                        .swipeActions(edge: .trailing) {
                                            Button(role: .destructive) {
                                                Task { await delete(s) }
                                            } label: { Label("Delete", systemImage: "trash") }
                                        }
                                }
                            } header: {
                                Text(age.rawValue)
                                    .font(.brandLabel)
                                    .foregroundStyle(Brand.textDim)
                            }
                        }
                    }
                    .listStyle(.insetGrouped)
                    .scrollContentBackground(.hidden)
                    .refreshable { await load() }
                }
            }
            .background(Brand.shell)
            .navigationTitle("Conversations")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $query, prompt: "Search conversations")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") { dismiss() }
                }
            }
            .task { await load() }
        }
    }

    private func emptyState(_ title: String, _ body: String) -> some View {
        VStack(spacing: 8) {
            Text(title).font(.brandHeadline).foregroundStyle(Brand.text)
            Text(body)
                .font(.brandCaption)
                .foregroundStyle(Brand.textMuted)
                .multilineTextAlignment(.center)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func row(_ s: SessionSummary) -> some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text(s.displayTitle)
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
                Text("CURRENT").font(.brandLabel).foregroundStyle(Brand.cyan)
            } else {
                Image(systemName: "chevron.right")
                    .font(.caption2)
                    .foregroundStyle(Brand.textDim)
            }
        }
        .contentShape(Rectangle())
    }

    /// Delete on the hub. Optimistic — the row leaves immediately and comes
    /// back if the hub refuses, so a failure is never silent.
    private func delete(_ s: SessionSummary) async {
        let snapshot = sessions
        withAnimation { sessions.removeAll { $0.id == s.id } }
        do {
            try await APIClient.shared.send("/api/sessions/\(s.id)", method: "DELETE")
        } catch {
            withAnimation { sessions = snapshot }
            self.error = "Couldn't delete that conversation."
        }
    }

    private func load() async {
        struct Response: Decodable { let sessions: [SessionSummary]? }
        do {
            let r = try await APIClient.shared.get("/api/sessions", as: Response.self)
            let all = r.sessions ?? []
            withAnimation(Motion.spring) {
                sessions = all
                    .filter { ($0.sessionType ?? "user") == "user" }
                    .filter { ($0.messageCount ?? 0) > 0 }
                    .sorted { ($0.updatedAt ?? "") > ($1.updatedAt ?? "") }
                    .prefix(50)
                    .map { $0 }
                loaded = true
                error = nil
            }
        } catch {
            self.error = "Couldn't load your conversations."
            loaded = true
        }
    }
}
