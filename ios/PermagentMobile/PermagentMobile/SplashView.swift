// Launch splash — the iOS half of ui/command-center/src/components/splash/Splash.tsx.
// Same mark, same tagline, same beats, same duration. Keep the two in lockstep:
// this is the first thing a user sees on either device and a drift between them
// reads as two different products.

import SwiftUI

struct SplashView: View {
    let onDone: () -> Void

    @State private var showLine1 = false
    @State private var showLine2 = false
    @State private var fadingOut = false

    var body: some View {
        ZStack {
            // OPAQUE base first. The web splash writes the cyan breath as the
            // inner stop of one gradient (`rgba(0,213,255,0.05) 0%` → bg), which
            // makes the CENTRE 95% transparent — harmless on web, where nothing
            // is painted behind it yet, but on iOS the splash sits over live UI
            // and the pairing screen showed straight through the logo
            // (simulator capture, 2026-08-04). Void underneath, breath on top.
            // ChatSurface.bg IS the brand void/pearl — the same page every
            // screen now opens on.
            ChatSurface.bg.ignoresSafeArea()
            RadialGradient(
                colors: [Brand.cyan.opacity(0.05), .clear],
                center: .init(x: 0.5, y: 0.45),
                startRadius: 0,
                endRadius: 420
            )
            .ignoresSafeArea()

            VStack(spacing: 28) {
                MobiusView(size: 110, glow: 1)

                /* ===== TAGLINE LOCKED =====
                 * Format: "Built to grow with you. Forever."
                 * - Sentence case (capital B and F only)
                 * - Two periods (after "you" and after "Forever")
                 * - NEVER uppercase — no .textCase(.uppercase) on this text
                 * - This has regressed three times on web. It is mirrored here
                 *   verbatim; do not restyle the case without instruction.
                 * ========================= */
                HStack(spacing: 4) {
                    Text("Built to grow with you.")
                        .opacity(showLine1 ? 1 : 0)
                        .offset(y: showLine1 ? 0 : 8)
                    Text("Forever.")
                        .opacity(showLine2 ? 1 : 0)
                        .offset(y: showLine2 ? 0 : 8)
                        .shadow(color: Brand.cyan.opacity(showLine2 ? 0.3 : 0), radius: 10)
                }
                // tokens.ts: font.display / 14px / 600 / 0.08em, muted at 0.7.
                .font(.system(size: 14, weight: .semibold))
                .tracking(14 * 0.08)
                .foregroundStyle(Brand.textMuted)
                .opacity(0.7)
            }
        }
        .opacity(fadingOut ? 0 : 1)
        .animation(.easeOut(duration: 0.4), value: fadingOut)
        // Tapping skips, exactly as the web splash does.
        .contentShape(Rectangle())
        .onTapGesture { finish() }
        .task {
            // Timings mirror Splash.tsx: one full Möbius revolution at double
            // speed (76 frames @30fps ≈ 2.53s), taglines arriving under it.
            try? await Task.sleep(for: .milliseconds(300))
            withAnimation(.easeOut(duration: 0.8)) { showLine1 = true }
            try? await Task.sleep(for: .milliseconds(900))
            withAnimation(.spring(response: 0.8, dampingFraction: 0.6)) { showLine2 = true }
            try? await Task.sleep(for: .milliseconds(1300))
            finish()
        }
    }

    private func finish() {
        guard !fadingOut else { return }
        fadingOut = true
        Task {
            try? await Task.sleep(for: .milliseconds(400))
            onDone()
        }
    }
}
