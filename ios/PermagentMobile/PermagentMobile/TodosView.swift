// To-dos — every dated, unfinished card across every project, soonest first
// (the desktop dashboard's due list, in your pocket). Read comes from
// GET /api/cards/due; swipe-to-dismiss mirrors the dashboard's dismiss-due
// (hides the card from due lists everywhere — it does not complete the card).

import SwiftUI

struct DueCard: Decodable, Identifiable, Equatable {
    let id: String
    let title: String
    let projectId: String
    let projectName: String
    let columnName: String
    /// ISO date, `YYYY-MM-DD`.
    let dueDate: String
    let assignedTo: String?
}

enum DueBucket: String, CaseIterable {
    case overdue = "OVERDUE"
    case today = "TODAY"
    case week = "THIS WEEK"
    case later = "LATER"

    var accent: Color {
        switch self {
        case .overdue: return Brand.danger
        case .today: return Brand.cyan
        case .week: return Brand.violet
        case .later: return Brand.textDim
        }
    }
}

/// Bucket a `YYYY-MM-DD` due date against the device's calendar. Unparseable
/// dates land in `.later` — never dropped, never guessed.
func dueBucket(_ isoDay: String, now: Date = Date()) -> DueBucket {
    let df = DateFormatter()
    df.locale = Locale(identifier: "en_US_POSIX")
    df.dateFormat = "yyyy-MM-dd"
    df.timeZone = .current
    guard let due = df.date(from: isoDay) else { return .later }
    let cal = Calendar.current
    if cal.isDateInToday(due) { return .today }
    if due < now { return .overdue }
    if let weekOut = cal.date(byAdding: .day, value: 7, to: now), due <= weekOut {
        return .week
    }
    return .later
}

/// Short human label for a due day: "Mon 12", or "Today"/"Tomorrow".
func dueLabel(_ isoDay: String) -> String {
    let df = DateFormatter()
    df.locale = Locale(identifier: "en_US_POSIX")
    df.dateFormat = "yyyy-MM-dd"
    df.timeZone = .current
    guard let due = df.date(from: isoDay) else { return isoDay }
    if Calendar.current.isDateInToday(due) { return "Today" }
    if Calendar.current.isDateInTomorrow(due) { return "Tomorrow" }
    let out = DateFormatter()
    out.dateFormat = "EEE d"
    return out.string(from: due)
}

struct TodosView: View {
    @State private var items: [DueCard] = []
    @State private var loaded = false
    @State private var error: String?
    @State private var dismissedCount = 0

    private var grouped: [(DueBucket, [DueCard])] {
        let byBucket = Dictionary(grouping: items) { dueBucket($0.dueDate) }
        return DueBucket.allCases.compactMap { b in
            guard let rows = byBucket[b], !rows.isEmpty else { return nil }
            return (b, rows)
        }
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                if let error {
                    HubErrorCard(text: error) { await load() }
                } else if loaded && items.isEmpty {
                    SparkEmptyState(
                        line: "Nothing due.",
                        caption: "Clear runway. Dated cards from any project board land here the moment they exist."
                    )
                    .padding(.top, 60)
                } else {
                    ForEach(grouped, id: \.0) { bucket, rows in
                        RaisedCard {
                            VStack(alignment: .leading, spacing: 10) {
                                HStack(spacing: 6) {
                                    Text(bucket.rawValue)
                                        .font(.brandLabel).tracking(0.88)
                                        .foregroundStyle(bucket.accent)
                                    Text("\(rows.count)")
                                        .font(.system(.caption2, design: .monospaced))
                                        .foregroundStyle(ChatSurface.dim)
                                }
                                ForEach(rows) { row in
                                    todoRow(row, accent: bucket.accent)
                                }
                            }
                        }
                    }
                }
            }
            .padding()
        }
        .background(ChatSurface.bg.ignoresSafeArea())
        .navigationTitle("To-dos")
        .refreshable { await load() }
        .task { await load() }
        // Tactile: each dismissal lands with a light tap.
        .sensoryFeedback(.impact(weight: .light), trigger: dismissedCount)
    }

    private func todoRow(_ row: DueCard, accent: Color) -> some View {
        HStack(alignment: .top, spacing: 10) {
            // Dismiss — the due-list's own verb (hides from every due surface;
            // completing the card stays a board action).
            Button {
                Task { await dismiss(row) }
            } label: {
                Image(systemName: "circle")
                    .font(.body)
                    .foregroundStyle(ChatSurface.spark)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Dismiss \(row.title) from to-dos")

            VStack(alignment: .leading, spacing: 2) {
                Text(row.title)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.text)
                    .lineLimit(2)
                HStack(spacing: 6) {
                    Text(row.projectName)
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(ChatSurface.dim)
                        .lineLimit(1)
                    Text(dueLabel(row.dueDate))
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(accent)
                }
            }
            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
    }

    private func dismiss(_ row: DueCard) async {
        // Optimistic: the row leaves with a spring; a failed POST puts it back.
        let snapshot = items
        withAnimation(Motion.spring) { items.removeAll { $0.id == row.id } }
        dismissedCount += 1
        struct Card: Decodable { let id: String }
        do {
            _ = try await APIClient.shared.post(
                "/api/projects/\(row.projectId)/cards/\(row.id)/dismiss-due",
                body: ["dismissed": true],
                as: Card.self
            )
        } catch {
            withAnimation(Motion.spring) { items = snapshot }
        }
    }

    private func load() async {
        do {
            let due = try await APIClient.shared.get("/api/cards/due", as: [DueCard].self)
            withAnimation(Motion.spring) {
                items = due
                loaded = true
                error = nil
            }
        } catch {
            if !loaded { self.error = "Couldn't load your to-dos." }
        }
    }
}
