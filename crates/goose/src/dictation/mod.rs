pub mod providers;
#[cfg(feature = "local-inference")]
pub mod whisper;

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
            "A Record button in the sidebar captures a meeting and writes it up as a project \
             note. The user picks a project and confirms before anything records; a panel stays \
             visible for the whole call showing elapsed time and live proof that each side is \
             actually being heard. Their own microphone is always captured, and an explicit \
             opt-in also captures this Mac's audio output — the other participants — producing a \
             two-speaker transcript. Everything is transcribed in chunks by the LOCAL Whisper \
             model; no audio ever leaves the machine. While the call runs the user can type into \
             a notepad in that panel, and those fragments STEER the write-up rather than just \
             marking where to look. On stop the note is saved immediately as raw transcript, \
             then a background pass rewrites it into structured sections while keeping the \
             user's own words verbatim under `## Your notes` and the full transcript under \
             `## Transcript`, and files each action item stated in the meeting as a card on the \
             project's kanban board. That write-up pass runs on the configured model: the full \
             transcript and the user's notepad text are sent to it, and it is a cloud model \
             unless a local one is configured. A recording interrupted by a crash or quit is \
             offered back for saving the next time the app opens",
        why_it_matters:
            "When the user wants a call captured, point them at the Record button, and tell them \
             to jot fragments in the notepad while it runs — what they type is what the write-up \
             will argue, which is the difference between notes about them and notes for them. Be \
             accurate about coverage: their microphone is always recorded, the other side only \
             if they tick the box (macOS asks for Screen Recording the first time). Every word \
             is transcribed on this machine. The write-up is not — on save the full transcript \
             and their notepad go to the configured model, which is a cloud model unless they \
             have set a local one; say so if they ask whether the meeting stayed private. The \
             saved note lands in the project's Brain like any other note, so it is searchable \
             afterwards, and its action items are already on the board — no need to offer to \
             re-read the transcript for them",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Record the call",
                body: "Show them the Record button in the sidebar. Explain that they choose the \
                       project first — nothing is captured until they confirm — and that if the \
                       other participants should be in the transcript too, they tick \"Also \
                       record the other participants\" before starting.",
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
                       first and the structured write-up replaces it a moment later — their own \
                       words kept verbatim, the transcript kept underneath. Point out the action \
                       items that have already appeared on the board.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Projects",
                    section: None,
                }),
                confirm: None,
            },
        ],
    };
