// VoiceIdle — classify server frames that must return to ready silently.
//
// 20260821_14 logged `transcript: ""` at 23:46:41, 23:49:07, 23:49:39,
// 23:49:44, 01:31:44, 01:34:07, 01:36:53. The server used to send those as
// `error` / "No speech detected — try again". The orb flashed the toast
// even when the user had just spoken. New servers send `idle`. Old error
// strings still in flight must be treated the same way.

import Foundation

enum VoiceIdle {
    static func isTransientEmptyTurn(_ message: String?) -> Bool {
        guard let raw = message?.lowercased(), !raw.isEmpty else { return false }
        return raw.contains("no speech detected") || raw.contains("recording too short")
    }
}
