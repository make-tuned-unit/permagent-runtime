import XCTest

final class ModelSwitchFeedbackTests: XCTestCase {
    func testLegacyOrIncompleteRoutingNeverInventsAnActiveModel() {
        XCTAssertEqual(ModelSwitchFeedback.confirmedRoute(provider: nil, model: nil).1, "")
        XCTAssertEqual(ModelSwitchFeedback.confirmedRoute(provider: "", model: "model").1, "")
        XCTAssertEqual(ModelSwitchFeedback.confirmedRoute(provider: "provider", model: " ").1, "")
        XCTAssertEqual(ModelSwitchFeedback.confirmedRoute(provider: "provider", model: "model").1, "model")
    }

    func testOldHubExplainsVersionMismatchInsteadOfBlamingCredentials() {
        for status in [404, 405] {
            XCTAssertTrue(ModelSwitchFeedback.http(status).contains("Update and restart"))
        }
    }

    func testPairingAndStorageFailuresHaveDifferentRemedies() {
        XCTAssertTrue(ModelSwitchFeedback.http(401).contains("pairing"))
        XCTAssertTrue(ModelSwitchFeedback.http(503).contains("save the model"))
        XCTAssertTrue(ModelSwitchFeedback.http(422).contains("selection"))
        XCTAssertNotEqual(ModelSwitchFeedback.http(401), ModelSwitchFeedback.http(503))
    }

    func testNoTransportOrHTTPFailureInventsProviderKeyDiagnosis() {
        let messages = [0, 400, 401, 403, 404, 405, 409, 422, 429, 500, 503]
            .map(ModelSwitchFeedback.http) + [ModelSwitchFeedback.disconnected, ModelSwitchFeedback.unknown]
        for message in messages {
            XCTAssertFalse(message.localizedCaseInsensitiveContains("key"))
        }
    }
}
