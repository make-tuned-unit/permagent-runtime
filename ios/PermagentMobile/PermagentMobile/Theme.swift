// Permagent brand system for iOS — mirrors ui/command-center/src/styles/tokens.ts.
// Deep-void dark, neon cyan → violet, glass materials. Keep in lockstep with
// the web tokens; this file is the single source of brand truth on iOS.

import SwiftUI

enum Brand {
    // Palette (tokens.ts)
    static let deepVoid = Color(hex: 0x0A0E1A)
    static let surface = Color(hex: 0x141C30).opacity(0.78)
    static let cyan = Color(hex: 0x00D5FF)
    static let cyanGlow = Color(hex: 0x00D5FF).opacity(0.45)
    static let violet = Color(hex: 0x8D44AE)
    static let text = Color.white
    static let textMuted = Color(hex: 0x8A94A6)
    static let textDim = Color(hex: 0x5A6478)
    static let danger = Color(hex: 0xFFB4A2)
    static let success = Color(hex: 0x34D399)          // tokens.ts DARK_COLORS.success
    static let warning = Color(hex: 0xFBBF24)          // tokens.ts DARK_COLORS.warning
    static let border = Color.white.opacity(0.07)
    static let borderHi = Color(hex: 0x00D5FF).opacity(0.18)   // tokens.ts color.borderHi
    static let cyanSoft = Color(hex: 0x00D5FF).opacity(0.14)   // tokens.ts color.cyanSoft
    static let purpleSoft = Color(hex: 0x8D44AE).opacity(0.18) // tokens.ts color.purpleSoft

    /// The signature ribbon: cyan → indigo → violet (tokens.ts ribbonGradient).
    static let ribbon = LinearGradient(
        colors: [Color(hex: 0x00D5FF), Color(hex: 0x6366F1), Color(hex: 0x8D44AE)],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )

    /// Shell background: radial cyan breath over the void (WizardShell).
    static let shell = RadialGradient(
        colors: [Color(hex: 0x00D5FF).opacity(0.06), deepVoid],
        center: .init(x: 0.5, y: 0.4),
        startRadius: 40,
        endRadius: 520
    )
}

extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: 1
        )
    }
}

// ── Typography (one scale, SF Pro; SF Mono for technical accents) ────────────
// The web uses Inter/mono/display; on iOS the premium-native choice is SF Pro,
// which also gives us Dynamic Type for free. Numbers use tabular figures so
// stat tiles and counts don't jitter.
extension Font {
    /// Big hero numerals (stat tiles). Rounded + tabular so digits don't shift.
    static let brandDisplay = Font.system(size: 30, weight: .bold, design: .rounded)
    /// Section / screen titles.
    static let brandTitle = Font.system(.title3, design: .default).weight(.semibold)
    /// Card headlines, primary rows.
    static let brandHeadline = Font.system(.subheadline).weight(.semibold)
    /// Body copy.
    static let brandBody = Font.system(.subheadline)
    /// Secondary / supporting copy.
    static let brandCaption = Font.system(.caption)
    /// The house mono label — uppercase kicker over cards (kind badges, headers).
    static let brandLabel = Font.system(.caption2, design: .monospaced).weight(.semibold)
}

// ── Motion tokens ────────────────────────────────────────────────────────────
enum Motion {
    /// The house ease — matches tokens.ts `ease.out` feel for view transitions.
    static let ease = Animation.easeOut(duration: 0.22)
    /// Springy, for content arrival (message bubbles, card removal).
    static let spring = Animation.spring(response: 0.34, dampingFraction: 0.82)
    /// Slow inhale/exhale — live-state pulses (the recording ring). Pair with
    /// `.repeatForever(autoreverses: true)` at the call site.
    static let breath = Animation.easeInOut(duration: 1.1)
}

/// The house primary CTA: ribbon-gradient fill, deep-void label, continuous
/// radius. One shape for every "do the thing" button (pairing Connect, note
/// Save) so CTAs read identically across screens.
struct PrimaryCTA: View {
    let title: String
    var systemImage: String? = nil
    var enabled: Bool = true
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 7) {
                if let systemImage { Image(systemName: systemImage).font(.subheadline.weight(.semibold)) }
                Text(title).font(.body.weight(.semibold))
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 14)
            .background(Brand.ribbon.opacity(enabled ? 1 : 0.35))
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .foregroundStyle(Brand.deepVoid.opacity(enabled ? 1 : 0.7))
        }
        .disabled(!enabled)
        .animation(Motion.ease, value: enabled)
    }
}

/// The house glass card (Glass atom): blur + hairline + soft glow.
struct GlassCard<Content: View>: View {
    var content: () -> Content
    var body: some View {
        content()
            .padding(16)
            .background(.ultraThinMaterial.opacity(0.6))
            .background(Brand.surface)
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(Brand.borderHi, lineWidth: 1)
            )
            .shadow(color: .black.opacity(0.45), radius: 24, y: 12)
    }
}

// ── Agent presence: the "Henry is thinking" indicator ────────────────────────
// Premium agentic UX masks latency with a living cue, not a dead spinner. Three
// dots breathe in sequence while we await the first streamed token. Honors
// Reduce Motion (falls back to a static row).
struct ThinkingDots: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var phase = false

    var body: some View {
        HStack(spacing: 5) {
            ForEach(0..<3, id: \.self) { i in
                Circle()
                    .fill(Brand.cyan)
                    .frame(width: 6, height: 6)
                    .opacity(reduceMotion ? 0.6 : (phase ? 1 : 0.28))
                    .animation(
                        reduceMotion ? nil :
                            .easeInOut(duration: 0.6).repeatForever().delay(Double(i) * 0.18),
                        value: phase
                    )
            }
        }
        .onAppear { phase = true }
        .accessibilityLabel("Henry is thinking")
    }
}
