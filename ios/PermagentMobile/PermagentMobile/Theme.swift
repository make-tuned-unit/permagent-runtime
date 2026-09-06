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
    /// The Mobius magenta — the second brand accent, tokens.ts `purple` /
    /// `purpleBright`. The strip runs cyan into magenta, so anything that
    /// stands in FOR the strip (its glow, the spark, the primary action) reads
    /// as brand only when both are present. Used as a partner to cyan, never
    /// as a replacement: cyan stays the interactive colour.
    static let purple = Color.brand(dark: 0x8D44AE, light: 0x7B3A99)
    static let purpleBright = Color.brand(dark: 0xA855CC, light: 0x9147B8)

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
        #if os(iOS)
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

// ── Chat surface palette ─────────────────────────────────────────────────────
// The conversation screen borrows the ERGONOMICS of the familiar chat apps —
// prose on the page, one composer card, controls where thumbs expect them —
// but stays in Permagent's own palette (ruled 2026-08-06: "keep permagent
// colors… uniquely permagent but with a design UI people feel comfortable
// navigating"). Every value below is a Brand token or a solid derivative of
// one; nothing here introduces a second brand.
enum ChatSurface {
    /// The page — the brand void / pearl.
    static let bg = Brand.deepVoid
    /// Composer card and user-message bubble — one step off the page. Solid
    /// (not the translucent Brand.surface): the card must occlude transcript
    /// text scrolling beneath it.
    static let raised = Color.brand(dark: 0x1E2433, light: 0xFFFFFF)
    /// Controls that sit on the raised card (the agent pill, the plus button).
    static let control = Color.brand(dark: 0x262D3F, light: 0xEEF2F7)
    /// Primary prose.
    static let text = Brand.text
    /// Secondary — placeholders, timestamps, the greeting.
    static let muted = Brand.textMuted
    static let dim = Brand.textDim
    /// The accent — empty-state spark, send button, caret. Brand cyan.
    static let spark = Brand.cyan
    /// The Mobius magenta, cyan's partner. Touches only: the far end of an
    /// accent gradient, a glow's second stop, the spark glyph's tail.
    static let ember = Brand.purpleBright
    /// Cyan → magenta, the strip's own run. The one gradient that says
    /// "Permagent" without a wordmark.
    static var ribbon: LinearGradient {
        LinearGradient(colors: [Brand.cyan, Brand.purpleBright],
                       startPoint: .leading, endPoint: .trailing)
    }
    /// Ink on a spark fill (cyan is bright in both appearances).
    static let onSpark = Brand.onAccent
    /// Hairlines on the raised card.
    static let border = Brand.border
}

// ── Typography ───────────────────────────────────────────────────────────────
// One ramp, mirroring `type` in tokens.ts role for role and px for pt. The
// base sizes preserve the desktop brand ratios, while semantic scaling
// respects the operator's preferred reading size on each device.
//
// Base sizes retain the brand ramp; relative custom fonts scale with Dynamic
// Type. Matching desktop pixel sizes must not disable the phone's accessibility.
//
// TYPEFACES ARE NOW THE REAL BRAND FONTS, bundled in PermagentMobile/Fonts/
// and registered via UIAppFonts in project.yml (Info.plist is generated —
// never hand-edit it). This replaces the earlier "SF Pro at the web's
// metrics" compromise: Manrope carries display/titles, Inter carries prose,
// JetBrains Mono carries code — exactly the desktop's font.display /
// font.body / font.mono in tokens.ts. Bundled files (all OFL-licensed,
// license texts alongside in Fonts/):
//   Inter-Regular.otf / Inter-Medium.otf / Inter-SemiBold.otf / Inter-Bold.otf
//   Manrope-SemiBold.ttf / Manrope-Bold.ttf / Manrope-ExtraBold.ttf
//   JetBrainsMono-Regular.ttf / JetBrainsMono-Medium.ttf
// The serif (New York) chat voice is gone deliberately — that was borrowed
// from another product's look; Permagent's long-form voice is Inter.
extension Font {
    // PostScript names, verified with fontTools nameID 6 against the bundled
    // files (SwiftUI silently falls back to SF on a wrong name — keep exact):
    //   Manrope-SemiBold / Manrope-Bold / Manrope-ExtraBold
    //   Inter-Regular / Inter-Medium / Inter-SemiBold / Inter-Bold
    //   JetBrainsMono-Regular / JetBrainsMono-Medium
    /// Display face — tokens.ts font.display (Manrope 600/700/800).
    static func manrope(_ size: CGFloat, weight: Font.Weight = .semibold) -> Font {
        switch weight {
        case .bold: return .custom("Manrope-Bold", size: size, relativeTo: .title2)
        case .heavy, .black: return .custom("Manrope-ExtraBold", size: size, relativeTo: .title2)
        default: return .custom("Manrope-SemiBold", size: size, relativeTo: .title2)
        }
    }
    /// Body face — tokens.ts font.body (Inter 400/500/600/700).
    static func inter(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        switch weight {
        case .medium: return .custom("Inter-Medium", size: size, relativeTo: .body)
        case .semibold: return .custom("Inter-SemiBold", size: size, relativeTo: .body)
        case .bold, .heavy, .black: return .custom("Inter-Bold", size: size, relativeTo: .body)
        default: return .custom("Inter-Regular", size: size, relativeTo: .body)
        }
    }
    /// Mono face — tokens.ts font.mono (JetBrains Mono 400/500). Model ids,
    /// paths, keys — mirror the desktop's mono usage.
    static func jetbrainsMono(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        switch weight {
        case .medium, .semibold, .bold: return .custom("JetBrainsMono-Medium", size: size, relativeTo: .caption)
        default: return .custom("JetBrainsMono-Regular", size: size, relativeTo: .caption)
        }
    }

    /// tokens.ts type.display — 32/600 Manrope (-0.02em tracking is applied at
    /// call sites via `.tracking(-0.64)` where trivial).
    static let brandDisplay = Font.manrope(32)
    /// tokens.ts type.title — 20/600 Manrope. Section / screen titles.
    static let brandTitle = Font.manrope(20)
    /// tokens.ts type.heading — 16/600 Manrope. Card headlines, primary rows.
    static let brandHeading = Font.manrope(16)
    /// tokens.ts type.heading — retained name for existing call sites.
    static let brandHeadline = Font.manrope(16)
    /// tokens.ts type.body — 14/400 Inter.
    static let brandBody = Font.inter(14)
    /// tokens.ts type.small — 13/400 Inter.
    static let brandSmall = Font.inter(13)
    /// tokens.ts type.caption — 12/400 Inter.
    static let brandCaption = Font.inter(12)
    /// tokens.ts type.micro — 11/500 Inter.
    static let brandMicro = Font.inter(11, weight: .medium)
    /// tokens.ts type.label — 11/600 Inter, 0.08em tracking, UPPERCASE. The
    /// tracking and casing are not carried by the font: apply `.tracking(0.88)`
    /// and `.textCase(.uppercase)` at the call site, as the web token does.
    static let brandLabel = Font.inter(11, weight: .semibold)

    /// Chat prose — the assistant's answers in the brand body face.
    static let chatProse = Font.inter(17)
    /// The empty-state greeting — large, display face, quiet.
    static let chatGreeting = Font.manrope(28)
    /// User messages — the sender's own words, slightly smaller.
    static let chatUser = Font.inter(17)
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
            #if os(iOS)
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
            #else
            content()
                .padding(16)
                .background(.ultraThinMaterial.opacity(0.6))
                .background(Brand.surface)
                .clipShape(shape)
            #endif
        }
        .overlay(shape.strokeBorder(Brand.borderHi, lineWidth: 1))
        .shadow(color: Brand.cardShadow, radius: 24, y: 12)
    }
}

extension View {
    /// Brand chrome material for floating controls (VoiceView close button,
    /// hands-free pill): real Liquid Glass on iOS 26+ — interactive glass
    /// responds to touch with the native morph — ultraThinMaterial before.
    func glassChrome<S: Shape>(in shape: S, interactive: Bool = false) -> some View {
        modifier(AccessibleGlassChrome(shape: shape, interactive: interactive))
    }
}

private struct AccessibleGlassChrome<S: Shape>: ViewModifier {
    let shape: S
    let interactive: Bool
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    @ViewBuilder
    func body(content: Content) -> some View {
        if DesignPolicy.opaqueChrome(reduceTransparency: reduceTransparency, increasedContrast: contrast == .increased) {
            content.background(ChatSurface.raised, in: shape)
                .overlay(shape.stroke(Brand.textMuted.opacity(0.5), lineWidth: 1))
        } else {
            #if os(iOS)
            if #available(iOS 26.0, *) {
                content.glassEffect(interactive ? Glass.regular.interactive() : .regular, in: shape)
            } else {
                content.background(.regularMaterial, in: shape)
            }
            #else
            content.background(.regularMaterial, in: shape)
            #endif
        }
    }
}

/// Static ambient light gives the glass context without a continuously
/// rendering backdrop or any animation competing with the agent's orb.
struct AppBackdrop: View {
    var body: some View {
        ZStack {
            Brand.deepVoid
            RadialGradient(colors: [Brand.cyan.opacity(0.055), .clear],
                           center: .topLeading, startRadius: 0, endRadius: 470)
            RadialGradient(colors: [Brand.violet.opacity(0.065), .clear],
                           center: .bottomTrailing, startRadius: 0, endRadius: 430)
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
        .allowsHitTesting(false)
    }
}

// ── Chat-surface shared components ───────────────────────────────────────────
// The design language established by ChatView + DictateView, extracted so every
// screen shares one implementation: the raised card, the full-width spark
// action, and the spark empty state.

/// A raised card in the chat composer's shape: solid raised fill, 24pt
/// continuous corners, 1px hairline. Screens are built from these the way the
/// chat is built from its input card.
struct RaisedCard<Content: View>: View {
    private let content: () -> Content
    init(@ViewBuilder content: @escaping () -> Content) { self.content = content }
    var body: some View {
        VStack(alignment: .leading, spacing: 10, content: content)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(16)
            .background(ChatSurface.raised, in: RoundedRectangle(cornerRadius: DesignPolicy.cardRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 24, style: .continuous)
                    .strokeBorder(ChatSurface.border, lineWidth: 0.5)
            )
            .shadow(color: Brand.cardShadow.opacity(0.22), radius: 12, y: 5)
    }
}

/// The chat's send-button language grown to a full-width action: spark fill,
/// dark ink, composer-card radius. One shape for every primary "do the thing"
/// action on the chat-surface screens.
struct SparkCTA: View {
    let title: String
    var systemImage: String? = nil
    var enabled: Bool = true
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 7) {
                if let systemImage {
                    Image(systemName: systemImage).font(.subheadline.weight(.semibold))
                }
                Text(title).font(.body.weight(.semibold))
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 15)
            .foregroundStyle(ChatSurface.onSpark.opacity(enabled ? 1 : 0.6))
            .background(
                ChatSurface.ribbon.opacity(enabled ? 1 : 0.3),
                in: RoundedRectangle(cornerRadius: 18, style: .continuous)
            )
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .animation(Motion.ease, value: enabled)
    }
}

/// The quiet empty moment: the cyan spark, one serif line, and an optional
/// muted caption — exactly the chat page's empty state.
struct SparkEmptyState: View {
    let line: String
    var caption: String? = nil

    var body: some View {
        VStack(spacing: 22) {
            Text("✻")
                .font(.system(size: 40))
                .foregroundStyle(ChatSurface.ribbon)
            Text(line)
                .font(.chatGreeting)
                .foregroundStyle(ChatSurface.text.opacity(0.9))
                .multilineTextAlignment(.center)
            if let caption {
                Text(caption)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 44)
            }
        }
        .frame(maxWidth: .infinity)
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
        .accessibilityLabel(thinkingLabel)
    }

    private var thinkingLabel: String {
        #if os(iOS)
        "\(AgentIdentity.shared.nameCapitalized) is thinking"
        #else
        "Thinking"
        #endif
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
        #if os(iOS)
        if #available(iOS 26.0, *) {
            self.tabBarMinimizeBehavior(.onScrollDown)
        } else {
            self
        }
        #else
        self
        #endif
    }
}
