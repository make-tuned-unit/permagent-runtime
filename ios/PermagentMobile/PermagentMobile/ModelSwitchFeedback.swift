import Foundation

/// Model-route persistence is not a provider credential probe. Never diagnose
/// a bad API key from an unrelated HTTP, pairing, or transport failure.
enum ModelSwitchFeedback {
    /// A global default is not evidence of a role's resolved routing. Legacy
    /// hubs may apply an independent voice override on the server.
    static func confirmedRoute(provider: String?, model: String?) -> (String, String) {
        guard let provider, let model,
              !provider.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return ("", "") }
        return (provider, model)
    }

    static func http(_ status: Int) -> String {
        switch status {
        case 401, 403:
            return "The hub couldn't authorize this change. Check your phone's pairing in the desktop app."
        case 404, 405:
            return "This hub doesn't support model switching from this app yet. Update and restart the desktop hub."
        case 400, 422:
            return "The hub couldn't accept this provider/model selection. Refresh the model list and try again."
        case 409:
            return "The model settings changed elsewhere. Refresh the model list and try again."
        case 429:
            return "The hub is busy. Wait a moment and try again."
        case 500...599:
            return "The hub couldn't save the model setting. Check the desktop hub and try again."
        default:
            return "Couldn't change models (HTTP \(status)). Your previous selection is unchanged."
        }
    }

    static let disconnected = "Couldn't reach the hub. Check the connection and try again; your previous selection is unchanged."
    static let unknown = "Couldn't change models. Your previous selection is unchanged; check the hub and try again."
}
