// Teach the speech engine a word once — the same lexicon Settings → Voice
// on the desktop writes (`PUT /voice/pronunciations`). Conversational
// `save_pronunciation` is the primary path; this is the review queue so a
// coined name can be fixed before the next spoken sentence.

import SwiftUI

struct PronunciationView: View {
    @State private var saved: [String: PronunciationEntry] = [:]
    @State private var unresolved: [UnresolvedPronunciation] = []
    @State private var word = ""
    @State private var soundsLike = ""
    @State private var errorText: String?
    @State private var busy = false
    @State private var loaded = false

    private var savedWords: [String] { saved.keys.sorted() }

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                Text("Respell with real English words, the way you'd tell a person. 'per ma gent', not IPA. Applied on the next spoken sentence — the same list the Mac uses.")
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.muted)
                    .frame(maxWidth: .infinity, alignment: .leading)

                if let errorText {
                    HubErrorCard(text: errorText) { await reload() }
                }

                RaisedCard {
                    Text("TEACH A WORD")
                        .font(.brandLabel).tracking(0.88)
                        .foregroundStyle(ChatSurface.dim)
                    TextField("word", text: $word)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.brandBody)
                    TextField("sounds like", text: $soundsLike)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.brandBody)
                    SparkCTA(
                        title: busy ? "Saving…" : "Save",
                        enabled: !busy && !word.trimmingCharacters(in: .whitespaces).isEmpty
                            && !soundsLike.trimmingCharacters(in: .whitespaces).isEmpty
                    ) {
                        Task { await teach(word, soundsLike) }
                    }
                }

                if !unresolved.isEmpty {
                    RaisedCard {
                        Text("NEEDS A READING")
                            .font(.brandLabel).tracking(0.88)
                            .foregroundStyle(ChatSurface.dim)
                        Text("Spoken with a guess last time. Teach each once — it's remembered.")
                            .font(.brandCaption)
                            .foregroundStyle(ChatSurface.muted)
                        ForEach(unresolved) { item in
                            UnresolvedTeachRow(item: item, busy: busy) { like in
                                await teach(item.word, like)
                            }
                        }
                    }
                }

                if !savedWords.isEmpty {
                    RaisedCard {
                        Text("SAVED")
                            .font(.brandLabel).tracking(0.88)
                            .foregroundStyle(ChatSurface.dim)
                        ForEach(savedWords, id: \.self) { w in
                            HStack(spacing: 8) {
                                Text(w)
                                    .font(.brandCaption)
                                    .foregroundStyle(ChatSurface.text)
                                Text(saved[w]?.sounds_like ?? "")
                                    .font(.brandCaption)
                                    .foregroundStyle(ChatSurface.muted)
                                Spacer()
                                Button("Remove") {
                                    Task { await remove(w) }
                                }
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(Brand.danger)
                                .buttonStyle(.plain)
                            }
                        }
                    }
                } else if loaded && unresolved.isEmpty {
                    SparkEmptyState(
                        line: "No words taught yet",
                        caption: "When synthesis has to spell a name out, it lands in the review queue above."
                    )
                    .padding(.top, 20)
                }
            }
            .padding()
        }
        .background { AppBackdrop() }
        .navigationTitle("Pronunciation")
        .task { await reload() }
        .refreshable { await reload() }
    }

    private func reload() async {
        errorText = nil
        do {
            async let lex = APIClient.shared.pronunciations()
            async let queue = APIClient.shared.unresolvedPronunciations()
            saved = try await lex
            unresolved = try await queue
            loaded = true
        } catch {
            errorText = "Couldn't load pronunciations — is the hub awake?"
            loaded = true
        }
    }

    private func teach(_ w: String, _ like: String) async {
        let trimmedWord = w.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedLike = like.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedWord.isEmpty, !trimmedLike.isEmpty else { return }
        busy = true
        errorText = nil
        defer { busy = false }
        do {
            try await APIClient.shared.savePronunciation(word: trimmedWord, soundsLike: trimmedLike)
            word = ""
            soundsLike = ""
            await reload()
        } catch {
            errorText = "Could not save pronunciation."
        }
    }

    private func remove(_ w: String) async {
        do {
            try await APIClient.shared.deletePronunciation(w)
            await reload()
        } catch {
            errorText = "Could not remove \(w)."
        }
    }
}

private struct UnresolvedTeachRow: View {
    let item: UnresolvedPronunciation
    let busy: Bool
    let onTeach: (String) async -> Void
    @State private var like = ""

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(item.word)
                    .font(.brandCaption)
                    .foregroundStyle(ChatSurface.text)
                Text("×\(item.spelled_out_times)")
                    .font(.caption2)
                    .foregroundStyle(ChatSurface.dim)
            }
            TextField("sounds like", text: $like)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.brandCaption)
            Button("Teach") {
                Task { await onTeach(like) }
            }
            .font(.caption.weight(.semibold))
            .foregroundStyle(ChatSurface.spark)
            .disabled(busy || like.trimmingCharacters(in: .whitespaces).isEmpty)
            .buttonStyle(.plain)
        }
    }
}
