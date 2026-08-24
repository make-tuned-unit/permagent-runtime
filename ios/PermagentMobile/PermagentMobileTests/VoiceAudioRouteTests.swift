// VoiceAudioRouteTests — the two capture regressions: speakerphone AEC
// muting the built-in mic, and a forced-speaker override stealing headphones.
// Policy is pure (port-type strings in, decisions out) so these run in the
// hosted-app-free PermagentTests target with no microphone.

import AVFoundation
import XCTest

final class VoiceAudioRouteTests: XCTestCase {

    private var speaker: String { AVAudioSession.Port.builtInSpeaker.rawValue }
    private var headphones: String { AVAudioSession.Port.headphones.rawValue }
    private var a2dp: String { AVAudioSession.Port.bluetoothA2DP.rawValue }
    private var hfp: String { AVAudioSession.Port.bluetoothHFP.rawValue }
    private var le: String { AVAudioSession.Port.bluetoothLE.rawValue }
    private var car: String { AVAudioSession.Port.carAudio.rawValue }
    private var receiver: String { AVAudioSession.Port.builtInReceiver.rawValue }

    private var headsetPorts: [AVAudioSession.Port] {
        [.headphones, .bluetoothA2DP, .bluetoothHFP, .bluetoothLE, .carAudio]
    }

    // ── SDK pin ─────────────────────────────────────────────────────────────

    /// Speaker must never be treated as an external sink, or we would skip
    /// the loudspeaker override and sit on the receiver.
    func testSpeakerIsNotAnExternalOutput() {
        XCTAssertFalse(VoiceAudioRoute.externalOutputPorts.contains(speaker))
        XCTAssertFalse(VoiceAudioRoute.externalOutputPorts.contains(receiver))
    }

    func testEveryHeadsetPortIsExternal() {
        for port in headsetPorts {
            XCTAssertTrue(
                VoiceAudioRoute.externalOutputPorts.contains(port.rawValue),
                "\(port.rawValue) missing from externalOutputPorts"
            )
        }
    }

    func testSessionModeIsDefaultNotACallMode() {
        XCTAssertEqual(VoiceAudioRoute.sessionMode, .default)
        XCTAssertNotEqual(VoiceAudioRoute.sessionMode, .voiceChat,
                          ".voiceChat ducks TTS and used to win the receiver")
        XCTAssertNotEqual(VoiceAudioRoute.sessionMode, .videoChat,
                          ".videoChat session AEC muted the mic on the 2026-08-21 rebuild")
    }

    // ── THE SPEAKERPHONE REGRESSION ─────────────────────────────────────────

    /// Forcing voice processing + loudspeaker AEC cancelled the near-end mic
    /// (2026-08-21: orb said LISTENING, never heard speech).
    func testSpeakerphoneDisablesVoiceProcessing() {
        let p = VoiceAudioRoute.policy(outputPortTypes: [speaker])
        XCTAssertTrue(p.speakerphone)
        XCTAssertFalse(p.voiceProcessing, "AEC on speakerphone mutes the built-in mic")
        XCTAssertEqual(p.outputOverride, .speaker)
    }

    /// Empty/settling route is still the phone, not a headset.
    func testNoOutputsIsSpeakerphone() {
        let p = VoiceAudioRoute.policy(outputPortTypes: [])
        XCTAssertTrue(p.speakerphone)
        XCTAssertFalse(p.voiceProcessing)
        XCTAssertEqual(p.outputOverride, .speaker)
    }

    func testBuiltInReceiverIsStillSpeakerphone() {
        let p = VoiceAudioRoute.policy(outputPortTypes: [receiver])
        XCTAssertTrue(p.speakerphone)
        XCTAssertFalse(p.voiceProcessing)
        XCTAssertEqual(p.outputOverride, .speaker)
    }

    // ── THE HEADPHONES REGRESSION ───────────────────────────────────────────

    /// `overrideOutputAudioPort(.speaker)` while AirPods were connected stole
    /// playback and could drop the headset mic.
    func testHeadphonesDoNotForceSpeaker() {
        for port in headsetPorts {
            let p = VoiceAudioRoute.policy(outputPortTypes: [port.rawValue])
            XCTAssertFalse(p.speakerphone, "\(port.rawValue) treated as speakerphone")
            XCTAssertTrue(p.voiceProcessing, "\(port.rawValue) must keep AEC")
            XCTAssertEqual(p.outputOverride, .none,
                           "\(port.rawValue) must not override to speaker")
        }
    }

    /// Headset wins when both speaker and headphones appear in the route.
    func testHeadphonesWinOverSpeakerInSameRoute() {
        let p = VoiceAudioRoute.policy(outputPortTypes: [speaker, headphones])
        XCTAssertFalse(p.speakerphone)
        XCTAssertTrue(p.voiceProcessing)
        XCTAssertEqual(p.outputOverride, .none)
    }

    func testWiredAndBluetoothAreBothHeadset() {
        let wired = VoiceAudioRoute.policy(outputPortTypes: [headphones])
        let airpods = VoiceAudioRoute.policy(outputPortTypes: [hfp])
        XCTAssertEqual(wired, airpods)
        XCTAssertEqual(VoiceAudioRoute.policy(outputPortTypes: [a2dp]).outputOverride, .none)
        XCTAssertEqual(VoiceAudioRoute.policy(outputPortTypes: [le]).outputOverride, .none)
        XCTAssertEqual(VoiceAudioRoute.policy(outputPortTypes: [car]).outputOverride, .none)
    }

    // ── Graph rebuild (dead tap after a route change) ───────────────────────

    func testRebuildWhenVoiceProcessingFlips() {
        XCTAssertTrue(VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: false, wantVoiceProcessing: true,
            captureSampleRate: 48_000, captureChannels: 1,
            liveSampleRate: 48_000, liveChannels: 1
        ), "plugging in headphones must rebuild — AEC has to come on")
        XCTAssertTrue(VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: true, wantVoiceProcessing: false,
            captureSampleRate: 48_000, captureChannels: 1,
            liveSampleRate: 48_000, liveChannels: 1
        ), "unplugging headphones must rebuild — AEC has to come off")
    }

    func testRebuildWhenInputFormatChanges() {
        XCTAssertTrue(VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: true, wantVoiceProcessing: true,
            captureSampleRate: 48_000, captureChannels: 1,
            liveSampleRate: 16_000, liveChannels: 1
        ), "AirPods often retarget the input rate; keeping the old MicPipe drops every frame")
        XCTAssertTrue(VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: false, wantVoiceProcessing: false,
            captureSampleRate: 48_000, captureChannels: 1,
            liveSampleRate: 48_000, liveChannels: 2
        ))
    }

    func testNoRebuildWhenRouteIsUnchanged() {
        XCTAssertFalse(VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: false, wantVoiceProcessing: false,
            captureSampleRate: 48_000, captureChannels: 1,
            liveSampleRate: 48_000, liveChannels: 1
        ))
    }

    /// THE TODAY REGRESSION: setCategory/speaker override posts a route
    /// change while the input is still 0 Hz. Rebuilding then threw, the
    /// error was swallowed, and both speaker and headphones sat on LISTENING.
    func testSettlingZeroRateDoesNotTearDownAWorkingGraph() {
        XCTAssertFalse(VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: false, wantVoiceProcessing: false,
            captureSampleRate: 48_000, captureChannels: 1,
            liveSampleRate: 0, liveChannels: 0
        ), "a settling 0 Hz route must not rebuild — that kills the tap")
        XCTAssertFalse(VoiceAudioRoute.mustRebuildGraph(
            voiceProcessing: true, wantVoiceProcessing: false,
            captureSampleRate: 48_000, captureChannels: 1,
            liveSampleRate: 0, liveChannels: 1
        ), "VP flip while the session is still 0 Hz must wait, not teardown")
    }

    // ── Barge-in vs speaker TTS ─────────────────────────────────────────────

    func testSpeakerphoneIgnoresEchoButAllowsALouderInterrupt() {
        XCTAssertTrue(
            VoiceAudioRoute.ignoreBargeIn(speakerphone: true, playbackRms: 0.2, micRms: 0.08),
            "his own voice in the room must not cut him off"
        )
        XCTAssertFalse(
            VoiceAudioRoute.ignoreBargeIn(speakerphone: true, playbackRms: 0.2, micRms: 0.45),
            "a real barge-in (user louder than playback) must interrupt"
        )
        XCTAssertFalse(
            VoiceAudioRoute.ignoreBargeIn(speakerphone: true, playbackRms: 0, micRms: 0.08),
            "silence after TTS must not lock out a real interrupt"
        )
    }

    func testHeadphonesNeverIgnoreBargeBecauseOfSpeakerPlayback() {
        XCTAssertFalse(VoiceAudioRoute.ignoreBargeIn(speakerphone: false, playbackRms: 0.9, micRms: 0.03))
    }

    func testSpeakerphonePolicySelectsSpeakerphoneVAD() {
        let speakerPolicy = VoiceAudioRoute.policy(outputPortTypes: [speaker])
        let speakerVAD = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.builtInMic.rawValue], speakerphone: speakerPolicy.speakerphone
        )
        XCTAssertEqual(speakerVAD.barge, VoiceVAD.speakerphoneConfig.barge)
        let headset = VoiceAudioRoute.policy(outputPortTypes: [hfp])
        let headsetVAD = VoiceVAD.configForRoute(
            inputPortTypes: [AVAudioSession.Port.bluetoothHFP.rawValue], speakerphone: headset.speakerphone
        )
        XCTAssertEqual(headsetVAD.barge, VoiceVAD.headsetConfig.barge)
    }
}
