import XCTest

final class VoiceOrbDriveTests: XCTestCase {
    /// Listening at RMS 0 must still pulse — last night the orb was a dead
    /// sphere while the label said LISTENING.
    func testListeningBreathesWhenMicIsQuiet() {
        let a = VoiceOrbDrive.bands(thinking: false, listening: true, speaking: false, level: 0, t: 0)
        let b = VoiceOrbDrive.bands(thinking: false, listening: true, speaking: false, level: 0, t: 0.7)
        XCTAssertGreaterThan(a.low, 0.12, "listening orb was static at silence")
        XCTAssertNotEqual(a.low, b.low, accuracy: 0.001, "listening breath did not move")
    }

    /// Speaking must move even when the playback tap is quiet, and swell
    /// harder than listening when the tap is live.
    func testSpeakingMovesWithVoiceAndKeepsAResidualPulse() {
        let quiet = VoiceOrbDrive.bands(thinking: false, listening: false, speaking: true, level: 0, t: 0.2)
        XCTAssertGreaterThan(quiet.low, 0.08, "speaking orb died on a quiet syllable")
        let loud = VoiceOrbDrive.bands(thinking: false, listening: false, speaking: true, level: 0.6, t: 0.2)
        XCTAssertGreaterThan(loud.low, quiet.low)
        XCTAssertGreaterThan(
            VoiceOrbDrive.amp(low: 0.6, speaking: true),
            VoiceOrbDrive.amp(low: 0.6, speaking: false)
        )
        XCTAssertGreaterThan(
            VoiceOrbDrive.spin(mid: 0.6, speaking: true),
            VoiceOrbDrive.spin(mid: 0.6, speaking: false)
        )
    }

    func testThinkingStillHasItsOwnBreath() {
        let a = VoiceOrbDrive.bands(thinking: true, listening: false, speaking: false, level: 0, t: 0)
        let b = VoiceOrbDrive.bands(thinking: true, listening: false, speaking: false, level: 0, t: 1.0)
        XCTAssertGreaterThan(a.low, 0.05)
        XCTAssertNotEqual(a.low, b.low, accuracy: 0.001)
    }
}
