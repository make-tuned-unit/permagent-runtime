// Meeting mode — "sit my phone on the table at a meeting and have it record
// it, so I come home to organised notes with to-dos as follow-ups."
//
// Deliberately a different room from quick-note dictation, because they are
// different jobs that have been wearing the same microphone icon: a quick note
// is seconds long and reviewed before it is filed; a meeting runs for an hour
// with the screen dark and files itself when it is done.
//
// What this screen owes the user, all of it learned from losing a real
// meeting on 2026-08-18:
//   • the elapsed time, always visible, and no silent ceiling;
//   • the truth about what is still only on this phone ("N waiting to send");
//   • an honest account of what leaves the device and what does not.

import SwiftUI
import UIKit

struct MeetingView: View {
    @ObservedObject private var capture = MeetingCapture.shared
    @ObservedObject private var store = RecordingStore.shared
    @ObservedObject private var uploader = MeetingUploader.shared
    @ObservedObject private var identity = AgentIdentity.shared
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL

    @State private var projects: [ProjectSummary] = []
    @State private var chosenProject: ProjectSummary?
    @State private var pickingProjectFor: PickerTarget?
    @State private var errorText: String?
    @State private var micDenied = false
    @State private var confirmingDelete: UUID?
    @State private var recordTaps = 0

    /// The project sheet serves two callers: arming a new recording, and
    /// giving a home to a recording that was stranded without one.
    private enum PickerTarget: Identifiable {
        case newRecording
        case existing(UUID)
        var id: String {
            switch self {
            case .newRecording: return "new"
            case .existing(let id): return id.uuidString
            }
        }
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                if capture.isRecording {
                    liveCard
                } else {
                    armCard
                }
                if let errorText {
                    errorCard(errorText)
                }
                if !store.unfinished.isEmpty {
                    queueSection
                }
                truthCard
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 18)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(ChatSurface.bg.ignoresSafeArea())
        .navigationTitle("Meeting")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $pickingProjectFor) { target in projectPicker(for: target) }
        .sensoryFeedback(.impact(weight: .medium), trigger: recordTaps)
        .task { await load() }
        // Keep trying while this screen is open. The queue also drains at
        // launch and whenever a segment lands, so a meeting recorded with the
        // hub asleep sends itself the moment the Mac wakes up.
        .task {
            while !Task.isCancelled {
                uploader.requestDrain()
                try? await Task.sleep(nanoseconds: 20 * NSEC_PER_SEC)
            }
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { uploader.requestDrain() }
        }
        .confirmationDialog(
            "Delete this recording and its audio?",
            isPresented: Binding(get: { confirmingDelete != nil }, set: { if !$0 { confirmingDelete = nil } }),
            titleVisibility: .visible
        ) {
            Button("Delete permanently", role: .destructive) {
                if let id = confirmingDelete { store.remove(id) }
                confirmingDelete = nil
            }
            Button("Keep it", role: .cancel) { confirmingDelete = nil }
        } message: {
            Text("The audio is on this phone and nowhere else. This cannot be undone.")
        }
    }

    // ── Arm ──────────────────────────────────────────────────────────────────

    private var armCard: some View {
        RaisedCard {
            Text("RECORD A MEETING")
                .font(.brandLabel).tracking(0.88).foregroundStyle(ChatSurface.spark)
            Text("Phone flat on the table. It keeps recording with the screen locked, for as long as the meeting runs.")
                .font(.brandCaption).foregroundStyle(ChatSurface.muted)

            Button { pickingProjectFor = .newRecording } label: {
                HStack(spacing: 12) {
                    Image(systemName: "folder")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(chosenProject == nil ? ChatSurface.dim : ChatSurface.spark)
                        .frame(width: 34, height: 34)
                        .background(ChatSurface.control, in: Circle())
                    VStack(alignment: .leading, spacing: 2) {
                        Text(chosenProject?.name ?? "Choose a project")
                            .font(.brandHeadline)
                            .foregroundStyle(chosenProject == nil ? ChatSurface.muted : ChatSurface.text)
                        // Picked BEFORE recording on purpose: a meeting that
                        // survives an app kill already knows where it belongs.
                        Text(chosenProject.map { "The notes land on /\($0.slug)" }
                             ?? "Chosen up front, so a recovered recording still knows where to go")
                            .font(.brandLabel).foregroundStyle(ChatSurface.dim)
                    }
                    Spacer(minLength: 6)
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.caption).foregroundStyle(ChatSurface.dim)
                }
            }
            .buttonStyle(.plain)

            if let free = MeetingCapture.freeMegabytes(), free < 600 {
                Text("Only \(Int(free)) MB free on this phone. Audio waiting to send takes about \(Int(MeetingCapture.megabytesPerHour)) MB an hour.")
                    .font(.brandCaption).foregroundStyle(Brand.warning)
            }

            SparkCTA(title: "Start recording", systemImage: "record.circle", enabled: chosenProject != nil) {
                recordTaps += 1
                startRecording()
            }
            if chosenProject == nil {
                Text("Pick a project first — that is where the notes and follow-ups go.")
                    .font(.caption2).foregroundStyle(ChatSurface.dim)
            }
        }
    }

    // ── Live ─────────────────────────────────────────────────────────────────

    private var liveCard: some View {
        RaisedCard {
            HStack(spacing: 8) {
                Circle()
                    .fill(capture.interrupted ? Brand.warning : Brand.danger)
                    .frame(width: 9, height: 9)
                Text(capture.interrupted ? "PAUSED" : "RECORDING")
                    .font(.brandLabel).tracking(0.88)
                    .foregroundStyle(capture.interrupted ? Brand.warning : ChatSurface.text)
                Spacer()
                Text(liveProjectName ?? "")
                    .font(.brandLabel).foregroundStyle(ChatSurface.dim)
            }

            // Elapsed time, always on screen. The lost meeting stopped at ten
            // minutes without saying so; this screen can never be silent about
            // how long it has been recording.
            Text(CaptureText.elapsed(capture.elapsed))
                .font(.manrope(44)).monospacedDigit()
                .contentTransition(.numericText())
                .foregroundStyle(ChatSurface.text)
                .frame(maxWidth: .infinity, alignment: .leading)
                .animation(Motion.ease, value: Int(capture.elapsed))

            LevelMeter(levels: capture.levels)

            Text(progressLine)
                .font(.brandCaption).foregroundStyle(ChatSurface.muted)

            Text("You can lock the phone and put it down. There is no time limit — the recording is cut into \(Int(MeetingCapture.segmentSeconds / 60))-minute pieces and each one is sent as it finishes.")
                .font(.caption2).foregroundStyle(ChatSurface.dim)

            if let problem = capture.lastProblem {
                Text(problem).font(.brandCaption).foregroundStyle(Brand.warning)
            }

            SparkCTA(title: "Stop and write it up", systemImage: "stop.fill") {
                recordTaps += 1
                capture.stop()
                uploader.requestDrain()
            }
        }
    }

    /// Where the recording in progress is headed, read from the store rather
    /// than from view state — the recorder outlives this screen, so the label
    /// has to come from the same place the recording does.
    private var liveProjectName: String? {
        capture.recordingId.flatMap { store.recording($0)?.projectName }
    }

    private var progressLine: String {
        let captured = capture.segmentsCaptured
        let waiting = store.segmentsWaiting
        if captured == 0 { return "First piece is still recording." }
        if waiting == 0 { return "\(captured) piece\(captured == 1 ? "" : "s") captured — all of it is on your Mac." }
        return "\(captured) piece\(captured == 1 ? "" : "s") captured, \(waiting) still waiting to send."
    }

    // ── The queue ────────────────────────────────────────────────────────────

    private var queueSection: some View {
        VStack(spacing: 12) {
            HStack {
                Text(waitingHeadline)
                    .font(.brandHeading).foregroundStyle(ChatSurface.text)
                Spacer()
                if uploader.isDraining {
                    ThinkingDots()
                } else {
                    Button("Send now") { uploader.requestDrain() }
                        .font(.caption.weight(.semibold)).foregroundStyle(ChatSurface.spark)
                }
            }
            .padding(.horizontal, 4)

            if let error = uploader.lastError {
                Text(error).font(.brandCaption).foregroundStyle(Brand.warning)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 4)
            }

            ForEach(store.unfinished) { recording in
                queueRow(recording)
            }
        }
    }

    private var waitingHeadline: String {
        let n = store.recordingsWaiting
        if n == 0 { return "Finishing up" }
        return "\(n) recording\(n == 1 ? "" : "s") waiting to send"
    }

    private func queueRow(_ recording: CapturedRecording) -> some View {
        RaisedCard {
            HStack(spacing: 8) {
                Text(recording.kind == .meeting ? "Meeting" : "Note")
                    .font(.brandLabel).tracking(0.88).foregroundStyle(ChatSurface.spark)
                Text(recording.startedAt.formatted(date: .abbreviated, time: .shortened))
                    .font(.brandLabel).foregroundStyle(ChatSurface.dim)
                Spacer()
                Text(CaptureText.elapsed(recording.duration))
                    .font(.brandLabel).monospacedDigit().foregroundStyle(ChatSurface.muted)
            }

            Text(statusLine(for: recording))
                .font(.brandCaption).foregroundStyle(ChatSurface.muted)

            if recording.segments.count > 0 {
                ProgressView(value: Double(recording.sentCount), total: Double(recording.segments.count))
                    .tint(ChatSurface.spark)
            }

            if let problem = recording.lastError {
                Text(problem).font(.caption2).foregroundStyle(Brand.warning)
            }

            HStack(spacing: 14) {
                if recording.projectId == nil {
                    Button("Choose a project") { pickingProjectFor = .existing(recording.id) }
                        .font(.caption.weight(.semibold)).foregroundStyle(ChatSurface.spark)
                }
                if recording.waitingCount > 0 && recording.hasWords {
                    // The desktop recorder's honesty, as a button: the part
                    // that transcribed survived — file it as the note.
                    Button("File what transcribed") {
                        Task { await uploader.fileWhatTranscribed(recordingId: recording.id) }
                    }
                    .font(.caption.weight(.semibold)).foregroundStyle(ChatSurface.spark)
                }
                Spacer()
                Button("Delete") { confirmingDelete = recording.id }
                    .font(.caption.weight(.semibold)).foregroundStyle(Brand.danger)
            }
        }
    }

    private func statusLine(for recording: CapturedRecording) -> String {
        if recording.savedNoteId != nil {
            let held = recording.segments.filter(\.holdsAudio).count
            return held == 0
                ? "Filed."
                : "Filed without \(held) piece\(held == 1 ? "" : "s") that never transcribed — their audio is still here."
        }
        if recording.projectId == nil {
            return "Transcribed, but it has nowhere to go yet — pick a project and it files itself."
        }
        if recording.waitingCount > 0 {
            let waiting = recording.waitingCount
            return "\(recording.sentCount) of \(recording.segments.count) pieces on your Mac. \(waiting) piece\(waiting == 1 ? " is" : "s are") still only on this phone."
        }
        return "All of it is on your Mac. Writing the note…"
    }

    // ── The truth card ───────────────────────────────────────────────────────

    private var truthCard: some View {
        RaisedCard {
            Text("WHAT ACTUALLY HAPPENS")
                .font(.brandLabel).tracking(0.88).foregroundStyle(ChatSurface.dim)
            Text("The microphone records this phone's surroundings — your side of the room, not a phone call. Nothing on this screen captures a call.")
                .font(.brandCaption).foregroundStyle(ChatSurface.muted)
            Text("Audio is written to this phone first and kept there until your Mac confirms it has the words. It is transcribed by Whisper running on your own Mac.")
                .font(.brandCaption).foregroundStyle(ChatSurface.muted)
            // PR #1027 exists because a blanket "nothing goes to a cloud
            // service" claim was false for this pass. Say exactly what is true.
            Text("Turning the transcript into organised notes and follow-ups is done by \(identity.name) on your hub, using whichever model the hub is set to use for meeting write-ups. That is a cloud model unless you have set write-ups to local only on the desktop.")
                .font(.brandCaption).foregroundStyle(ChatSurface.muted)
            // The honest version of the storage number: what it costs depends
            // entirely on whether the Mac is there to take it.
            Text("Each piece is deleted from this phone as soon as your Mac confirms it, so a normal recording holds about two minutes of audio at a time. If your Mac is unreachable the whole way through, the audio stays here — about \(Int(MeetingCapture.megabytesPerHour)) MB an hour — until it can send.")
                .font(.caption2).foregroundStyle(ChatSurface.dim)
        }
    }

    private func errorCard(_ text: String) -> some View {
        RaisedCard {
            Text(text).font(.brandCaption).foregroundStyle(Brand.danger)
            if micDenied {
                Button("Open Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                }
                .font(.caption.weight(.semibold)).foregroundStyle(ChatSurface.spark)
            }
        }
    }

    // ── Project picker ───────────────────────────────────────────────────────

    private func projectPicker(for target: PickerTarget) -> some View {
        NavigationStack {
            List {
                ForEach(projects) { p in
                    Button {
                        switch target {
                        case .newRecording:
                            chosenProject = p
                        case .existing(let id):
                            uploader.setTargetAndDrain(recordingId: id, projectId: p.id, projectName: p.name)
                        }
                        pickingProjectFor = nil
                    } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(p.name).font(.brandHeadline).foregroundStyle(ChatSurface.text)
                                Text("/\(p.slug)").font(.brandCaption).foregroundStyle(ChatSurface.muted)
                            }
                            Spacer()
                            if case .newRecording = target, chosenProject == p {
                                Image(systemName: "checkmark").foregroundStyle(ChatSurface.spark)
                            }
                        }
                    }
                    .listRowBackground(Color.clear)
                }
                if projects.isEmpty {
                    Text("No projects yet — create one on your desktop first.")
                        .font(.brandCaption).foregroundStyle(ChatSurface.muted)
                        .listRowBackground(Color.clear)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(ChatSurface.bg.ignoresSafeArea())
            .navigationTitle("Project")
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.medium, .large])
        .presentationBackground(ChatSurface.bg)
    }

    // ── Flow ─────────────────────────────────────────────────────────────────

    private func load() async {
        projects = (try? await APIClient.shared.projects()) ?? []
        capture.onSegmentReady = { MeetingUploader.shared.requestDrain() }
        uploader.requestDrain()
    }

    private func startRecording() {
        errorText = nil
        micDenied = false
        guard let project = chosenProject else { return }
        Task {
            guard await capture.requestPermission() == .granted else {
                micDenied = true
                errorText = "Microphone access is off for Permagent. Turn it on in Settings to record a meeting."
                return
            }
            do {
                try capture.start(projectId: project.id, projectName: project.name)
            } catch {
                errorText = "Couldn't start the microphone — close other recording apps and try again."
            }
        }
    }
}
