// Holds the standalone voice credential on the watch.
//
// The phone pushes it through WCSession application context
// (`WatchVoiceCredential.contextKey`); WatchRelay forwards the context here.
// It lives in the Keychain (this device only, after first unlock) and is
// never shown or logged. Absent, unusable or unreachable → the Chat button
// falls back to the phone relay.

import Foundation
import Security

@MainActor
final class WatchVoiceProvisioning: ObservableObject {
    static let shared = WatchVoiceProvisioning()

    @Published private(set) var credential: WatchVoiceCredential?

    private static let service = "ai.permagent.watch.voice"
    private static let account = "credential.v1"

    private init() {
        credential = Self.loadKeychain()
        #if DEBUG
        if let rehearsal = Self.rehearsalCredential() { credential = rehearsal }
        #endif
    }

    var available: Bool { credential?.isUsable == true }

    /// Application context from the phone. WatchRelay (the WCSession
    /// delegate) extracts these Sendable values and hops here.
    nonisolated static func extract(_ context: [String: Any]) -> (data: Data?, clear: Bool) {
        (context[WatchVoiceCredential.contextKey] as? Data,
         context[WatchVoiceCredential.clearKey] as? Bool == true)
    }

    func apply(credentialData data: Data?, clear: Bool) {
        if clear {
            Self.deleteKeychain()
            credential = nil
            NSLog("permagent-watchvoice: credential cleared by phone")
            return
        }
        guard let data,
              let cred = try? JSONDecoder().decode(WatchVoiceCredential.self, from: data) else { return }
        guard cred != credential else { return }
        guard cred.isUsable else {
            NSLog("permagent-watchvoice: ignored unusable credential from phone")
            return
        }
        if !Self.saveKeychain(data) {
            NSLog("permagent-watchvoice: keychain write failed; credential kept in memory only")
        }
        credential = cred
        NSLog("permagent-watchvoice: credential stored (host=%@)", URL(string: cred.hubURL)?.host ?? "?")
    }

    // MARK: Keychain

    private static var baseQuery: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private static func loadKeychain() -> WatchVoiceCredential? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess,
              let data = out as? Data else { return nil }
        return try? JSONDecoder().decode(WatchVoiceCredential.self, from: data)
    }

    private static func saveKeychain(_ data: Data) -> Bool {
        let attrs: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(baseQuery as CFDictionary, attrs as CFDictionary)
        if status == errSecSuccess { return true }
        guard status == errSecItemNotFound else { return false }
        var add = baseQuery
        add.merge(attrs) { _, new in new }
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    private static func deleteKeychain() {
        SecItemDelete(baseQuery as CFDictionary)
    }

    #if DEBUG
    /// Simulator rehearsal before the hub's LAN gateway exists: launch with
    /// `SIMCTL_CHILD_PERMAGENT_WATCH_VOICE_CREDENTIAL=/abs/path.json` holding
    /// the same JSON the phone would push (it may set `query_token_auth` for
    /// today's loopback `/voice`). Release builds contain no such hook.
    private static func rehearsalCredential() -> WatchVoiceCredential? {
        guard let path = ProcessInfo.processInfo.environment["PERMAGENT_WATCH_VOICE_CREDENTIAL"],
              path.hasPrefix("/"),
              let data = FileManager.default.contents(atPath: path),
              let cred = try? JSONDecoder().decode(WatchVoiceCredential.self, from: data),
              cred.isUsable else { return nil }
        NSLog("permagent-watchvoice: DEBUG rehearsal credential loaded (host=%@)",
              URL(string: cred.hubURL)?.host ?? "?")
        return cred
    }
    #endif
}
