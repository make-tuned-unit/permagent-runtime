// Permagent brand system for iOS — mirrors ui/command-center/src/styles/tokens.ts.
// Neon cyan → violet over a Deep Void dark or a Pearl White light surface,
// glass materials. Keep in lockstep with the web tokens; this file is the
// single source of brand truth on iOS.
//
// APPEARANCE FOLLOWS THE SYSTEM, like the desktop app. The web has a `system`
// theme preference that resolves to `dark` or `silver` from
// `prefers-color-scheme` and re-resolves live when the OS flips (tokens.ts
// `_resolve` / the matchMedia listener). iOS was pinned to the Deep Void
// palette with `.preferredColorScheme(.dark)`, so a phone in Light Mode showed
// a different product from the Mac beside it. Every token below is now a
// DYNAMIC color: it resolves against the trait collection at draw time, so the
// switch is free at every call site and live when the user changes Settings —
// no view has to observe anything, and no `Brand.x` usage had to change.

import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

enum Brand {
    // Palette. Dark values are tokens.ts DARK_COLORS; light values are
    // SILVER_COLORS — the SAME two palettes the desktop resolves between, so
    // the clients cannot drift apart in either appearance. (They already had
    // once: deepVoid was 0x0A0E1A vs the web's bg #0B1220.)
    static let deepVoid = Color.brand(dark: 0x0B1220, light: 0xF8FAFC)   // color.bg
    static let bgDeeper = Color.brand(dark: 0x070B14, light: 0xEEF2F7)   // color.bgDeeper
    static let surface = Color.brand(dark: 0x1E2433, darkAlpha: 0.78,    // color.surface
                                     light: 0xFFFFFF, lightAlpha: 0.92)
    static let surfaceHi = Color.brand(dark: 0x262D3F, light: 0xF8FAFC)  // color.surfaceHi
    static let cyan = Color.brand(dark: 0x00D5FF, light: 0x00BFEF)
    static let cyanGlow = Color.brand(dark: 0x00D5FF, darkAlpha: 0.45,
                                      light: 0x00BFEF, lightAlpha: 0.25)
    static let violet = Color.brand(dark: 0x8D44AE, light: 0x8B5CFF)
    static let text = Color.brand(dark: 0xFFFFFF, light: 0x1E2530)
    static let textMuted = Color.brand(dark: 0x8A94A6, light: 0x4B5563)
    static let textDim = Color.brand(dark: 0x5A6478, light: 0x6B7585)
    static let danger = Color.brand(dark: 0xFFB4A2, light: 0xDC2626)
    static let success = Color.brand(dark: 0x34D399, light: 0x059669)
    static let warning = Color.brand(dark: 0xFBBF24, light: 0xD97706)
    static let border = Color.brand(dark: 0xFFFFFF, darkAlpha: 0.07,
                                    light: 0xA7B0BE, lightAlpha: 0.35)
    static let borderHi = Color.brand(dark: 0x00D5FF, darkAlpha: 0.18,   // color.borderHi
                                      light: 0x00BFEF, lightAlpha: 0.40)
    static let cyanSoft = Color.brand(dark: 0x00D5FF, darkAlpha: 0.14,   // color.cyanSoft
                                      light: 0x00BFEF, lightAlpha: 0.10)
    static let purpleSoft = Color.brand(dark: 0x8D44AE, darkAlpha: 0.18, // color.purpleSoft
                                        light: 0x8B5CFF, lightAlpha: 0.10)

    /// Ink for text and glyphs sitting ON an accent fill — cyan, the ribbon, a
    /// danger chip. tokens.ts `textOnCyan`, and deliberately NOT adaptive: the
    /// fill under it is bright in both appearances, so the label must stay dark
    /// in both. This used to be `deepVoid`, which was correct only while
    /// `deepVoid` was always dark; the moment the page background followed the
    /// system, every one of those labels would have turned near-white on cyan.
    static let onAccent = Color(hex: 0x04141B)

    /// Accent used as INK — an eyebrow label, a glyph, a checkmark — rather
    /// than as a fill. Neon cyan is right on the void and unreadable on Pearl
    /// White (#00BFEF is ~2:1 there), so light resolves to the deeper Sky-700
    /// the desktop silver theme already uses for accent text (SILVER_COLORS
    /// `codeText`). `cyan` stays the FILL colour in both appearances — the
    /// ribbon, chips and buttons must keep their saturation.
    static let cyanInk = Color.brand(dark: 0x00D5FF, light: 0x0369A1)

    /// Ink on a DANGER fill, which is the one accent that inverts between the
    /// palettes: dark's danger is a pale salmon (#FFB4A2) wanting dark ink,
    /// light's is a saturated red (#DC2626) wanting white. `onAccent` is
    /// correct for cyan and the ribbon in both appearances; it is not correct
    /// here, and near-black on #DC2626 fails contrast outright.
    static let onDanger = Color.brand(dark: 0x04141B, light: 0xFFFFFF)

    /// Shadow under floating glass. Black at 45% is right over the void and far
    /// too heavy over Pearl White, where elevation is a soft graphite haze
    /// (tokens.ts SILVER_COLORS.elevationOverlay).
    static let cardShadow = Color.brand(dark: 0x000000, darkAlpha: 0.45,
                                        light: 0x1E2530, lightAlpha: 0.13)

    /// The signature ribbon: cyan → indigo → violet (tokens.ts ribbonGradient).
    /// Light weights the middle stop toward blue, as SILVER_COLORS does, so
    /// text over it still clears AA.
    static let ribbon = LinearGradient(
        colors: [
            Color.brand(dark: 0x00D5FF, light: 0x00BFEF),
            Color.brand(dark: 0x6366F1, light: 0x3A7BFF),
            Color.brand(dark: 0x8D44AE, light: 0x8B5CFF),
        ],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )

    /// Shell background: a radial cyan breath over the page colour (WizardShell).
    static let shell = RadialGradient(
        colors: [
            Color.brand(dark: 0x00D5FF, darkAlpha: 0.06, light: 0x00BFEF, lightAlpha: 0.07),
            deepVoid,
        ],
        center: .init(x: 0.5, y: 0.4),
        startRadius: 40,
        endRadius: 520
    )
}

#if canImport(UIKit)
extension UIColor {
    /// Straight from components — no SwiftUI types involved, so this is safe to
    /// call from a dynamic-provider block on any thread. See `Color.brand`.
    convenience init(rgbHex hex: UInt32, alpha: Double) {
        self.init(
            red: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: CGFloat(alpha)
        )
    }
}
#endif

extension Color {
    init(hex: UInt32, alpha: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: alpha
        )
    }

    /// A token that resolves from the system appearance at draw time.
    ///
    /// The dynamic `UIColor` provider is what makes this free: SwiftUI resolves
    /// it against the current trait collection on every render, so a live
    /// Light/Dark switch repaints without any view observing anything. A
    /// `@Environment(\.colorScheme)` approach would instead force every call
    /// site to become a computed property inside a `View`.
    ///
    /// The provider body MUST stay pure arithmetic over UIKit types. It first
    /// read `UIColor(Color(hex:))`, which crosses the SwiftUI→UIKit bridge —
    /// main-thread-affine, and it allocates a `Color` per invocation. The
    /// provider is invoked at COLOR-RESOLVE time, which for an animating or
    /// `Canvas`-drawn subtree is the render path, not necessarily the main
    /// thread. The voice screen is the only place with a `TimelineView`
    /// redrawing every frame, and it is the screen that crashed a second in
    /// (reported 2026-08-04). Building the UIColor straight from components
    /// removes the bridge, the allocation, and the thread affinity together.
    static func brand(dark: UInt32, darkAlpha: Double = 1,
                      light: UInt32, lightAlpha: Double = 1) -> Color {
        #if canImport(UIKit)
        return Color(UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(rgbHex: dark, alpha: darkAlpha)
                : UIColor(rgbHex: light, alpha: lightAlpha)
        })
        #else
        return Color(hex: dark, alpha: darkAlpha)
        #endif
    }
}

// ── Typography ───────────────────────────────────────────────────────────────
// One ramp, mirroring `type` in tokens.ts role for role and px for pt. The
// scale previously used Dynamic Type styles (.subheadline, .caption2), which
// meant every size differed from the desktop app — `brandBody` rendered at 15pt
// against the web's 14, `brandDisplay` at 30 against 32. Brand alignment across
// the two clients is the reason these are now fixed sizes.
//
// TRADE-OFF, deliberate: fixed sizes do not scale with the user's Dynamic Type
// setting. That is the cost of matching the desktop ramp exactly. If
// accessibility scaling is wanted back, wrap call sites in @ScaledMetric rather
// than reverting these to text styles — that keeps the ratios while restoring
// scaling, whereas text styles reintroduce the drift.
//
// Typeface stays SF Pro: the web's Manrope/Inter are not bundled here, and a
// downloaded webfont on iOS would cost launch time and a licence review. SF at
// the web's metrics is the closer match of the two available options.
extension Font {
    /// tokens.ts type.display — 32/600. Hero numerals, stat tiles.
    static let brandDisplay = Font.system(size: 32, weight: .semibold)
    /// tokens.ts type.title — 20/600. Section / screen titles.
    static let brandTitle = Font.system(size: 20, weight: .semibold)
    /// tokens.ts type.heading — 16/600. Card headlines, primary rows.
    static let brandHeading = Font.system(size: 16, weight: .semibold)
    /// tokens.ts type.heading — retained name for existing call sites.
    static let brandHeadline = Font.system(size: 16, weight: .semibold)
    /// tokens.ts type.body — 14/400.
    static let brandBody = Font.system(size: 14, weight: .regular)
    /// tokens.ts type.small — 13/400.
    static let brandSmall = Font.system(size: 13, weight: .regular)
    /// tokens.ts type.caption — 12/400.
    static let brandCaption = Font.system(size: 12, weight: .regular)
    /// tokens.ts type.micro — 11/500.
    static let brandMicro = Font.system(size: 11, weight: .medium)
    /// tokens.ts type.label — 11/600, 0.08em tracking, UPPERCASE. The tracking
    /// and casing are not carried by the font: apply `.tracking(0.88)` and
    /// `.textCase(.uppercase)` at the call site, as the web token does.
    static let brandLabel = Font.system(size: 11, weight: .semibold)
}

/// Tabular figures — the iOS mirror of `tabularNums` in tokens.ts. Digits stop
/// reflowing as counts and timers change.
extension View {
    func tabularNums() -> some View { self.monospacedDigit() }
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
///
/// It USED to branch on iOS 26 and apply `.buttonStyle(.glassProminent)` with
/// `.tint(Brand.cyan)` on top of the ribbon. That fills the button twice — a
/// bright system-tinted glass slab with the brand gradient washed over it — so
/// it read as a stock blue button rather than a Permagent one, and it looked
/// like a different app from the same button on iOS 25. Glass belongs on
/// CHROME (the voice close button, the hands-free pill), where the material can
/// sit over content; a primary action wants the brand ribbon, flat and
/// unambiguous, identical on every OS version.
///
/// The corner is `radius.lg` from tokens.ts (14), continuous rather than
/// circular, matching the desktop's primary buttons. The old 12 was applied via
/// `buttonBorderShape`, which the glass style rounded differently again.
struct PrimaryCTA: View {
    let title: String
    var systemImage: String? = nil
    var enabled: Bool = true
    let action: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pressed = false

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: 14, style: .continuous)
    }

    private var label: some View {
        HStack(spacing: 7) {
            if let systemImage { Image(systemName: systemImage).font(.subheadline.weight(.semibold)) }
            Text(title).font(.body.weight(.semibold))
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 15)
        .foregroundStyle(Brand.onAccent.opacity(enabled ? 1 : 0.55))
    }

    var body: some View {
        Button(action: action) {
            label
                .background(Brand.ribbon.opacity(enabled ? 1 : 0.3), in: shape)
                // A hairline of the accent lifts the fill off a dark background
                // without the hard bright border the glass style was drawing.
                .overlay(shape.strokeBorder(Color.white.opacity(enabled ? 0.16 : 0.06), lineWidth: 0.5))
                .shadow(color: Brand.cyanGlow.opacity(enabled ? 0.35 : 0), radius: 14, y: 5)
                .scaleEffect(pressed && enabled && !reduceMotion ? 0.975 : 1)
                .animation(Motion.ease, value: pressed)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .animation(Motion.ease, value: enabled)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in pressed = true }
                .onEnded { _ in pressed = false }
        )
        .accessibilityAddTraits(.isButton)
    }
}

/// The house glass card (Glass atom): blur + hairline + soft glow. On iOS 26+
/// the blur is the real Liquid Glass material (tinted to the brand surface);
/// earlier systems keep the ultraThinMaterial look unchanged.
struct GlassCard<Content: View>: View {
    var content: () -> Content
    private var shape: RoundedRectangle { RoundedRectangle(cornerRadius: 16, style: .continuous) }
    var body: some View {
        Group {
            if #available(iOS 26.0, *) {
                content()
                    .padding(16)
                    .glassEffect(.regular.tint(Brand.surface), in: shape)
            } else {
                content()
                    .padding(16)
                    .background(.ultraThinMaterial.opacity(0.6))
                    .background(Brand.surface)
                    .clipShape(shape)
            }
        }
        .overlay(shape.strokeBorder(Brand.borderHi, lineWidth: 1))
        .shadow(color: Brand.cardShadow, radius: 24, y: 12)
    }
}

extension View {
    /// Brand chrome material for floating controls (VoiceView close button,
    /// hands-free pill): real Liquid Glass on iOS 26+ — interactive glass
    /// responds to touch with the native morph — ultraThinMaterial before.
    @ViewBuilder
    func glassChrome<S: Shape>(in shape: S, interactive: Bool = false) -> some View {
        if #available(iOS 26.0, *) {
            self.glassEffect(interactive ? Glass.regular.interactive() : .regular, in: shape)
        } else {
            self.background(.ultraThinMaterial, in: shape)
        }
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
        .accessibilityLabel("\(AgentIdentity.shared.nameCapitalized) is thinking")
    }
}

// ── Liquid Glass tab bar ──────────────────────────────────────────────────────

extension View {
    /// Minimize the tab bar as the user scrolls the page down — and expand it
    /// again on scroll up — the modern Instagram / Liquid Glass behavior. Uses
    /// the native iOS 26 `tabBarMinimizeBehavior`; no-ops gracefully on earlier
    /// iOS so the standard tab bar is untouched. Requires the selected tab to
    /// host scrollable content (ScrollView/List) for the scroll to drive it.
    @ViewBuilder
    func liquidGlassTabMinimize() -> some View {
        if #available(iOS 26.0, *) {
            self.tabBarMinimizeBehavior(.onScrollDown)
        } else {
            self
        }
    }
}
