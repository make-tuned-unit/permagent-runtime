import XCTest

final class WatchBridgeTests: XCTestCase {
    private let projects = [
        WatchProject(id: "1", name: "Evntally", slug: "evntally"),
        WatchProject(id: "2", name: "Permagent", slug: "permagent"),
        WatchProject(id: "3", name: "Personal", slug: "personal"),
    ]

    func testExactName() {
        XCTAssertEqual(ProjectMatcher.match(spoken: "Evntally", among: projects), .one(projects[0]))
    }

    func testStripsFiller() {
        XCTAssertEqual(
            ProjectMatcher.match(spoken: "the permagent project", among: projects),
            .one(projects[1])
        )
    }

    func testAmbiguousContains() {
        // "per" is a prefix of both Permagent and Personal after normalize
        if case .many(let hits) = ProjectMatcher.match(spoken: "per", among: projects) {
            XCTAssertEqual(Set(hits.map(\.id)), Set(["2", "3"]))
        } else {
            XCTFail("expected several matches for a shared prefix")
        }
    }

    func testNoMatch() {
        XCTAssertEqual(ProjectMatcher.match(spoken: "citycircle", among: projects), .none)
    }

    func testEmptyIsNone() {
        XCTAssertEqual(ProjectMatcher.match(spoken: "   ", among: projects), .none)
    }

    func testChatEndpointStopsOnTightSilence() {
        let e = WatchEndpoint.chat
        XCTAssertFalse(e.shouldStop(heardSpeech: false, spokenFor: 0, silentFor: 5, elapsed: 5))
        XCTAssertFalse(e.shouldStop(heardSpeech: true, spokenFor: 0.1, silentFor: 2, elapsed: 2.1))
        XCTAssertTrue(e.shouldStop(heardSpeech: true, spokenFor: 0.5, silentFor: 1.1, elapsed: 1.6))
        XCTAssertTrue(e.shouldStop(heardSpeech: true, spokenFor: 0.5, silentFor: 0, elapsed: e.maxDuration))
    }

    func testNoteEndpointKeepsAThinkingPause() {
        let e = WatchEndpoint.note
        XCTAssertFalse(e.shouldStop(heardSpeech: true, spokenFor: 2, silentFor: 1.1, elapsed: 3.1),
                       "a note must not cut on the chat window")
        XCTAssertTrue(e.shouldStop(heardSpeech: true, spokenFor: 2, silentFor: 1.8, elapsed: 3.8))
    }

    func testRoundTripEnvelope() throws {
        let req = WatchRequest(op: .saveNote, id: "abc", text: "hello", projectId: "1")
        let data = try JSONEncoder().encode(req)
        let decoded = try JSONDecoder().decode(WatchRequest.self, from: data)
        XCTAssertEqual(decoded, req)
    }
}
