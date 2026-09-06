import XCTest
import UIKit

final class DesignPolicyTests: XCTestCase {
    func testVoiceIdentityUsesAnAvailableSystemSymbol() {
        XCTAssertNotNil(UIImage(systemName: DesignPolicy.voiceIdentitySymbol))
    }
    #if DEBUG
    func testPreviewRequiresExplicitRecognizedLaunchArgument() {
        XCTAssertNil(DesignPreview.screen(arguments: []))
        XCTAssertNil(DesignPreview.screen(arguments: ["--design-preview=unknown"]))
        XCTAssertNil(DesignPreview.screen(arguments: ["--design-preview="]))
        XCTAssertEqual(DesignPreview.screen(arguments: ["app", "--design-preview=voice"]), "voice")
    }
    #endif
    func testFloatingControlsMeetMinimumTouchTarget() {
        XCTAssertGreaterThanOrEqual(DesignPolicy.controlSize, 44)
    }
    func testTabPageHeadersKeepDecisionsTitleBandGeometry() {
        XCTAssertEqual(DesignPolicy.pageHeaderHorizontalPadding, 18)
        XCTAssertEqual(DesignPolicy.pageHeaderTopPadding, 6)
        XCTAssertEqual(DesignPolicy.pageHeaderBottomPadding, 8)
    }
    func testUnknownHealthIsNotPresentedAsOnline() {
        XCTAssertEqual(DesignPolicy.hubStatus(nil), "Connecting to your hub")
        XCTAssertEqual(DesignPolicy.hubStatus(true), "Hub online")
        XCTAssertEqual(DesignPolicy.hubStatus(false), "Hub unreachable")
    }
    func testAccessibleVoiceCaptionGetsScrollableRoomWithoutShrinkingOrb() {
        XCTAssertGreaterThan(DesignPolicy.voiceCaptionHeight(accessibilitySize: true),
                             DesignPolicy.voiceCaptionHeight(accessibilitySize: false))
    }
    func testAccessibilitySettingsAlwaysChooseOpaqueChrome() {
        XCTAssertFalse(DesignPolicy.opaqueChrome(reduceTransparency: false, increasedContrast: false))
        XCTAssertTrue(DesignPolicy.opaqueChrome(reduceTransparency: true, increasedContrast: false))
        XCTAssertTrue(DesignPolicy.opaqueChrome(reduceTransparency: false, increasedContrast: true))
        XCTAssertTrue(DesignPolicy.opaqueChrome(reduceTransparency: true, increasedContrast: true))
    }
}
