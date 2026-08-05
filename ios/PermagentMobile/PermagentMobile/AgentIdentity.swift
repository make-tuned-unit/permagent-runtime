// The agent's NAME, from the hub — never a literal.
//
// "Henry" is this hub's default persona, not the product. Every user names
// their own agent (Settings → Identity on the desktop, `PUT /api/agent/identity`),
// and until 2026-08-04 the iOS app had "Henry" hardcoded in ~20 places across
// six files, so anyone else's phone lied about who they were talking to.
//
// The hub is the single source of truth: `GET /api/agent/identity` returns
// `first_name` and a composed `display_name`. Surfaces read `AgentIdentity.shared.name`.

import SwiftUI

@MainActor
final class AgentIdentity: ObservableObject {
    static let shared = AgentIdentity()

    /// Given name — the form that reads naturally mid-sentence ("Ask Ada to…").
    @Published private(set) var name: String = AgentIdentity.fallback

    /// Full display name, for titles and headers.
    @Published private(set) var displayName: String = AgentIdentity.fallback

    /// Used until the hub answers, and if it never does. Deliberately generic:
    /// a wrong NAME is worse than no name, because it asserts an identity the
    /// user did not choose. "your agent" is true for everybody.
    static let fallback = "your agent"

    /// Sentence-start form. The fallback is lowercase so it reads correctly
    /// mid-sentence ("Ask your agent to…"), which makes it wrong at the start
    /// of one ("All quiet. your agent will…"). Capitalising only the first
    /// character leaves a real name untouched — "Henry" stays "Henry", never
    /// "HENRY" or a re-cased "McTavish".
    var nameCapitalized: String {
        guard let first = name.first else { return name }
        return first.uppercased() + name.dropFirst()
    }

    private struct Response: Decodable {
        let first_name: String?
        let display_name: String?
    }

    /// Refresh from the hub. Safe to call on every launch and after re-pairing;
    /// failure quietly leaves the previous value rather than reverting to the
    /// fallback, so a flaky network does not rename the agent mid-session.
    func refresh() async {
        guard let r = try? await APIClient.shared.get("/api/agent/identity", as: Response.self) else {
            return
        }
        let first = r.first_name?.trimmingCharacters(in: .whitespaces) ?? ""
        let display = r.display_name?.trimmingCharacters(in: .whitespaces) ?? ""
        if !first.isEmpty { name = first }
        if !display.isEmpty { displayName = display }
        else if !first.isEmpty { displayName = first }
    }
}
