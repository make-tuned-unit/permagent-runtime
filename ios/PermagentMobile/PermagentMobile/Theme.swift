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
    static let border = Color.white.opacity(0.07)
    static let borderHi = Color(hex: 0x00D5FF).opacity(0.16)

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
