// Notes + voice dictation (#2) — the killer mobile surface: jot or *speak* a
// note to a project from your pocket. Reads/writes GET/POST
// /api/projects/{id}/notes, and dictation records mic audio natively
// (AVAudioRecorder → 16 kHz mono WAV) and POSTs it to
// /api/dictation/transcribe, appending the returned text to the note body.
//
// The note is saved on the hub and indexed into the Brain server-side (the POST
// handler does that) — nothing but the pairing token lives on this device.
//
// REQUIRES on the target (mini, at Xcode project setup): an Info.plist
// `NSMicrophoneUsageDescription` string, or the app crashes on first mic use.

import SwiftUI
import AVFoundation

// ── Wire models ──────────────────────────────────────────────────────────────

/// A project from GET /api/projects. That response is camelCase-encoded on the
/// server (ProjectResponse uses `rename_all = "camelCase"`), so these property
/// names are already the wire keys — no CodingKeys needed. Only the fields the
/// picker needs are decoded.
struct Project: Decodable, Identifiable {
    let id: String
    let name: String
    let status: String
}

/// A note from GET /api/projects/{id}/notes. ProjectNote is snake_case on the
/// server (plain `Serialize`, no rename), so these snake_case names match the
/// wire exactly. Extra fields (project_id, memory_key, updated_at) are ignored.
struct ProjectNote: Decodable, Identifiable {
    let id: String
    let title: String?
    let body: String
    let created_at: String
}

// ── Notes screen ─────────────────────────────────────────────────────────────

struct NotesView: View {
    @ObservedObject private var identity = AgentIdentity.shared
    @State private var projects: [Project] = []
    @State private var selected: Project?
    @State private var notes: [ProjectNote] = []
    @State private var projectsLoaded = false
    @State private var notesLoaded = false
    @State private var errorText: String?
    @State private var showComposer = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                projectPicker

                if let errorText {
                    HubErrorCard(text: errorText) { await loadProjects() }
                }

                notesSection
            }
            .padding()
        }
        .background { AppBackdrop() }
        .navigationTitle("Notes")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    showComposer = true
                } label: {
                    Image(systemName: "square.and.pencil")
                }
                .disabled(selected == nil)
            }
        }
        .refreshable { await loadNotes() }
        .task { await loadProjects() }
        .sheet(isPresented: $showComposer) {
            if let selected {
                NoteComposer(project: selected) { await loadNotes() }
            }
        }
    }

    private var projectPicker: some View {
        Menu {
            ForEach(projects) { p in
                Button {
                    Task { await select(p) }
                } label: {
                    if p.id == selected?.id {
                        Label(p.name, systemImage: "checkmark")
                    } else {
                        Text(p.name)
                    }
                }
            }
        } label: {
            HStack(spacing: 12) {
                Image(systemName: "folder")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(ChatSurface.spark)
                    .frame(width: 34, height: 34)
                    .background(ChatSurface.control, in: Circle())
                Text(selected?.name ?? (projectsLoaded ? "Select a project" : "Loading…"))
                    .font(.brandHeadline)
                    .foregroundStyle(ChatSurface.text)
                Spacer()
                Image(systemName: "chevron.up.chevron.down")
                    .font(.caption)
                    .foregroundStyle(ChatSurface.dim)
            }
            .padding(14)
            .background(ChatSurface.raised, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 24, style: .continuous)
                    .strokeBorder(ChatSurface.border, lineWidth: 1)
            )
        }
        .disabled(projects.isEmpty)
    }

    @ViewBuilder
    private var notesSection: some View {
        if selected == nil && projectsLoaded && projects.isEmpty {
            SparkEmptyState(
                line: "No projects yet.",
                caption: "Create one on the desktop (or ask \(identity.name)), then jot or dictate notes to it from here."
            )
            .padding(.top, 60)
        } else if !notesLoaded && selected != nil {
            ProgressView()
                .tint(ChatSurface.spark)
                .frame(maxWidth: .infinity)
                .padding(.top, 30)
        } else if notesLoaded && notes.isEmpty {
            SparkEmptyState(
                line: "No notes yet.",
                caption: "Tap the compose button to write — or dictate — your first note for this project."
            )
            .padding(.top, 60)
        } else {
            ForEach(notes) { note in
                noteCard(note)
            }
        }
    }

    private func noteCard(_ note: ProjectNote) -> some View {
        RaisedCard {
            VStack(alignment: .leading, spacing: 6) {
                if let title = note.title, !title.isEmpty {
                    Text(title)
                        .font(.brandHeadline)
                        .foregroundStyle(ChatSurface.text)
                }
                // A note's body is reading prose — the serif surface.
                Text(note.body)
                    .font(.chatProse)
                    .lineSpacing(4)
                    .foregroundStyle(ChatSurface.text)
                    .lineLimit(8)
                if let when = RelativeTime.string(from: note.created_at) {
                    Text(when)
                        .font(.caption2)
                        .foregroundStyle(ChatSurface.dim)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func loadProjects() async {
        do {
            let list = try await APIClient.shared.get("/api/projects", as: [Project].self)
            projects = list
            errorText = nil
            projectsLoaded = true
            if selected == nil, let first = list.first {
                await select(first)
            }
        } catch {
            errorText = "Couldn't load your projects — is your Mac awake and on the tailnet?"
            projectsLoaded = true
        }
    }

    private func select(_ project: Project) async {
        selected = project
        notes = []
        notesLoaded = false
        await loadNotes()
    }

    private func loadNotes() async {
        guard let selected else { return }
        do {
            notes = try await APIClient.shared.get("/api/projects/\(selected.id)/notes", as: [ProjectNote].self)
            errorText = nil
        } catch {
            errorText = "Couldn't load notes for \(selected.name)."
        }
        notesLoaded = true
    }
}

// ── Composer (sheet) ─────────────────────────────────────────────────────────
// Presented modally, so it owns its own NavigationStack (unlike the pushed
// control screens). Type OR dictate; Save POSTs the note and refreshes the list.

struct NoteComposer: View {
    let project: Project
    let onSaved: () async -> Void

    @Environment(\.dismiss) private var dismiss
    @StateObject private var recorder = NoteDictationRecorder()
    @State private var title = ""
    @State private var text = ""          // note body — NOT named `body` (that's View.body)
    @State private var saving = false
    @State private var transcribing = false
    @State private var errorText: String?

    private var canSave: Bool {
        !saving && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    TextField("Title (optional)", text: $title)
                        .font(.brandHeadline)
                        .foregroundStyle(ChatSurface.text)
                        .tint(ChatSurface.spark)
                        .padding(14)
                        .background(ChatSurface.raised, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                        .overlay(
                            RoundedRectangle(cornerRadius: 24, style: .continuous)
                                .strokeBorder(ChatSurface.border, lineWidth: 1)
                        )

                    ZStack(alignment: .topLeading) {
                        if text.isEmpty {
                            Text("Write your note…")
                                .font(.chatProse)
                                .foregroundStyle(ChatSurface.dim)
                                .padding(.horizontal, 18)
                                .padding(.vertical, 22)
                                .allowsHitTesting(false)
                        }
                        // The note is written the way it will be read — serif.
                        TextEditor(text: $text)
                            .font(.chatProse)
                            .foregroundStyle(ChatSurface.text)
                            .tint(ChatSurface.spark)
                            .scrollContentBackground(.hidden)
                            .frame(minHeight: 160)
                            .padding(8)
                    }
                    .background(ChatSurface.raised, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 24, style: .continuous)
                            .strokeBorder(ChatSurface.border, lineWidth: 1)
                    )

                    dictateBar

                    if let errorText {
                        Text(errorText)
                            .font(.brandCaption)
                            .foregroundStyle(Brand.danger)
                    }
                }
                .padding()
            }
            .background(ChatSurface.bg.ignoresSafeArea())
            .navigationTitle("New note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .foregroundStyle(ChatSurface.muted)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        save()
                    } label: {
                        Text(saving ? "Saving…" : "Save").font(.body.weight(.semibold))
                    }
                    .disabled(!canSave)
                }
            }
            .sensoryFeedback(.impact(weight: .light), trigger: recorder.isRecording)
        }
    }

    private var dictateBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                Task { await toggleDictation() }
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: recorder.isRecording ? "stop.fill" : "mic")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(recorder.isRecording ? Brand.onDanger : ChatSurface.text)
                        .frame(width: 34, height: 34)
                        .background(recorder.isRecording ? AnyShapeStyle(Brand.danger) : AnyShapeStyle(ChatSurface.control), in: Circle())
                    Text(recorder.isRecording
                         ? "Recording… tap to stop"
                         : (transcribing ? "Transcribing…" : "Dictate a note"))
                        .font(.brandCaption)
                        .foregroundStyle(ChatSurface.muted)
                    Spacer()
                    if transcribing {
                        ProgressView().tint(ChatSurface.spark)
                    } else if recorder.isRecording {
                        PulseDot(color: Brand.danger)
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(ChatSurface.raised, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 24, style: .continuous)
                        .strokeBorder(recorder.isRecording ? Brand.danger.opacity(0.5) : ChatSurface.border, lineWidth: 1)
                )
            }
            .buttonStyle(.plain)
            .disabled(transcribing)

            if recorder.permissionDenied {
                Text("Microphone access is off — enable it in Settings › Permagent to dictate.")
                    .font(.caption2)
                    .foregroundStyle(Brand.warning)
            }
        }
    }

    private func toggleDictation() async {
        if recorder.isRecording {
            guard let audio = recorder.stop() else { return }
            transcribing = true
            errorText = nil
            do {
                let transcript = try await APIClient.shared.transcribe(audio)
                let clean = transcript.trimmingCharacters(in: .whitespacesAndNewlines)
                if clean.isEmpty {
                    errorText = "Didn't catch that — try again, a little closer to the mic."
                } else {
                    if !text.isEmpty && !text.hasSuffix(" ") && !text.hasSuffix("\n") { text += " " }
                    text += clean
                }
            } catch APIError.badStatus(503) {
                errorText = "No dictation model is set up on your hub yet — configure one on the desktop."
            } catch {
                errorText = "Transcription failed — check your hub and try again."
            }
            transcribing = false
        } else {
            await recorder.start()
        }
    }

    private func save() {
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let b = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !b.isEmpty, !saving else { return }
        saving = true
        errorText = nil
        Task {
            struct NewNote: Encodable { let title: String?; let body: String }
            do {
                _ = try await APIClient.shared.post(
                    "/api/projects/\(project.id)/notes",
                    body: NewNote(title: t.isEmpty ? nil : t, body: b),
                    as: ProjectNote.self
                )
                await onSaved()
                dismiss()
            } catch {
                errorText = "Couldn't save the note — is your hub reachable?"
                saving = false
            }
        }
    }
}

// ── Native mic capture ───────────────────────────────────────────────────────
// Records to a 16 kHz mono 16-bit WAV (what the local Whisper model decodes
// cleanly — matching the daemon's note that clips are "encoded as WAV so they
// decode on any build"). A class so AVAudioRecorder is retained for the whole
// take; @MainActor so the @Published flags drive the composer directly.

@MainActor
final class NoteDictationRecorder: ObservableObject {
    @Published var isRecording = false
    @Published var permissionDenied = false

    private var recorder: AVAudioRecorder?
    private var fileURL: URL?

    func start() async {
        let granted = await Self.requestPermission()
        guard granted else {
            permissionDenied = true
            return
        }
        permissionDenied = false
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.record, mode: .spokenAudio)
            try session.setActive(true)

            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent("dictation-\(UUID().uuidString).wav")
            let settings: [String: Any] = [
                AVFormatIDKey: Int(kAudioFormatLinearPCM),
                AVSampleRateKey: 16_000.0,
                AVNumberOfChannelsKey: 1,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false,
                AVLinearPCMIsBigEndianKey: false,
            ]
            let rec = try AVAudioRecorder(url: url, settings: settings)
            guard rec.record() else {
                isRecording = false
                return
            }
            recorder = rec
            fileURL = url
            isRecording = true
        } catch {
            isRecording = false
        }
    }

    /// Stops the take and returns the recorded WAV bytes (nil if nothing was
    /// captured). Cleans up the temp file and deactivates the audio session.
    func stop() -> Data? {
        defer {
            recorder = nil
            fileURL = nil
            isRecording = false
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
        guard let recorder, let fileURL else { return nil }
        recorder.stop()
        let data = try? Data(contentsOf: fileURL)
        try? FileManager.default.removeItem(at: fileURL)
        return data
    }

    private static func requestPermission() async -> Bool {
        await withCheckedContinuation { continuation in
            AVAudioApplication.requestRecordPermission { granted in
                continuation.resume(returning: granted)
            }
        }
    }
}
