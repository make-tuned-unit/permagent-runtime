// Model picker — change the hub's chat model from the phone.
//
// Thin renderer over the same three daemon endpoints the desktop's model
// surfaces use: GET /config (the current GOOSE_PROVIDER/GOOSE_MODEL),
// GET /config/providers (which providers have saved keys, and their models),
// POST /config/model-route (chat or voice role; applied on the next turn).
// Only CONFIGURED providers are offered — a
// provider without a key saved in the desktop app can't run a chat, so it
// isn't listed. Chat and voice scopes change only their role route, beginning
// with the next turn; the current turn keeps its original provider/model.

import SwiftUI

private struct ModelConfigResponse: Decodable {
    let config: [String: AnyCodable]?
    let resolved_routes: [String: ResolvedRouteResponse]?
}

private struct ResolvedRouteResponse: Decodable {
    let provider: String
    let model: String
}

@MainActor
final class ModelSelectionStore: ObservableObject {
    static let shared = ModelSelectionStore()

    @Published private(set) var chatProvider = ""
    @Published private(set) var chatModel = ""
    @Published private(set) var voiceProvider = ""
    @Published private(set) var voiceModel = ""

    func update(scope: ModelPickerView.Scope, provider: String, model: String) {
        switch scope {
        case .chat:
            chatProvider = provider
            chatModel = model
        case .voice:
            voiceProvider = provider
            voiceModel = model
        }
    }

    func accessibilityLabel(scope: ModelPickerView.Scope) -> String {
        let provider = scope == .voice ? voiceProvider : chatProvider
        let model = scope == .voice ? voiceModel : chatModel
        guard !model.isEmpty else { return "Choose model. Open model settings." }
        if provider.isEmpty { return "Choose model. Configured model \(model)." }
        return "Choose model. Configured model \(model) via \(provider)."
    }

    func refresh(scope: ModelPickerView.Scope) async {
        guard let response = try? await APIClient.shared.get("/config", as: ModelConfigResponse.self) else { return }
        let route = ModelSwitchFeedback.confirmedRoute(
            provider: response.resolved_routes?[scope.routeKey]?.provider,
            model: response.resolved_routes?[scope.routeKey]?.model)
        update(scope: scope, provider: route.0, model: route.1)
    }
}

struct ModelPickerView: View {
    enum Scope: Equatable {
        case chat
        case voice

        var title: String { self == .voice ? "Voice model" : "Chat model" }
        var routeKey: String { self == .voice ? "voice" : "chat" }
    }

    var scope: Scope = .chat

    private struct ProviderRow: Identifiable {
        let id: String        // provider name ("anthropic", "openrouter", …)
        let displayName: String
        let models: [String]
    }

    @State private var providers: [ProviderRow] = []
    @State private var currentProvider = ""
    @State private var currentModel = ""
    @State private var busyModel: String?
    @State private var errorText: String?
    @State private var loading = true
    @State private var switchCount = 0
    @State private var expandedProvider: String?
    @ObservedObject private var selection = ModelSelectionStore.shared

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                if !currentModel.isEmpty {
                    RaisedCard {
                        Text("CONFIGURED")
                            .font(.brandLabel).tracking(0.88)
                            .foregroundStyle(ChatSurface.dim)
                        HStack(spacing: 10) {
                            Text("✻").foregroundStyle(ChatSurface.spark)
                            Text(currentModel)
                                .font(.brandHeadline)
                                .foregroundStyle(ChatSurface.text)
                        }
                        Text("via \(currentProvider)")
                            .font(.brandCaption).foregroundStyle(ChatSurface.muted)
                    }
                }

                if let errorText {
                    Text(errorText)
                        .font(.brandCaption).foregroundStyle(Brand.danger)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                if loading {
                    ProgressView().tint(ChatSurface.spark).padding(.top, 30)
                } else if providers.isEmpty {
                    SparkEmptyState(
                        line: "No providers ready",
                        caption: "Save an API key in the desktop app (Settings → API keys) and it appears here."
                    )
                    .padding(.top, 30)
                } else {
                    ForEach(providers) { provider in
                        RaisedCard {
                            Button {
                                withAnimation(.easeOut(duration: 0.16)) {
                                    expandedProvider = expandedProvider == provider.id ? nil : provider.id
                                }
                            } label: {
                                HStack {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(provider.displayName)
                                            .font(.brandHeadline)
                                            .foregroundStyle(ChatSurface.text)
                                        Text("\(provider.models.count) available model\(provider.models.count == 1 ? "" : "s")")
                                            .font(.brandCaption).foregroundStyle(ChatSurface.muted)
                                    }
                                    Spacer()
                                    Image(systemName: expandedProvider == provider.id ? "chevron.up" : "chevron.down")
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(ChatSurface.muted)
                                }
                                .frame(minHeight: DesignPolicy.controlSize)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)

                            if expandedProvider == provider.id {
                            ForEach(provider.models, id: \.self) { model in
                                Button {
                                    select(provider: provider.id, model: model)
                                } label: {
                                    HStack(spacing: 10) {
                                        Text(model)
                                            .font(.inter(15))
                                            .foregroundStyle(isCurrent(provider.id, model) ? ChatSurface.text : ChatSurface.muted)
                                            .multilineTextAlignment(.leading)
                                        Spacer()
                                        if busyModel == model {
                                            ProgressView().controlSize(.small).tint(ChatSurface.spark)
                                        } else if isCurrent(provider.id, model) {
                                            Image(systemName: "checkmark")
                                                .font(.caption.weight(.bold))
                                                .foregroundStyle(ChatSurface.spark)
                                        }
                                    }
                                    .padding(.horizontal, 12)
                                    .frame(minHeight: 48)
                                    .padding(.vertical, 2)
                                    .background(isCurrent(provider.id, model) ? Brand.cyanSoft : Color.clear,
                                                in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .disabled(busyModel != nil)
                            }
                            }
                        }
                    }
                    Text(scope == .voice
                        ? "Applies to the next spoken turn. The current turn finishes on its existing model."
                        : "Applies to the next chat turn. Voice keeps its own configured route.")
                        .font(.caption2).foregroundStyle(ChatSurface.dim)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding()
        }
        .background { AppBackdrop() }
        .safeAreaPadding(.top, 12)
        .navigationTitle(scope.title)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sensoryFeedback(.success, trigger: switchCount)
    }

    private func isCurrent(_ provider: String, _ model: String) -> Bool {
        provider == currentProvider && model == currentModel
    }

    // ── Data ────────────────────────────────────────────────────────────────

    private struct ProviderResp: Decodable {
        struct KnownModel: Decodable { let name: String }
        struct Metadata: Decodable {
            let display_name: String?
            let default_model: String?
            let known_models: [KnownModel]?
        }
        let name: String
        let metadata: Metadata?
        let is_configured: Bool
    }

    private func load() async {
        #if DEBUG
        if DesignPreview.enabled {
            providers = [ProviderRow(id: "local", displayName: "On your Mac", models: ["Local model"]),
                         ProviderRow(id: "cloud", displayName: "Configured provider", models: ["Balanced model", "Reasoning model", "Fast model"])]
            currentProvider = "cloud"
            currentModel = "Balanced model"
            expandedProvider = "cloud"
            loading = false
            return
        }
        #endif
        errorText = nil
        do {
            async let configTask = APIClient.shared.get("/config", as: ModelConfigResponse.self)
            async let providersTask = APIClient.shared.get("/config/providers", as: [ProviderResp].self)
            let (config, all) = try await (configTask, providersTask)
            let providerKey = scope == .voice ? "voice_provider" : "chat_provider"
            let modelKey = scope == .voice ? "voice_model" : "chat_model"
            currentProvider = config.config?[providerKey]?.string
                ?? config.config?["GOOSE_PROVIDER"]?.string ?? ""
            currentModel = config.config?[modelKey]?.string
                ?? config.config?["GOOSE_MODEL"]?.string ?? ""
            if let resolved = config.resolved_routes?[scope.routeKey] {
                currentProvider = resolved.provider
                currentModel = resolved.model
            }
            let confirmed = ModelSwitchFeedback.confirmedRoute(
                provider: config.resolved_routes?[scope.routeKey]?.provider,
                model: config.resolved_routes?[scope.routeKey]?.model)
            selection.update(scope: scope, provider: confirmed.0, model: confirmed.1)
            if config.resolved_routes?[scope.routeKey] == nil {
                errorText = "This hub reports saved defaults, not the resolved model for this role. Update and restart the hub to confirm routing."
            }
            providers = all
                .filter(\.is_configured)
                .map { p in
                    var models = (p.metadata?.known_models ?? []).map(\.name)
                    if let def = p.metadata?.default_model, !models.contains(def) {
                        models.insert(def, at: 0)
                    }
                    // The live model must always be selectable-back even if the
                    // catalog doesn't list it (custom/edge models).
                    if p.name == currentProvider, !currentModel.isEmpty, !models.contains(currentModel) {
                        models.insert(currentModel, at: 0)
                    }
                    return ProviderRow(
                        id: p.name,
                        displayName: p.metadata?.display_name ?? p.name,
                        models: models
                    )
                }
                .filter { !$0.models.isEmpty }
                .sorted { a, b in
                    // The active provider reads first.
                    if a.id == currentProvider { return true }
                    if b.id == currentProvider { return false }
                    return a.displayName.localizedCaseInsensitiveCompare(b.displayName) == .orderedAscending
                }
            if expandedProvider == nil { expandedProvider = currentProvider.isEmpty ? providers.first?.id : currentProvider }
        } catch {
            errorText = "Couldn't load models — check that your hub is reachable and its providers are configured."
        }
        loading = false
    }

    private func select(provider: String, model: String) {
        guard !isCurrent(provider, model), busyModel == nil else { return }
        busyModel = model
        errorText = nil
        Task {
            do {
                struct RouteBody: Encodable { let role: String; let provider: String; let model: String }
                let role = scope.routeKey
                try await APIClient.shared.send("/config/model-route", method: "POST", body: RouteBody(role: role, provider: provider, model: model))
                currentProvider = provider
                currentModel = model
                selection.update(scope: scope, provider: provider, model: model)
                switchCount += 1
            } catch APIError.badStatus(let status) {
                errorText = ModelSwitchFeedback.http(status)
            } catch APIError.unauthorized {
                errorText = ModelSwitchFeedback.http(401)
            } catch APIError.notPaired {
                errorText = ModelSwitchFeedback.http(401)
            } catch is URLError {
                errorText = ModelSwitchFeedback.disconnected
            } catch {
                errorText = ModelSwitchFeedback.unknown
            }
            busyModel = nil
        }
    }
}
