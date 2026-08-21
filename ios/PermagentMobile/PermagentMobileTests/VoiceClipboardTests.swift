import UIKit
import XCTest

@MainActor
final class VoiceClipboardTests: XCTestCase {
    func testWriteBumpsPasteboardChangeCount() {
        let before = UIPasteboard.general.changeCount
        XCTAssertTrue(VoiceClipboard.write("We acknowledge that we are on the traditional territories."))
        XCTAssertGreaterThan(UIPasteboard.general.changeCount, before)
    }

    func testWriteRefusesAnEmptyBody() {
        let before = UIPasteboard.general.changeCount
        XCTAssertFalse(VoiceClipboard.write(""))
        XCTAssertEqual(UIPasteboard.general.changeCount, before)
    }
}
