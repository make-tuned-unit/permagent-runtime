// VoiceOrbDrive — the orb's motion targets, extracted so they can be tested
// without a Canvas. Last night's listening orb sat still at RMS≈0 (AGC /
// dead tap) and barely moved on TTS because `speaking` was unused and
// `level` was the only driver.
//
// Listening must breathe on its own. Speaking must swell with the playback
// tap and keep a residual pulse so quiet syllables still move the sphere.

import Foundation

enum VoiceOrbDrive {
    struct Bands: Equatable {
        var low: Double
        var mid: Double
        var high: Double
    }

    /// Band targets for one frame. `level` is 0…1 (mic or playback RMS).
    static func bands(
        thinking: Bool,
        listening: Bool,
        speaking: Bool,
        level: Double,
        t: Double
    ) -> Bands {
        let level = level.isFinite ? min(1.5, max(0, level)) : 0
        if thinking {
            let breath = 0.10 + 0.06 * (0.5 + 0.5 * sin(t * 1.6))
            return Bands(low: breath, mid: breath * 0.6, high: breath * 0.3)
        }
        if listening {
            let breath = 0.16 + 0.11 * (0.5 + 0.5 * sin(t * 2.2))
            return Bands(
                low: max(breath, level),
                mid: max(breath * 0.75, level * (0.7 + 0.3 * sin(t * 11))),
                high: max(breath * 0.45, level * (0.5 + 0.5 * sin(t * 19 + 1.7)))
            )
        }
        if speaking {
            let residual = 0.10 + 0.05 * (0.5 + 0.5 * sin(t * 2.6))
            let drive = max(level * 1.45, residual)
            return Bands(
                low: drive,
                mid: drive * (0.7 + 0.3 * sin(t * 11)),
                high: drive * (0.5 + 0.5 * sin(t * 19 + 1.7))
            )
        }
        let breath = 0.09 + 0.06 * (0.5 + 0.5 * sin(t * 1.5))
        return Bands(
            low: max(breath, level),
            mid: max(breath * 0.65, level * 0.7),
            high: max(breath * 0.35, level * 0.5)
        )
    }

    static func amp(low: Double, speaking: Bool) -> Double {
        speaking ? 0.055 + low * 0.50 : 0.045 + low * 0.34
    }

    static func spin(mid: Double, speaking: Bool) -> Double {
        speaking ? 0.28 + mid * 1.25 : 0.20 + mid * 0.9
    }
}
