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
                            icon: "slider.horizontal.3",
                            title: "Model",
                            subtitle: "Switch the chat model — any provider with a saved key",
                            accent: ChatSurface.spark)

                    hubLink(destination: FeaturesView(),
                            icon: "switch.2",
                            title: "Features",
                            subtitle: "Initiative, Playbook, Concierge, Steward, Guard — same switches as the Mac",
                            accent: Brand.violet)

                    hubLink(destination: PronunciationView(),
                            icon: "mouth.fill",
                            title: "Pronunciation",
                            subtitle: "Teach the voice a word once — remembered on every device",
                            accent: ChatSurface.spark)

                    hubLink(destination: VoiceIdentityView(),
                            icon: DesignPolicy.voiceIdentitySymbol,
                            title: "Voice identity",
                            subtitle: "Set, redo, or forget whose voice your agent answers",
                            accent: Brand.violet)

                }
                .padding()
            }
            .background { AppBackdrop() }
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
        .padding(.horizontal, DesignPolicy.pageHeaderHorizontalPadding)
        .padding(.top, DesignPolicy.pageHeaderTopPadding)
        .padding(.bottom, DesignPolicy.pageHeaderBottomPadding)
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

// ── Voice identity ──────────────────────────────────────────────────────────

private struct SpeakerIdentityStatus: Decodable {
    let models_present: Bool
    let verifier_loaded: Bool
    let enrolled: Bool
    let downloading: Bool
}

/// Dedicated identity setup/management. Enrollment deliberately has no Orb:
/// the Orb is the live conversation surface, while this is a one-time setup
/// flow and an explicit Control thereafter.
struct VoiceIdentityView: View {
    var onboarding = false
    var onFinished: () -> Void = {}

    @StateObject private var engine = VoiceEngine()
    @State private var preparing = true
    @State private var preparationError: String?

    var body: some View {
        ZStack {
            ChatSurface.bg.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    header
                    Text("Your voice stays on your hub as a learned identity embedding. Enrollment audio is never saved.")
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.muted)

                    RaisedCard { content }

                    if onboarding, !engine.printEnrolled {
                        Button("Skip for now") {
                            engine.skipEnroll()
                            onFinished()
                        }
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(ChatSurface.muted)
                        .frame(maxWidth: .infinity)
                    }
                }
                .padding(20)
            }
        }
        .task { await prepare() }
        .onDisappear { engine.stop() }
        .onChange(of: engine.printEnrolled) { wasEnrolled, enrolled in
            guard onboarding, !wasEnrolled, enrolled else { return }
            Task {
                try? await Task.sleep(for: .milliseconds(700))
                onFinished()
            }
        }
        .toolbar(.hidden, for: .navigationBar)
    }

    private var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 3) {
                Text(onboarding ? "ONE-TIME SETUP" : "CONTROL")
                    .font(.brandLabel)
                    .foregroundStyle(ChatSurface.spark)
                Text("Voice identity")
                    .font(.brandTitle)
                    .foregroundStyle(ChatSurface.text)
            }
            Spacer()
            if onboarding {
                Button(action: onFinished) {
                    Image(systemName: "xmark")
                        .foregroundStyle(ChatSurface.text)
                        .frame(width: 38, height: 38)
                }
                .glassChrome(in: Circle(), interactive: true)
                .accessibilityLabel("Skip voice identity setup")
            }
        }
    }

    @ViewBuilder
    private var content: some View {
        if preparing || engine.identityModelDownloading {
            HStack(spacing: 12) {
                ProgressView()
                VStack(alignment: .leading, spacing: 3) {
                    Text("Preparing private voice identity")
                        .font(.brandHeadline)
                        .foregroundStyle(ChatSurface.text)
                    Text("Downloading and loading the verified speaker model…")
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.muted)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else if let preparationError {
            VStack(alignment: .leading, spacing: 12) {
                Text("Voice identity isn't ready")
                    .font(.brandHeadline)
                    .foregroundStyle(Brand.danger)
                Text(preparationError)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                Button("Try again") { Task { await prepare() } }
                    .buttonStyle(.borderedProminent)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else if engine.enrolling, let prompt = engine.enrollPrompt {
            VStack(alignment: .leading, spacing: 16) {
                Text("Sentence \(min(engine.enrollHave + 1, engine.enrollNeed)) of \(engine.enrollNeed)")
                    .font(.brandLabel)
                    .foregroundStyle(ChatSurface.spark)
                Text(prompt)
                    .font(.brandTitle)
                    .foregroundStyle(ChatSurface.text)
                    .fixedSize(horizontal: false, vertical: true)
                Label("Say the sentence naturally. Recording stops after you pause.", systemImage: "mic.fill")
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                Button("Cancel setup") { engine.skipEnroll() }
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(ChatSurface.muted)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else if engine.printEnrolled {
            VStack(alignment: .leading, spacing: 14) {
                Label("Your voice is protected", systemImage: "checkmark.shield.fill")
                    .font(.brandHeadline)
                    .foregroundStyle(ChatSurface.spark)
                Text("Your agent uses learned speaker verification to reject other talkers before transcription.")
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                HStack(spacing: 12) {
                    Button("Redo my voice") { engine.beginEnroll() }
                        .buttonStyle(.borderedProminent)
                    Button("Forget my voice", role: .destructive) { engine.clearEnroll() }
                        .buttonStyle(.bordered)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            VStack(alignment: .leading, spacing: 14) {
                Label("Only answer your voice", systemImage: "waveform.badge.person.crop")
                    .font(.brandHeadline)
                    .foregroundStyle(ChatSurface.text)
                Text("You'll say three short sentences. This runs once during setup; you can change it later in Control.")
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                Button("Set my voice") { engine.beginEnroll() }
                    .buttonStyle(.borderedProminent)
                    .disabled(!engine.identityModelAvailable)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }

        if let notice = engine.notice {
            Text(notice)
                .font(.brandCaption)
                .foregroundStyle(Brand.warning)
                .padding(.top, 8)
        }
        if case .failed(let reason) = engine.state {
            Text(reason)
                .font(.brandCaption)
                .foregroundStyle(Brand.danger)
                .padding(.top, 8)
        }
    }

    @MainActor
    private func prepare() async {
        preparing = true
        preparationError = nil
        do {
            var status = try await APIClient.shared.get(
                "/voice/speaker/models", as: SpeakerIdentityStatus.self
            )
            if status.enrolled, onboarding {
                preparing = false
                onFinished()
                return
            }
            if !status.verifier_loaded {
                try await APIClient.shared.send("/voice/speaker/models/download")
                for _ in 0..<120 {
                    try await Task.sleep(for: .seconds(1))
                    status = try await APIClient.shared.get(
                        "/voice/speaker/models", as: SpeakerIdentityStatus.self
                    )
                    if status.verifier_loaded { break }
                }
            }
            guard status.verifier_loaded else {
                throw APIError.daemon("The speaker model did not finish loading.")
            }
            preparing = false
            // This is a setup/control screen, not a conversation surface. Do
            // not let ambient speech open a normal agent turn before the user
            // explicitly begins an enrollment take.
            engine.handsFree = false
            await engine.start(sessionId: nil)
        } catch {
            preparing = false
            preparationError = String(describing: error)
        }
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
