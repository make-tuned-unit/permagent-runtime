import Foundation
import XCTest

final class VoiceProtocolFixtureTests: XCTestCase {
    private struct Fixture: Codable, Equatable {
        let client_messages: [VoiceClientWireMessage]
        let server_messages: [VoiceServerWireMessage]
    }

    private func repoRoot() -> URL {
        var cursor = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        while cursor.path != "/" {
            if FileManager.default.fileExists(
                atPath: cursor.appendingPathComponent("ASTRA_RULING_VOICE.md").path
            ) {
                return cursor
            }
            cursor.deleteLastPathComponent()
        }
        XCTFail("could not locate repository root from test path")
        return URL(fileURLWithPath: "/")
    }

    private func fixtureData() throws -> Data {
        let url = repoRoot()
            .appendingPathComponent("crates/goose-server/src/routes/voice_fixtures/protocol_fixture.json")
        return try Data(contentsOf: url)
    }

    func testSharedFixtureDecodesAndEncodesSemantically() throws {
        let decoder = JSONDecoder()
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let fixture = try decoder.decode(Fixture.self, from: fixtureData())
        let encoded = try encoder.encode(fixture)

        let originalObject = try JSONSerialization.jsonObject(with: fixtureData(), options: [])
        let encodedObject = try JSONSerialization.jsonObject(with: encoded, options: [])
        XCTAssertEqual(
            originalObject as? NSDictionary,
            encodedObject as? NSDictionary,
            "fixture semantic JSON changed"
        )
    }
}
