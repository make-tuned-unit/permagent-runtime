// VoiceOrbView — the iOS port of ui/command-center/src/components/voice/VoiceOrb.tsx.
//
// The desktop conversation view is a pseudo-3D particle sphere: points on a
// noise-deformed rotating sphere inside a scattered twinkling halo, lit
// cyan → electric blue → violet → magenta across the frame, with live audio
// driving deformation, spin and brightness. iOS had an unrelated visual (two
// breathing rings plus a bar meter), so the same product read as two different
// apps mid-conversation.
//
// Same constants, same maths, same palette as the web — deliberately, so the
// two stay recognisably one thing. If you retune one, retune both.
//
// BAND SYNTHESIS. The web splits a live FFT into low/mid/high; iOS surfaces a
// single RMS `level` from the voice engine. Rather than fake an FFT, this uses
// the web's own MIRROR path — the branch VoiceOrb already takes in a popped-out
// window where the analyser lives elsewhere — which spreads one level across
// three bands with synthetic detail. It is the same code path, not a new one.

import SwiftUI

#if os(watchOS)
private let N_SPHERE = 160
private let N_HALO = 50
private let ORB_SIZE: CGFloat = 96
#else
private let N_SPHERE = 500   // web uses 700; mobile budget, see note below
private let N_HALO = 170     // web uses 240
private let ORB_SIZE: CGFloat = 300
#endif

// Point counts are ~70% of the web's. SwiftUI's Canvas re-issues every draw
// command per frame with no retained scene, so the full 940 particles cost
// noticeably more here than on a GPU-composited 2D canvas. 670 keeps 60fps on
// an A17 while staying visually indistinguishable at this size.

private struct Pt {
    let x, y, z: Double
    let seed: Double
    var r: Double = 1
    var tw: Double = 1
}

/// Fibonacci sphere — even coverage with no pole clustering.
private func makeSphere() -> [Pt] {
    var out: [Pt] = []
    out.reserveCapacity(N_SPHERE)
    let golden = Double.pi * (3 - (5.0).squareRoot())
    for i in 0..<N_SPHERE {
        let y = 1 - (Double(i) / Double(N_SPHERE - 1)) * 2
        let rad = max(0, 1 - y * y).squareRoot()
        let th = golden * Double(i)
        out.append(Pt(x: cos(th) * rad, y: y, z: sin(th) * rad,
                      // Knuth multiplicative hash as UInt32 — Int is 32-bit on
                      // watchOS, and this constant does not fit in Int32.
                      seed: Double((UInt32(i) &* 2_654_435_761) % 1000) / 1000))
    }
    return out
}

/// Loose shell of drifting dust. Deterministic LCG so the halo is identical
/// every launch — a randomly reseeded halo makes visual regressions unreadable.
private func makeHalo() -> [Pt] {
    var out: [Pt] = []
    var s: UInt64 = 42
    func rnd() -> Double { s = (s &* 16807) % 2_147_483_647; return Double(s) / 2_147_483_647 }
    for _ in 0..<N_HALO {
        let u = rnd() * 2 - 1
        let th = rnd() * Double.pi * 2
        let rad = max(0, 1 - u * u).squareRoot()
        out.append(Pt(x: cos(th) * rad, y: u, z: sin(th) * rad,
                      seed: rnd(), r: 1.14 + rnd() * 0.42, tw: 1.5 + rnd() * 3.5))
    }
    return out
}

/// cyan → electric blue → violet → magenta, matching `palette()` in VoiceOrb.tsx.
private func palette(_ g: Double) -> Color {
    let stops: [(Double, Double, Double)] = [
        (0, 213, 255), (64, 120, 255), (141, 68, 174), (255, 79, 216),
    ]
    // `Int(NaN)` and `Int(inf)` are FATAL in Swift — not a wrong number, a
    // trap. `g` is derived from px/W, so a Canvas that reports zero width
    // during layout produces NaN here and takes the whole app down. That is
    // the crash on opening the voice view (reported 2026-08-04). Sanitise
    // before converting; NaN fails every comparison, so test it explicitly
    // rather than relying on min/max to clamp it.
    let safe = g.isFinite ? g : 0
    let t = min(0.9999, max(0, safe)) * Double(stops.count - 1)
    let i = Int(t), f = t - Double(i)
    let a = stops[i], b = stops[min(i + 1, stops.count - 1)]
    return Color(
        .sRGB,
        red: (a.0 + (b.0 - a.0) * f) / 255,
        green: (a.1 + (b.1 - a.1) * f) / 255,
        blue: (a.2 + (b.2 - a.2) * f) / 255,
        opacity: 1
    )
}

struct VoiceOrbView: View {
    /// Live audio level, 0…1 — the engine's RMS meter.
    var level: Double
    /// True while the agent is speaking; false while listening or thinking.
    var speaking: Bool
    /// Thinking has no audio, so the orb swells on its own rather than flatlining.
    var thinking: Bool
    /// Render size. iOS conversation uses 300; Watch chat is ~88.
    var diameter: CGFloat = ORB_SIZE

    // Motion integrator state lives in a reference type, NOT @State. Writing
    // @State from inside a Canvas draw invalidates the view on every frame —
    // a redraw loop feeding itself. TimelineView already drives the redraw;
    // this only needs somewhere to accumulate between frames.
    private final class Motion {
        var low = 0.0, mid = 0.0, high = 0.0
        var rotY = 0.0, noiseT = 0.0, lastTick = 0.0
        let sphere = makeSphere()
        let halo = makeHalo()
    }
    @State private var motion = Motion()

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        // 30fps cap: the orb this replaced ran at minimumInterval 1/30 and
        // honored Reduce Motion; uncapped .animation runs at up to 120Hz on
        // ProMotion with ~670 fills/frame on the main actor, fighting the
        // audio pipeline. `.contentShape` keeps the parent's tap-to-interrupt
        // alive even though the Canvas itself is hit-test-disabled.
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: reduceMotion)) { tl in
            let t = tl.date.timeIntervalSinceReferenceDate
            Canvas { ctx, size in
                draw(ctx: &ctx, size: size, t: t)
            }
            .frame(width: diameter, height: diameter)
            .allowsHitTesting(false)
        }
        .contentShape(Circle())
        .accessibilityHidden(true)
    }

    private func draw(ctx: inout GraphicsContext, size: CGSize, t: Double) {
        // Nothing to draw into, and every px/W below would be NaN or infinite.
        guard size.width > 1, size.height > 1 else { return }
        let dt = motion.lastTick > 0 ? min(0.1, t - motion.lastTick) : 0.016
        motion.lastTick = t

        // Sanitise the INPUT as well as the geometry. `level` is an RMS from
        // the audio engine; a single NaN sample there would otherwise poison
        // low/mid/high permanently, because the smoothing carries state
        // forward frame to frame — one bad buffer would leave the orb dead
        // for the rest of the session even after audio recovered.
        let level = self.level.isFinite ? min(1.5, max(0, self.level)) : 0

        // Mirror-path band synthesis (see file header).
        var tLow: Double, tMid: Double, tHigh: Double
        if thinking {
            let breath = 0.10 + 0.06 * (0.5 + 0.5 * sin(t * 1.6))
            tLow = breath; tMid = breath * 0.6; tHigh = breath * 0.3
        } else {
            // Speaking: `level` is now the playback tap's RMS (VoiceView's
            // player tap), i.e. the audio actually being heard — so the orb
            // tracks his syllables rather than a synthetic idle. No breath
            // fallback here on purpose: a self-driven animation during speech
            // is exactly the "random animation" this replaced.
            tLow = level
            tMid = level * (0.7 + 0.3 * sin(t * 11))
            tHigh = level * (0.5 + 0.5 * sin(t * 19 + 1.7))
        }
        // Fast attack, slow release — syllables land visibly, decay is graceful.
        func smooth(_ cur: Double, _ target: Double) -> Double {
            cur + (target - cur) * (target > cur ? 0.5 : 0.08)
        }
        let l = smooth(motion.low, tLow), m = smooth(motion.mid, tMid), h = smooth(motion.high, tHigh)
        motion.low = l; motion.mid = m; motion.high = h

        let lvl = min(1.2, l * 0.5 + m * 0.35 + h * 0.15)
        let ry = motion.rotY + dt * (0.2 + m * 0.9)          // mids spin it up
        let nt = motion.noiseT + dt * (0.9 + m * 4.5 + h * 2.0) // speech churns the surface
        motion.rotY = ry; motion.noiseT = nt

        let rotX = 0.42 + 0.06 * sin(t * 0.13)
        let cosY = cos(ry), sinY = sin(ry), cosX = cos(rotX), sinX = sin(rotX)
        let amp = 0.045 + l * 0.34                    // lows swell the whole body
        let W = size.width, cx = W / 2, cy = size.height / 2
        let R = W * 0.30

        // Backdrop glow. Fades to its OWN hue at zero alpha, never to clear
        // black — the web learned that interpolating toward transparent black
        // drags RGB down and washes the halo grey.
        ctx.fill(
            Path(ellipseIn: CGRect(x: 0, y: 0, width: W, height: size.height)),
            with: .radialGradient(
                Gradient(stops: [
                    .init(color: Color(.sRGB, red: 64/255, green: 120/255, blue: 1, opacity: 0.10 + lvl * 0.14), location: 0),
                    .init(color: Color(.sRGB, red: 141/255, green: 68/255, blue: 174/255, opacity: 0.05 + lvl * 0.08), location: 0.55),
                    .init(color: Color(.sRGB, red: 141/255, green: 68/255, blue: 174/255, opacity: 0), location: 1),
                ]),
                center: CGPoint(x: cx, y: cy),
                startRadius: R * 0.2,
                endRadius: W * 0.5
            )
        )

        // Halo dust (behind the body): slow orbital drift plus twinkle.
        for p in motion.halo {
            let a = nt * 0.12 * (0.5 + p.seed)
            let ca = cos(a), sa = sin(a)
            let x = p.x * ca - p.z * sa
            var z = p.x * sa + p.z * ca
            let y2 = p.y * cosX - z * sinX
            z = p.y * sinX + z * cosX
            let px = cx + x * R * p.r
            let py = cy + y2 * R * p.r
            // A NaN origin in a CGRect is fatal in CoreGraphics, exactly like
            // Int(NaN). Guarding only the conversions was not enough: one NaN
            // anywhere in the level→amp→rr chain reaches the geometry too.
            guard px.isFinite, py.isFinite else { continue }
            let twinkle = 0.25 + 0.5 * (0.5 + 0.5 * sin(nt * p.tw + p.seed * 40))
            let g = (px / W) * 0.6 + (py / W) * 0.4
            ctx.fill(
                Path(CGRect(x: px, y: py, width: 1.3, height: 1.3)),
                with: .color(palette(g).opacity(min(1, twinkle)))
            )
        }

        // Sphere body.
        for p in motion.sphere {
            // Organic noise: three drifting harmonics along the unit dirs.
            let d = sin(4.1 * p.x + nt * 0.9 + p.seed) * 0.5
                  + sin(3.4 * p.y - nt * 0.7) * 0.3
                  + sin(5.2 * p.z + nt * 1.2) * 0.2
            let rr = 1 + d * amp

            let x = p.x * cosY - p.z * sinY
            var z = p.x * sinY + p.z * cosY
            let y2 = p.y * cosX - z * sinX
            z = p.y * sinX + z * cosX

            let px = cx + x * rr * R
            let py = cy + y2 * rr * R
            guard px.isFinite, py.isFinite else { continue }
            let depth = (z + 1) / 2                  // 0 back … 1 front
            let g = (px / W) * 0.62 + (py / W) * 0.38
            // Highs shimmer: brightness ripples race the surface on consonants.
            let shimmer = h * 0.5 * (0.5 + 0.5 * sin(nt * 6 + p.seed * 40))
            // Clamped at BOTH ends and NaN-checked, like `sz` below. A negative
            // or non-finite opacity reaches CoreGraphics as a malformed colour;
            // the level→lvl→shimmer chain is sanitised upstream, so this is
            // defence in depth on the one value that was still unguarded.
            let alphaRaw = 0.14 + depth * (0.6 + lvl * 0.35) + shimmer
            let alpha = alphaRaw.isFinite ? min(1, max(0, alphaRaw)) : 0.14
            let szRaw = 0.9 + depth * (1.3 + lvl * 1.2) + shimmer * 1.1
            let sz = szRaw.isFinite ? max(0.1, szRaw) : 0.9
            ctx.fill(
                Path(CGRect(x: px - sz / 2, y: py - sz / 2, width: sz, height: sz)),
                with: .color(palette(g).opacity(alpha))
            )
        }
    }
}
