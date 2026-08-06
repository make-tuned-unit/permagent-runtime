// Model picker — change the hub's chat model from the phone.
//
// Thin renderer over the same three daemon endpoints the desktop's model
// surfaces use: GET /config (the current GOOSE_PROVIDER/GOOSE_MODEL),
// GET /config/providers (which providers have saved keys, and their models),
// POST /config/set_provider. Only CONFIGURED providers are offered — a
// provider without a key saved in the desktop app can't run a chat, so it
// isn't listed. Setting a model here changes the hub for every client.

import SwiftUI

struct ModelPickerView: View {
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

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                if !currentModel.isEmpty {
                    RaisedCard {
                        Text("CURRENT")
                            .font(.brandLabel).tracking(0.88)
                            .foregroundStyle(ChatSurface.dim)
                        HStack(spacing: 10) {
                            Text("✻").foregroundStyle(ChatSurface.spark)
                            Text(currentModel)
                                .font(.system(.subheadline, design: .monospaced).weight(.semibold))
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
                            Text(provider.displayName.uppercased())
                                .font(.brandLabel).tracking(0.88)
                                .foregroundStyle(ChatSurface.dim)
                            ForEach(provider.models, id: \.self) { model in
                                Button {
                                    select(provider: provider.id, model: model)
                                } label: {
                                    HStack(spacing: 10) {
                                        Text(model)
                                            .font(.system(.footnote, design: .monospaced))
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
                                    .padding(.vertical, 7)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .disabled(busyModel != nil)
                            }
                        }
                    }
                    Text("Changes the hub's default model for every surface — desktop chat, this phone, and voice.")
                        .font(.caption2).foregroundStyle(ChatSurface.dim)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding()
        }
        .background(ChatSurface.bg.ignoresSafeArea())
        .navigationTitle("Model")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .sensoryFeedback(.success, trigger: switchCount)
    }

    private func isCurrent(_ provider: String, _ model: String) -> Bool {
        provider == currentProvider && model == currentModel
    }

    // ── Data ────────────────────────────────────────────────────────────────

    private struct ConfigResp: Decodable {
        let config: [String: AnyCodable]?
    }

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
        errorText = nil
        do {
            async let configTask = APIClient.shared.get("/config", as: ConfigResp.self)
            async let providersTask = APIClient.shared.get("/config/providers", as: [ProviderResp].self)
            let (config, all) = try await (configTask, providersTask)
            currentProvider = config.config?["GOOSE_PROVIDER"]?.string ?? ""
            currentModel = config.config?["GOOSE_MODEL"]?.string ?? ""
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
        } catch {
            errorText = "Couldn't load providers — is your Mac awake and on the tailnet?"
        }
        loading = false
    }

    private func select(provider: String, model: String) {
        guard !isCurrent(provider, model), busyModel == nil else { return }
        busyModel = model
        errorText = nil
        Task {
            struct Body: Encodable { let provider: String; let model: String }
            do {
                try await APIClient.shared.send("/config/set_provider", method: "POST", body: Body(provider: provider, model: model))
                currentProvider = provider
                currentModel = model
                switchCount += 1
            } catch {
                errorText = "The hub refused that model — check its key in the desktop app."
            }
            busyModel = nil
        }
    }
}
