// Watch ↔ iPhone message contract. Foundation-only so the logic tests can
// compile it without WatchConnectivity, a watch destination, or the app
// target. Both the iOS relay and the watch app speak this JSON.
//
// Transport is WatchConnectivity, not the tailnet: watchOS cannot run
// Tailscale, so the phone (which already holds the hub pairing) is the
// hop. A dictated note that arrives while the phone is unreachable is
// queued on the watch and sent on reconnect — never dropped.

import Foundation

enum WatchOp: String, Codable, Sendable {
    case ping
    case chat
    case listProjects
    case resolveProject
    case saveNote
}

struct WatchRequest: Codable, Equatable, Sendable {
    var op: WatchOp
    var id: String
    var text: String?
    var projectId: String?
}

struct WatchProject: Codable, Equatable, Sendable {
    var id: String
    var name: String
    var slug: String
}

struct WatchResponse: Codable, Equatable, Sendable {
    var id: String
    var ok: Bool
    var op: String
    var text: String?
    var agentName: String?
    var paired: Bool?
    var reachable: Bool?
    var thinking: Bool?
    var done: Bool?
    var projects: [WatchProject]?
    var error: String?

    static func ack(_ id: String, op: String) -> WatchResponse {
        WatchResponse(id: id, ok: true, op: op, text: nil, agentName: nil,
                      paired: nil, reachable: nil, thinking: nil, done: nil,
                      projects: nil, error: nil)
    }

    static func fail(_ id: String, op: String, _ message: String) -> WatchResponse {
        WatchResponse(id: id, ok: false, op: op, text: nil, agentName: nil,
                      paired: nil, reachable: nil, thinking: nil, done: nil,
                      projects: nil, error: message)
    }
}

enum ProjectMatch: Equatable, Sendable {
    case none
    case one(WatchProject)
    case many([WatchProject])
}

/// Fuzzy project picker for "all via dictation" on the watch. Strips filler
/// ("the", "project") and matches name or slug, exact first then contains.
enum ProjectMatcher {
    private static let stop: Set<String> = ["the", "a", "an", "my", "project", "to", "for"]

    static func match(spoken: String, among projects: [WatchProject]) -> ProjectMatch {
        let needle = normalize(spoken)
        guard !needle.isEmpty else { return .none }

        let exact = projects.filter {
            normalize($0.name) == needle || normalize($0.slug) == needle
        }
        if exact.count == 1 { return .one(exact[0]) }
        if exact.count > 1 { return .many(exact) }

        let contained = projects.filter {
            let name = normalize($0.name)
            let slug = normalize($0.slug)
            return name.contains(needle) || needle.contains(name) || slug.contains(needle)
        }
        if contained.count == 1 { return .one(contained[0]) }
        if contained.count > 1 { return .many(contained) }
        return .none
    }

    static func normalize(_ raw: String) -> String {
        let tokens = raw.lowercased()
            .split { !$0.isLetter && !$0.isNumber }
            .map(String.init)
            .filter { !stop.contains($0) }
        return tokens.joined(separator: " ")
    }
}

/// Hands-free endpoint for watch dictation. Pure so WatchBridgeTests can drive
/// it without a microphone. Chat uses the tight window; notes use the patient
/// one so a mid-thought pause does not cut the recording.
struct WatchEndpoint: Equatable, Sendable {
    /// Trailing silence that completes the turn, once some speech was heard.
    var silence: TimeInterval
    /// Ignore trailing silence until this much voiced time has landed, so a
    /// breath before the first word does not send an empty clip.
    var minSpeech: TimeInterval
    /// Hard cap. Chat is short; notes may run to two minutes.
    var maxDuration: TimeInterval

    static let chat = WatchEndpoint(silence: 1.1, minSpeech: 0.35, maxDuration: 45)
    static let note = WatchEndpoint(silence: 1.8, minSpeech: 0.45, maxDuration: 120)

    func shouldStop(heardSpeech: Bool, spokenFor: TimeInterval, silentFor: TimeInterval,
                    elapsed: TimeInterval) -> Bool {
        if elapsed >= maxDuration { return true }
        guard heardSpeech, spokenFor >= minSpeech else { return false }
        return silentFor >= silence
    }
}

