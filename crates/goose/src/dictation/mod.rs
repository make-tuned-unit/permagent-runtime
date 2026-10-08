pub mod providers;
#[cfg(feature = "local-inference")]
pub mod whisper;
#[cfg(feature = "local-inference")]
mod whisper_segments;

/// Self-knowledge descriptor for the meeting-notes surface (call-notes MVP 1A
/// and after): the sidebar Record button that captures meeting speech,
/// transcribes it in chunks on the LOCAL Whisper model (sovereignty ruling —
/// never a cloud STT), lets the user steer the write-up from a notepad while
/// the call runs, and saves the result as a project note whose action items
/// become kanban cards. Static: a UI surface with no cheaply queryable state.
///
/// This descriptor is the ONLY place the agent learns what the feature can do,
/// so it must keep pace with the surface. It was stale once already — it still
/// claimed "their own voice only" months after system-audio capture shipped,
/// which made the agent tell users the other side of a call could not be
/// recorded when it could.
pub const MEETING_DICTATION_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "meeting_dictation",
        display_name: "Meeting notes",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The sidebar Record picker binds a recording to a project. The user selects their \
             microphone, this Mac's system audio, or both before starting; opening the picker \
             never starts capture. Native system capture requires the helper and macOS permission. \
             Local Whisper transcribes captured audio in chunks. Source activity and a saved note \
             do not prove every spoken word was captured. The user can add a notepad during the call. \
             Saving retains the transcript and notes in the selected project, then queues durable \
             Orchestrator triage. Its receipt reports pending/running/completed/failed/skipped. \
             Completed triage can produce a structured write-up and relevant Kanban todos; saving \
             alone does not prove triage completed, and extracted todos are not execution approval. \
             The UI links the saved note, resulting board cards and triage session when available, \
             and supports retry and interrupted-draft recovery",
        why_it_matters:
            "Use app_action surface record action open to reveal the picker or existing recording \
             panel, then let the user confirm sources and Start. Explain which side is actually \
             selected; never promise both sides from a microphone-only recording. Audio transcription \
             stays local. Write-up and triage use configured privacy routing and may send transcript \
             and notepad text to a cloud model when permitted; local-only must not silently fall back \
             to cloud. Check the receipt before claiming action items exist. Open note is the saved \
             project artifact; Open board is available for created cards. Failed/skipped triage can \
             be retried without treating the recording as a new meeting",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Record the call",
                body: "Show them the Record button in the sidebar. Explain that they choose the \
                       project and sources first — nothing is captured until Start. Microphone \
                       and system audio are separate choices; macOS permission may be needed.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Steer the notes while it runs",
                body: "Point at the notepad in the recording panel. Tell them to type fragments \
                       as things come up — shorthand and typos are fine. Those fragments decide \
                       what the write-up covers; the transcript is captured either way, so an \
                       empty notepad costs nothing.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "See what it wrote",
                body: "After they stop, open the project's notes. The raw transcript is saved \
                       first. Check the triage receipt before claiming the write-up or cards \
                       completed. If cards were created, Open board reveals them; failed/skipped \
                       triage offers Retry triage. Keep the transcript available for review.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Projects",
                    section: None,
                }),
                confirm: None,
            },
        ],
    };
