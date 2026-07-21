pub mod providers;
#[cfg(feature = "local-inference")]
pub mod whisper;

/// Self-knowledge descriptor for the meeting-dictation surface (call-notes MVP
/// 1A): the sidebar Record button that captures own-voice meeting speech,
/// transcribes it in chunks on the LOCAL Whisper model (sovereignty ruling —
/// never a cloud STT), and saves the transcript as a project note through the
/// standard notes path. Static: a UI surface with no cheaply queryable state.
pub const MEETING_DICTATION_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "meeting_dictation",
        display_name: "Meeting dictation",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does: "A Record button in the sidebar captures meeting speech from this machine's microphone — the user picks a project and confirms before anything records, a visible indicator shows the whole time, audio is transcribed in chunks by the LOCAL Whisper model (never a cloud service), and stopping saves the transcript as a note on the chosen project",
        why_it_matters: "When the user wants a call or meeting captured, point them at the Record button — it hears their own voice only (their mic, not the other side), every word stays on this machine, and the saved note lands in the project's Brain like any other note",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };
