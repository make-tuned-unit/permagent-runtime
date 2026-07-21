// Dictate — speak a note on the phone, land it on the hub.
//
// The capture flow, end to end: record (visible pulse + live level meter) →
// the HUB transcribes with its local Whisper (`/api/dictation/transcribe`;
// no cloud STT, ever) → review an editable transcript + proposed to-dos
// (deterministic on-device heuristics, nothing auto-created) → pick a project
// (confirm-first — nothing is written until you choose and tap Save) → the
// note lands via `POST /api/projects/{id}/notes` (the daemon enforces the
// note contract: durable source tag, description-less Brain write) and each
// CONFIRMED to-do becomes a standard card on the project board. If the
// project has no board columns, the to-dos are folded into the note as a
// `## To-dos` checklist instead — honest, never silently dropped.

import SwiftUI
import AVFoundation

// ── Recorder: 16 kHz mono 16-bit WAV, exactly what the hub's Whisper wants ────

@MainActor
final class DictationRecorder: NSObject, ObservableObject, AVAudioRecorderDelegate {
    @Published var isRecording = false
    @Published var elapsed: TimeInterval = 0
    /// Rolling mic-level history (0…1), newest last — drives the live meter.
    @Published var levels: [CGFloat] = Array(repeating: 0, count: 28)

    /// Hard cap: ~10 min of 16 kHz/16-bit mono ≈ 19 MB, safely under the
    /// daemon's 25 MB transcribe ceiling.
    static let maxDuration: TimeInterval = 600

    /// Fires exactly once per clip, when recording ends (user stop OR the
    /// duration cap) with the finished WAV's URL — nil if the clip failed.
    var onFinish: ((URL?) -> Void)?

    private var recorder: AVAudioRecorder?
    private var meterTimer: Timer?
    private var fileURL: URL?

    enum MicPermission { case granted, denied }

    func requestPermission() async -> MicPermission {
        await AVAudioApplication.requestRecordPermission() ? .granted : .denied
    }

    func start() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.record, mode: .default)
        try session.setActive(true)

        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("dictation-\(UUID().uuidString).wav")
        // LinearPCM into a .wav container — decodes cleanly on any daemon build
        // (mirrors the desktop composer's "PCM as WAV" choice).
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 16_000.0,
            AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
        ]
        let rec = try AVAudioRecorder(url: url, settings: settings)
        rec.delegate = self
        rec.isMeteringEnabled = true
        rec.record(forDuration: Self.maxDuration)

        recorder = rec
        fileURL = url
        elapsed = 0
        levels = Array(repeating: 0, count: levels.count)
        isRecording = true
        meterTimer = Timer.scheduledTimer(withTimeInterval: 0.07, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }

    func stop() { recorder?.stop() }  // completion flows through the delegate

    func cancel() {
        onFinish = nil
        recorder?.stop()
        recorder?.deleteRecording()
        teardown()
    }

    private func tick() {
        guard let rec = recorder, rec.isRecording else { return }
        elapsed = rec.currentTime
        rec.updateMeters()
        // dB (−160…0) → 0…1 with a gentle boost so speech reads visibly.
        let db = rec.averagePower(forChannel: 0)
        let level = min(1, CGFloat(pow(10, db / 20)) * 3.2)
        levels.removeFirst()
        levels.append(level)
    }

    nonisolated func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        Task { @MainActor in
            let url = flag ? self.fileURL : nil
            let finish = self.onFinish
            self.onFinish = nil
            self.teardown()
            finish?(url)
        }
    }

    private func teardown() {
        meterTimer?.invalidate()
        meterTimer = nil
        recorder = nil
        isRecording = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}

// ── To-do extraction: deterministic v1, propose-only ─────────────────────────
// Heuristic marker phrases over sentence segments. Every hit is a PROPOSAL the
// user toggles on-device — nothing is created unconfirmed. (LLM extraction is
// a flagged follow-up, deliberately not v1: this path must be predictable.)

enum TodoExtractor {
    /// Markers that introduce an action. Matched case-insensitively; the to-do
    /// text is what FOLLOWS the marker (or the whole sentence for bare labels).
    private static let markers = [
        "i need to", "i have to", "i've got to", "we need to", "we have to",
        "remind me to", "don't forget to", "make sure to", "make sure we",
        "todo", "to do", "to-do", "action item", "next step is to", "next step",
    ]

    static func propose(from transcript: String) -> [String] {
        let sentences = transcript
            .replacingOccurrences(of: "\n", with: ". ")
            .components(separatedBy: CharacterSet(charactersIn: ".!?;"))
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }

        var seen = Set<String>()
        var out: [String] = []
        for sentence in sentences {
            let lower = sentence.lowercased()
            guard let marker = markers.first(where: { lower.contains($0) }) else { continue }
            guard let range = lower.range(of: marker) else { continue }
            var item = String(sentence[range.upperBound...])
                .trimmingCharacters(in: CharacterSet(charactersIn: " ,:–—-"))
            if item.isEmpty { item = sentence }  // bare "action item" style labels
            item = String(item.prefix(120))
            if let first = item.first { item = first.uppercased() + item.dropFirst() }
            let key = item.lowercased()
            if item.count >= 3, !seen.contains(key) {
                seen.insert(key)
                out.append(item)
            }
            if out.count == 10 { break }
        }
        return out
    }
}

// ── The screen ───────────────────────────────────────────────────────────────

struct DictateView: View {
    private enum Phase: Equatable {
        case idle, recording, transcribing, review, saving
        case saved(project: String, cards: Int, todosInNote: Bool)
    }

    private struct ProposedTodo: Identifiable {
        let id = UUID()
        let text: String
        var included = true
    }

    @StateObject private var recorder = DictationRecorder()
    @State private var phase: Phase = .idle
    @State private var transcript = ""
    @State private var todos: [ProposedTodo] = []
    @State private var projects: [ProjectSummary] = []
    @State private var chosenProject: ProjectSummary?
    @State private var pickingProject = false
    @State private var errorText: String?
    @State private var micDenied = false
    @State private var recordTaps = 0
    @State private var savedCount = 0
    @Environment(\.openURL) private var openURL

    var body: some View {
        NavigationStack {
            Group {
                switch phase {
                case .idle, .recording, .transcribing: captureStage
                case .review, .saving: reviewStage
                case .saved(let project, let cards, let inNote):
                    savedStage(project: project, cards: cards, todosInNote: inNote)
                }
            }
            .background(Brand.shell)
            .navigationTitle("Dictate")
            .sensoryFeedback(.impact(weight: .medium), trigger: recordTaps)
            .sensoryFeedback(.success, trigger: savedCount)
        }
    }

    // ── Stage 1: capture ─────────────────────────────────────────────────────

    private var captureStage: some View {
        VStack(spacing: 26) {
            Spacer()

            ZStack {
                // The visible recording indicator (house rule): breathing rings,
                // not a slapped-on red dot. Honors Reduce Motion via the pulse.
                if phase == .recording {
                    RecordingPulse()
                }
                Button {
                    recordTaps += 1
                    if phase == .recording { recorder.stop() } else { startRecording() }
                } label: {
                    ZStack {
                        Circle()
                            .fill(Brand.ribbon)
                            .frame(width: 92, height: 92)
                            .shadow(color: Brand.cyanGlow, radius: phase == .recording ? 26 : 14)
                        Image(systemName: phase == .recording ? "stop.fill" : "mic.fill")
                            .font(.system(size: 30, weight: .semibold))
                            .foregroundStyle(Brand.deepVoid)
                            .contentTransition(.symbolEffect(.replace))
                    }
                }
                .buttonStyle(.plain)
                .disabled(phase == .transcribing)
                .accessibilityLabel(phase == .recording ? "Stop recording" : "Start recording")
            }
            .frame(height: 150)
            .animation(Motion.spring, value: phase)

            switch phase {
            case .recording:
                VStack(spacing: 10) {
                    HStack(spacing: 7) {
                        Circle().fill(Brand.cyan).frame(width: 7, height: 7)
                            .shadow(color: Brand.cyanGlow, radius: 5)
                        Text("RECORDING").font(.brandLabel).foregroundStyle(Brand.cyan)
                    }
                    Text(timeString(recorder.elapsed))
                        .font(.brandDisplay).monospacedDigit()
                        .contentTransition(.numericText())
                        .foregroundStyle(Brand.text)
                        .animation(Motion.ease, value: Int(recorder.elapsed))
                    LevelMeter(levels: recorder.levels)
                    Text("Tap stop when you're done — up to ten minutes.")
                        .font(.brandCaption).foregroundStyle(Brand.textDim)
                }
                .transition(.opacity)
            case .transcribing:
                VStack(spacing: 12) {
                    ThinkingDots()
                    Text("Transcribing on your hub…")
                        .font(.brandHeadline).foregroundStyle(Brand.text)
                    Text("Your Mac's local Whisper is listening back. Audio never leaves your machines.")
                        .font(.brandCaption).foregroundStyle(Brand.textMuted)
                        .multilineTextAlignment(.center).padding(.horizontal, 40)
                }
                .transition(.opacity)
            default:
                VStack(spacing: 8) {
                    Text("Speak a note").font(.brandTitle).foregroundStyle(Brand.text)
                    Text("Dictate on the phone; it lands as a note on a project you choose. Say things like \u{201C}I need to\u{2026}\u{201D} and Henry will propose them as to-dos.")
                        .font(.brandCaption).foregroundStyle(Brand.textMuted)
                        .multilineTextAlignment(.center).padding(.horizontal, 36)
                }
                .transition(.opacity)
            }

            if let errorText {
                GlassCard {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(errorText).font(.brandCaption).foregroundStyle(Brand.danger)
                        if micDenied {
                            Button("Open Settings") {
                                if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                            }
                            .font(.caption.weight(.semibold)).foregroundStyle(Brand.cyan)
                        }
                    }
                }
                .padding(.horizontal, 24)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }

            Spacer()
            Spacer()
        }
        .animation(Motion.ease, value: errorText)
    }

    // ── Stage 2: review + confirm ────────────────────────────────────────────

    private var reviewStage: some View {
        ScrollView {
            VStack(spacing: 14) {
                GlassCard {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("TRANSCRIPT").font(.brandLabel).foregroundStyle(Brand.textDim)
                        TextEditor(text: $transcript)
                            .font(.brandBody).foregroundStyle(Brand.text)
                            .scrollContentBackground(.hidden)
                            .frame(minHeight: 120, maxHeight: 240)
                        Text("Edit freely — this becomes the note.")
                            .font(.caption2).foregroundStyle(Brand.textDim)
                    }
                }

                if !todos.isEmpty {
                    GlassCard {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("PROPOSED TO-DOS").font(.brandLabel).foregroundStyle(Brand.cyan)
                            Text("Heard in your note. Only ticked items are created — as cards on the project board.")
                                .font(.caption2).foregroundStyle(Brand.textDim)
                            ForEach($todos) { $todo in
                                Button {
                                    withAnimation(Motion.ease) { todo.included.toggle() }
                                } label: {
                                    HStack(spacing: 10) {
                                        Image(systemName: todo.included ? "checkmark.circle.fill" : "circle")
                                            .foregroundStyle(todo.included ? Brand.cyan : Brand.textDim)
                                        Text(todo.text)
                                            .font(.brandCaption)
                                            .foregroundStyle(todo.included ? Brand.text : Brand.textMuted)
                                            .strikethrough(!todo.included, color: Brand.textDim)
                                            .multilineTextAlignment(.leading)
                                        Spacer()
                                    }
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }

                // Confirm-first: the note is written nowhere until a project is
                // chosen and Save is tapped.
                GlassCard {
                    Button { pickingProject = true } label: {
                        HStack(spacing: 10) {
                            Image(systemName: "folder.fill")
                                .foregroundStyle(chosenProject == nil ? Brand.textDim : Brand.cyan)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(chosenProject?.name ?? "Choose a project")
                                    .font(.brandHeadline)
                                    .foregroundStyle(chosenProject == nil ? Brand.textMuted : Brand.text)
                                Text(chosenProject.map { "/\($0.slug)" } ?? "Required — the note lands on its page")
                                    .font(.brandLabel).foregroundStyle(Brand.textDim)
                            }
                            Spacer()
                            Image(systemName: "chevron.up.chevron.down")
                                .font(.caption).foregroundStyle(Brand.textDim)
                        }
                    }
                    .buttonStyle(.plain)
                }

                if let errorText {
                    Text(errorText).font(.brandCaption).foregroundStyle(Brand.danger)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                PrimaryCTA(
                    title: phase == .saving ? "Saving…" : saveTitle,
                    systemImage: phase == .saving ? nil : "arrow.up.doc.fill",
                    enabled: canSave
                ) { save() }

                Button("Discard and start over") { reset() }
                    .font(.caption).foregroundStyle(Brand.textDim)
                    .padding(.top, 2)
            }
            .padding()
        }
        .scrollDismissesKeyboard(.interactively)
        .sheet(isPresented: $pickingProject) { projectPicker }
    }

    private var saveTitle: String {
        let n = todos.filter(\.included).count
        return n == 0 ? "Save note" : "Save note + \(n) to-do\(n == 1 ? "" : "s")"
    }

    private var canSave: Bool {
        phase == .review && chosenProject != nil &&
        !transcript.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var projectPicker: some View {
        NavigationStack {
            List {
                ForEach(projects) { p in
                    Button {
                        chosenProject = p
                        pickingProject = false
                    } label: {
                        HStack(spacing: 10) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(p.name).font(.brandHeadline).foregroundStyle(Brand.text)
                                if !p.description.isEmpty {
                                    Text(p.description).font(.brandCaption)
                                        .foregroundStyle(Brand.textMuted).lineLimit(1)
                                }
                            }
                            Spacer()
                            if chosenProject == p {
                                Image(systemName: "checkmark").foregroundStyle(Brand.cyan)
                            }
                        }
                    }
                    .listRowBackground(Color.clear)
                }
                if projects.isEmpty {
                    Text("No projects yet — create one on your desktop first.")
                        .font(.brandCaption).foregroundStyle(Brand.textMuted)
                        .listRowBackground(Color.clear)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Brand.shell)
            .navigationTitle("Project")
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.medium, .large])
        .presentationBackground(Brand.deepVoid)
    }

    // ── Stage 3: saved ───────────────────────────────────────────────────────

    private func savedStage(project: String, cards: Int, todosInNote: Bool) -> some View {
        VStack(spacing: 18) {
            Spacer()
            ZStack {
                Circle().fill(Brand.surface).frame(width: 84, height: 84)
                    .overlay(Circle().strokeBorder(Brand.borderHi, lineWidth: 1))
                Image(systemName: "checkmark")
                    .font(.system(size: 34, weight: .bold)).foregroundStyle(Brand.cyan)
                    .shadow(color: Brand.cyanGlow, radius: 8)
            }
            Text("Noted.").font(.brandTitle).foregroundStyle(Brand.text)
            VStack(spacing: 4) {
                Text("Saved to \(project) and indexed into the Brain.")
                if todosInNote {
                    Text("To-dos are in the note as a checklist — that project has no board columns yet.")
                } else if cards > 0 {
                    Text("\(cards) to-do\(cards == 1 ? "" : "s") added to the project board.")
                }
            }
            .font(.brandCaption).foregroundStyle(Brand.textMuted)
            .multilineTextAlignment(.center).padding(.horizontal, 40)
            PrimaryCTA(title: "Dictate another", systemImage: "mic.fill") { reset() }
                .padding(.horizontal, 24).padding(.top, 8)
            Spacer()
            Spacer()
        }
        .transition(.opacity)
    }

    // ── Flow ─────────────────────────────────────────────────────────────────

    private func startRecording() {
        errorText = nil
        micDenied = false
        Task {
            guard await recorder.requestPermission() == .granted else {
                micDenied = true
                errorText = "Microphone access is off for Permagent. Enable it in Settings to dictate."
                return
            }
            recorder.onFinish = { url in handleClip(url) }
            do {
                try recorder.start()
                withAnimation(Motion.spring) { phase = .recording }
            } catch {
                recorder.onFinish = nil
                errorText = "Couldn't start the microphone — close other recording apps and try again."
            }
        }
    }

    private func handleClip(_ url: URL?) {
        guard let url else {
            errorText = "Recording failed — nothing was captured."
            withAnimation(Motion.ease) { phase = .idle }
            return
        }
        withAnimation(Motion.ease) { phase = .transcribing }
        Task {
            defer { try? FileManager.default.removeItem(at: url) }
            do {
                let wav = try Data(contentsOf: url)
                let text = try await APIClient.shared.transcribe(wav: wav)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else {
                    errorText = "Didn't catch anything — try again a little closer to the mic."
                    withAnimation(Motion.ease) { phase = .idle }
                    return
                }
                transcript = text
                todos = TodoExtractor.propose(from: text).map { ProposedTodo(text: $0) }
                projects = (try? await APIClient.shared.projects()) ?? []
                withAnimation(Motion.spring) { phase = .review }
            } catch APIError.dictationUnavailable {
                errorText = "Your hub has no dictation model yet — open the desktop app once to set up local Whisper."
                withAnimation(Motion.ease) { phase = .idle }
            } catch {
                errorText = "Couldn't reach your hub — is your Mac awake and on the tailnet?"
                withAnimation(Motion.ease) { phase = .idle }
            }
        }
    }

    private func save() {
        guard let project = chosenProject, canSave else { return }
        errorText = nil
        phase = .saving
        Task {
            do {
                let confirmed = todos.filter(\.included).map(\.text)
                var body = transcript.trimmingCharacters(in: .whitespacesAndNewlines)
                var todosInNote = false

                // Card path needs a board; if the project has none, fold the
                // to-dos into the note itself rather than dropping them.
                if !confirmed.isEmpty {
                    let cols = (try? await APIClient.shared.columns(projectId: project.id)) ?? []
                    if cols.isEmpty {
                        todosInNote = true
                        body += "\n\n## To-dos\n" + confirmed.map { "- [ ] \($0)" }.joined(separator: "\n")
                    }
                }

                let title = "Dictated note — " + Date().formatted(date: .abbreviated, time: .shortened)
                let note = try await APIClient.shared.createNote(projectId: project.id, title: title, body: body)

                var created = 0
                var failedCards = 0
                if !todosInNote {
                    for item in confirmed {
                        do {
                            try await APIClient.shared.createCard(projectId: project.id, title: item, noteId: note.id)
                            created += 1
                        } catch { failedCards += 1 }
                    }
                }

                savedCount += 1
                withAnimation(Motion.spring) {
                    phase = .saved(project: project.name, cards: created, todosInNote: todosInNote)
                }
                if failedCards > 0 {
                    errorText = "\(failedCards) to-do\(failedCards == 1 ? "" : "s") couldn't be added to the board — they're still in the transcript."
                }
            } catch {
                errorText = "Couldn't save the note — check the hub and try again. Nothing was lost."
                phase = .review
            }
        }
    }

    private func reset() {
        recorder.cancel()
        transcript = ""
        todos = []
        chosenProject = nil
        errorText = nil
        micDenied = false
        withAnimation(Motion.ease) { phase = .idle }
    }

    private func timeString(_ t: TimeInterval) -> String {
        let s = Int(t)
        return String(format: "%d:%02d", s / 60, s % 60)
    }
}

// ── Recording visuals ────────────────────────────────────────────────────────

/// Two breathing rings around the record button — the unmistakable "live mic"
/// cue. Falls back to a steady high-visibility ring under Reduce Motion.
private struct RecordingPulse: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var breathing = false

    var body: some View {
        ZStack {
            ring(delay: 0)
            ring(delay: 0.55)
        }
        .onAppear { breathing = true }
        .accessibilityHidden(true)
    }

    private func ring(delay: Double) -> some View {
        Circle()
            .strokeBorder(Brand.cyan.opacity(reduceMotion ? 0.5 : (breathing ? 0.05 : 0.55)), lineWidth: 2)
            .frame(width: 92, height: 92)
            .scaleEffect(reduceMotion ? 1.28 : (breathing ? 1.55 : 1.05))
            .animation(
                reduceMotion ? nil : Motion.breath.repeatForever(autoreverses: false).delay(delay),
                value: breathing
            )
    }
}

/// Live mic-level meter: a scrolling row of capsules, cyan-bright with voice.
private struct LevelMeter: View {
    let levels: [CGFloat]

    var body: some View {
        HStack(alignment: .center, spacing: 3) {
            ForEach(Array(levels.enumerated()), id: \.offset) { _, level in
                Capsule()
                    .fill(Brand.cyan.opacity(0.25 + 0.75 * level))
                    .frame(width: 3, height: 6 + 30 * level)
            }
        }
        .frame(height: 40)
        .animation(Motion.ease, value: levels)
        .accessibilityLabel("Microphone level")
    }
}
