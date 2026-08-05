// The Möbius mark, animated — the iOS half of ui/command-center/src/components/mobius/Mobius.tsx.
//
// The web logo is not a vector: it is a 151-frame WebP sequence in
// `ui/command-center/public/mobius/`. `MobiusFrames/` here carries every SECOND
// source frame (76 of them, 700px wide) so the loop plays at double speed —
// which is exactly what the web splash now does via its `frameStep={2}` prop.
// Both platforms therefore complete one full revolution in ~2.5s. If you
// regenerate one set, regenerate the other or the two splashes drift apart.

import SwiftUI

/// Frames are named `mobius_00` … `mobius_75` and live flat in the bundle
/// (xcodegen folds the folder into the resources phase, so there is no
/// subdirectory to address).
private let mobiusFrameCount = 76
private let mobiusFPS: Double = 30

/// Natural aspect of the source frames (1024 × 485). Width is derived from
/// height, matching the web component's `size = height` convention.
let mobiusAspect: CGFloat = 1024.0 / 485.0

@MainActor
final class MobiusFrames: ObservableObject {
    static let shared = MobiusFrames()
    @Published private(set) var images: [UIImage] = []

    private var loading = false

    /// Decode off the main thread and publish PROGRESSIVELY — frame 0 lands in
    /// milliseconds and the mark is on screen immediately, with the rest
    /// arriving ahead of the playhead.
    ///
    /// Two traps, both caught on the simulator 2026-08-04 (the splash rendered
    /// as a near-blank dark screen and the logo only appeared during the
    /// fade-out, i.e. after the splash was already over):
    ///
    /// 1. Publishing once at the end means nothing is drawable for the whole
    ///    decode, which is most of the splash's 2.5s.
    /// 2. `UIImage(data:)` does NOT decode — it defers until first draw, so all
    ///    76 decodes would land on the main thread mid-animation. Forcing it
    ///    here with `preparingForDisplay()` is what keeps playback smooth.
    func load() {
        guard !loading, images.isEmpty else { return }
        loading = true
        Task.detached(priority: .userInitiated) {
            for i in 0..<mobiusFrameCount {
                let name = String(format: "mobius_%02d", i)
                guard let url = Bundle.main.url(forResource: name, withExtension: "webp"),
                      let data = try? Data(contentsOf: url),
                      let image = UIImage(data: data)
                else { continue }
                let ready = image.preparingForDisplay() ?? image
                await MainActor.run { self.images.append(ready) }
            }
        }
    }
}

struct MobiusView: View {
    /// Height in points; width follows the source aspect, as on the web.
    var size: CGFloat = 120
    /// 0–1, matching the web `glow` prop.
    var glow: Double = 1
    /// Play once and stop on the final frame, or loop forever.
    var loops: Bool = true

    @StateObject private var frames = MobiusFrames.shared
    @State private var index = 0
    @State private var timer: Timer?

    private var width: CGFloat { (size * mobiusAspect).rounded() }

    var body: some View {
        ZStack {
            if glow > 0 {
                Circle()
                    .fill(
                        RadialGradient(
                            colors: [Brand.cyan.opacity(0.45 * glow), .clear],
                            center: .center,
                            startRadius: 0,
                            endRadius: size * 0.5
                        )
                    )
                    .frame(width: size, height: size)
                    .blur(radius: size * 0.4)
                    .allowsHitTesting(false)
            }
            if let image = currentImage {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(width: width, height: size)
            } else {
                // Reserve the footprint so the tagline below does not jump when
                // the frames finish decoding.
                Color.clear.frame(width: width, height: size)
            }
        }
        .frame(width: width, height: size)
        .onAppear {
            frames.load()
            start()
        }
        .onDisappear { stop() }
    }

    private var currentImage: UIImage? {
        guard !frames.images.isEmpty else { return nil }
        return frames.images[min(index, frames.images.count - 1)]
    }

    private func start() {
        stop()
        let t = Timer(timeInterval: 1.0 / mobiusFPS, repeats: true) { _ in
            Task { @MainActor in
                let loaded = frames.images.count
                guard loaded > 0 else { return }
                if index + 1 >= mobiusFrameCount {
                    // End of the real sequence — wrap or hold.
                    if loops { index = 0 } else { stop() }
                } else if index + 1 < loaded {
                    index += 1
                }
                // else: the playhead has caught up with the decoder. Hold this
                // frame rather than wrapping early — wrapping on the LOADED
                // count would make a partly-decoded sequence loop over its
                // first few frames and read as a stutter, not a revolution.
            }
        }
        // .common so the animation keeps running while a scroll view tracks —
        // the default run-loop mode would freeze the mark mid-turn.
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    private func stop() {
        timer?.invalidate()
        timer = nil
    }
}
