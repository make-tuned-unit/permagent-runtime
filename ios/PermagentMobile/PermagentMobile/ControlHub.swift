// Control — direct control of the hub from the phone, no chat required (the
// companion model: chat drives the hub too, these screens make the same
// capabilities glanceable + tappable). This is the 5th tab; it owns a single
// NavigationStack and pushes the three control surfaces (Agents at work,
// Automations, Notes) as destinations. Those destination views therefore MUST
// NOT wrap themselves in their own NavigationStack — they render into this one.

import Foundation
import SwiftUI

struct ControlHubView: View {
    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 14) {
                    Text("Direct control of your hub — glance and act, no chat required. Everything here runs on your Mac and shows live on your desktop.")
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.muted)
                        .frame(maxWidth: .infinity, alignment: .leading)

                    hubLink(destination: AgentsView(),
                            icon: "person.2.fill",
                            title: "Agents at work",
                            subtitle: "Your background agents + what's running now",
                            accent: ChatSurface.spark)

                    hubLink(destination: SchedulesView(),
                            icon: "clock.arrow.circlepath",
                            title: "Automations",
                            subtitle: "Scheduled jobs — run now, pause, or stop",
                            accent: Brand.violet)

                    hubLink(destination: ModelPickerView(),
                            icon: "cpu",
                            title: "Model",
                            subtitle: "Switch the chat model — any provider with a saved key",
                            accent: ChatSurface.spark)

                }
                .padding()
            }
            .background(ChatSurface.bg.ignoresSafeArea())
            .toolbar(.hidden, for: .navigationBar)
            .safeAreaInset(edge: .top, spacing: 0) { pageHeader }
        }
    }

    /// In-page title — the tab hides the system bar in favor of this.
    private var pageHeader: some View {
        HStack {
            Text("Control")
                .font(.brandTitle)
                .foregroundStyle(ChatSurface.text)
            Spacer()
        }
        .padding(.horizontal, 18)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .background(ChatSurface.bg)
    }

    private func hubLink<D: View>(
        destination: D, icon: String, title: String, subtitle: String, accent: Color
    ) -> some View {
        NavigationLink {
            destination
        } label: {
            RaisedCard {
                HStack(spacing: 14) {
                    Image(systemName: icon)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(accent)
                        .frame(width: 36, height: 36)
                        .background(ChatSurface.control, in: Circle())
                    VStack(alignment: .leading, spacing: 3) {
                        Text(title)
                            .font(.brandHeadline)
                            .foregroundStyle(ChatSurface.text)
                        Text(subtitle)
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.muted)
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundStyle(ChatSurface.dim)
                }
            }
        }
        .buttonStyle(.plain)
    }
}

// ── Shared control-surface atoms ─────────────────────────────────────────────

/// A living status dot (the Home health-dot idiom, reusable). Pulses cyan/live
/// unless Reduce Motion is on, in which case it holds steady.
struct PulseDot: View {
    var color: Color
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var on = false

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 8, height: 8)
            .shadow(color: color.opacity(0.7), radius: on ? 6 : 2)
            .opacity(reduceMotion ? 1 : (on ? 1 : 0.5))
            .animation(
                reduceMotion ? nil : .easeInOut(duration: 0.9).repeatForever(autoreverses: true),
                value: on
            )
            .onAppear { on = true }
    }
}

/// Error + retry card — the honest failure state shared by the control screens
/// (mirrors the desktop's "couldn't load, try again" rather than a blank view).
struct HubErrorCard: View {
    let text: String
    let retry: () async -> Void

    var body: some View {
        RaisedCard {
            VStack(alignment: .leading, spacing: 10) {
                Text(text)
                    .font(.brandCaption)
                    .foregroundStyle(Brand.danger)
                Button {
                    Task { await retry() }
                } label: {
                    Text("Retry")
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                        .background(ChatSurface.control)
                        .foregroundStyle(ChatSurface.text)
                        .clipShape(Capsule())
                }
                .buttonStyle(.plain)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Best-effort relative time ("2h ago") from the hub's date strings. Tolerant of
/// the two shapes the API emits: RFC3339 (chrono `DateTime<Utc>`, with or without
/// fractional seconds) and SQLite `CURRENT_TIMESTAMP` ("yyyy-MM-dd HH:mm:ss", UTC).
/// Returns the raw string if it can't be parsed, and nil for empty/absent input —
/// never a wrong-looking time.
enum RelativeTime {
    static func string(from raw: String?) -> String? {
        guard let raw, !raw.isEmpty else { return nil }
        guard let date = parse(raw) else { return raw }
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .abbreviated
        return f.localizedString(for: date, relativeTo: Date())
    }

    private static func parse(_ s: String) -> Date? {
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = iso.date(from: s) { return d }
        iso.formatOptions = [.withInternetDateTime]
        if let d = iso.date(from: s) { return d }
        let df = DateFormatter()
        df.locale = Locale(identifier: "en_US_POSIX")
        df.timeZone = TimeZone(identifier: "UTC")
        df.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return df.date(from: s)
    }
}
