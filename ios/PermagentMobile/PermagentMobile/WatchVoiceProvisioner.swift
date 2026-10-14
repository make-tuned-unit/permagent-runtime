// Hands the watch what it needs to talk to the hub on its own (Astra ruling
// 2026-10-13): the phone asks the hub to mint a watch-scoped, voice-only
// credential bound to this install's conversation, then pushes it to the
// watch through WCSession application context. The phone's own pairing
// token never leaves the phone, and the watch never opens a claim link.
//
// Hub route (voice-gateway lane, wq-reports/build/watch-voice-gateway-20261013.md):
//   POST /api/devices/{phone_device_id}/watch-credential  {"conversation_id": "…"}
//   200 {"hub_url_lan":"wss://<lan>:3443","voice_token":"…","conversation_id":"…",
//        "expires_at":"…","cert_sha256":"AA:BB:…","device":{…}}
//   403 not this phone · 404 unknown phone · 409 gateway off / no LAN host
// Minting again rotates (revokes the previous watch credential), so this
// mints only when due, not on every status push.

import Foundation
import WatchConnectivity

@MainActor
final class WatchVoiceProvisioner {
    static let shared = WatchVoiceProvisioner()

    /// Re-mint well inside the hub's 30-day default TTL.
    private static let refreshInterval: TimeInterval = 7 * 24 * 60 * 60
    /// Gateway off or phone id unknown: ask again later, not on every push.
    private static let retryInterval: TimeInterval = 10 * 60
    private nonisolated static let deviceIdKey = "ai.permagent.ios-device-id"
    private nonisolated static let lastMintKey = "ai.permagent.watch-voice-minted-at"

    private struct MintBody: Encodable { let conversation_id: String }

    private struct MintResponse: Decodable {
        let hub_url_lan: String
        let voice_token: String
        let conversation_id: String?
        let expires_at: String?
        let cert_sha256: String?
    }

    private var inFlight = false
    private var nextAttempt: Date?

    /// The pairing claim answers `{"token", "device": {"id", …}}`; the id is
    /// what the mint route is keyed on. Called from the claim exchange.
    nonisolated static func rememberPairedDevice(claimResponse data: Data) {
        struct Claim: Decodable { struct Device: Decodable { let id: String }; let device: Device? }
        guard let id = (try? JSONDecoder().decode(Claim.self, from: data))?.device?.id, !id.isEmpty else { return }
        UserDefaults.standard.set(id, forKey: deviceIdKey)
        // A new pairing means a new hub or identity: mint fresh.
        UserDefaults.standard.removeObject(forKey: lastMintKey)
    }

    /// Called whenever the phone pushes watch status (pairing, activation,
    /// watch state change). Cheap when nothing is due.
    /// The suite's current chat moved: the watch credential is bound to a
    /// conversation, so mint again now rather than at the weekly refresh.
    func currentChatChanged() {
        UserDefaults.standard.removeObject(forKey: Self.lastMintKey)
        nextAttempt = nil
        refreshIfNeeded()
    }

    func refreshIfNeeded() {
        guard !inFlight, WCSession.isSupported() else { return }
        let session = WCSession.default
        guard session.activationState == .activated, session.isPaired, session.isWatchAppInstalled else { return }
        if let nextAttempt, Date() < nextAttempt { return }
        if let last = UserDefaults.standard.object(forKey: Self.lastMintKey) as? Date,
           Date().timeIntervalSince(last) < Self.refreshInterval,
           !session.applicationContext.isEmpty { return }
        guard let deviceId = UserDefaults.standard.string(forKey: Self.deviceIdKey) else {
            // Paired before the id was kept: re-pairing once enables direct voice.
            NSLog("permagent-watchvoice: phone device id unknown; re-pair to enable watch direct voice")
            nextAttempt = Date().addingTimeInterval(Self.retryInterval)
            return
        }
        inFlight = true
        Task {
            defer { self.inFlight = false }
            guard await APIClient.shared.isPaired else { return }
            do {
                let conversation = try await MobileSession.currentChatSessionId()
                let data = try await APIClient.shared.postData(
                    "/api/devices/\(deviceId)/watch-credential",
                    body: MintBody(conversation_id: conversation))
                let minted = try JSONDecoder().decode(MintResponse.self, from: data)
                let credential = WatchVoiceCredential(
                    hubURL: minted.hub_url_lan,
                    voiceToken: minted.voice_token,
                    conversationId: minted.conversation_id ?? conversation,
                    certFingerprint: minted.cert_sha256,
                    expiresAt: minted.expires_at,
                    queryTokenAuth: nil)
                guard credential.isUsable, credential.conversationId == conversation else {
                    NSLog("permagent-watchvoice: hub returned an unusable watch credential; not sent")
                    self.nextAttempt = Date().addingTimeInterval(Self.retryInterval)
                    return
                }
                let payload = try JSONEncoder().encode(credential)
                try WCSession.default.updateApplicationContext([WatchVoiceCredential.contextKey: payload])
                UserDefaults.standard.set(Date(), forKey: Self.lastMintKey)
                NSLog("permagent-watchvoice: watch credential sent (host=%@)",
                      URL(string: credential.hubURL)?.host ?? "?")
            } catch APIError.badStatus(let status) {
                // 409: gateway off on this hub; 403/404: id mismatch. The relay
                // stays the path either way.
                NSLog("permagent-watchvoice: watch credential not minted (HTTP %d)", status)
                self.nextAttempt = Date().addingTimeInterval(Self.retryInterval)
            } catch {
                NSLog("permagent-watchvoice: watch credential not minted (%@)",
                      String(describing: type(of: error)))
                self.nextAttempt = Date().addingTimeInterval(Self.retryInterval)
            }
        }
    }

    /// Unpairing the phone should also make the watch forget direct voice
    /// (the hub already cascades the revocation to the watch credential).
    func clearWatchCredential() {
        UserDefaults.standard.removeObject(forKey: Self.lastMintKey)
        guard WCSession.isSupported(), WCSession.default.activationState == .activated else { return }
        try? WCSession.default.updateApplicationContext([WatchVoiceCredential.clearKey: true])
    }
}
