//! Versioned recognition evidence contracts.
//!
//! This module is deliberately a pure boundary over the existing
//! `recognition_events` / `recognition_set_members` instrumentation. It does
//! not create a second memory store, infer operator identity, or activate the
//! ambient `StreamTracker`. Observation facts and model interpretations remain
//! different enum variants, and incomplete metadata remains explicitly
//! partial instead of being filled from a placeholder persona.

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::recognition::{PendingRecognition, ProviderInvocationSeen, RecognitionSeen};
use crate::recognition_consent::{ambient_cue_allowed_with, RecognitionConsentConfig};

pub const RECOGNITION_CONTRACT_VERSION: &str = "recognition-contract.v1";

/// Evidence stage. Retrieval, selection, injection, action, and verification
/// are separate facts; one stage never implies a later stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStage {
    Retrieved,
    Selected,
    Injected,
    Acted,
    Verified,
}

/// What kind of claim is being represented. Only `Observation` is an event
/// fact by default; interpretation/preference/hypothesis/lesson require
/// stronger provenance and remain visibly distinct in serialized data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionKind {
    Observation,
    Interpretation,
    Preference,
    Hypothesis,
    VerifiedLesson,
}

/// Query instrumentation and ambient capture have different consent rules.
/// Query mode describes the existing recall observer; ambient mode is the
/// opt-in path governed by `recognition_consent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    Query,
    Ambient,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureBoundary {
    pub mode: CaptureMode,
    /// Existing activity capture may have recorded an event independently of
    /// recognition consent. Keeping this bit separate prevents an activity
    /// row from being misreported as ambient recognition consent.
    pub activity_recorded: bool,
    /// Capture-time policy metadata for audit/display only. It is never used
    /// as current authorization during validation; callers must provide the
    /// trusted live config to `RecognitionRecord::validate_with_current_consent`.
    pub consent: Option<RecognitionConsentConfig>,
    pub wing: Option<String>,
    pub source_surface: Option<String>,
}

impl CaptureBoundary {
    pub fn query() -> Self {
        Self {
            mode: CaptureMode::Query,
            activity_recorded: false,
            consent: None,
            wing: None,
            source_surface: None,
        }
    }

    pub fn ambient(
        consent: RecognitionConsentConfig,
        wing: Option<String>,
        source_surface: Option<String>,
    ) -> Self {
        Self {
            mode: CaptureMode::Ambient,
            activity_recorded: false,
            consent: Some(consent),
            wing,
            source_surface,
        }
    }

    pub fn with_activity_recorded(mut self, recorded: bool) -> Self {
        self.activity_recorded = recorded;
        self
    }

    /// Whether this boundary is the non-consent query path. Ambient capture
    /// cannot be admitted from serialized data alone.
    pub fn allows_recognition(&self) -> bool {
        matches!(self.mode, CaptureMode::Query)
    }

    /// The contract-level admission decision using a trusted current consent
    /// config. The activity journal's capture policy and serialized snapshot
    /// are intentionally not treated as recognition authorization.
    pub fn allows_recognition_with_current_consent(
        &self,
        current_consent: &RecognitionConsentConfig,
    ) -> bool {
        match self.mode {
            CaptureMode::Query => true,
            CaptureMode::Ambient => {
                let (Some(wing), Some(source)) =
                    (self.wing.as_deref(), self.source_surface.as_deref())
                else {
                    return false;
                };
                ambient_cue_allowed_with(current_consent, Some(wing), source)
            }
        }
    }
}

/// All IDs that can join a recognition fact back to existing Permagent and
/// Spectral records. Optional fields are intentionally not synthesized.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionProvenance {
    pub contract_version: String,
    pub source: String,
    pub source_event_id: Option<String>,
    pub observed_at: String,
    pub operator_scope: Option<String>,
    pub device_id: Option<String>,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub turn_index: Option<u64>,
    pub physical_invocation_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub model_version: Option<String>,
    pub contribution_id: Option<String>,
    pub retrieval_id: Option<String>,
    pub memory_id: Option<String>,
    pub memory_version: Option<String>,
    pub action_id: Option<String>,
    pub expected_outcome: Option<String>,
    pub observed_outcome: Option<String>,
    pub derived_from: Vec<String>,
}

/// A bounded confidence estimate. The score is never authoritative without a
/// declared basis and timestamp; `None` means no estimate was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Uncertainty {
    pub score: Option<f64>,
    pub basis: Option<String>,
    pub as_of: Option<String>,
}

impl Default for Uncertainty {
    fn default() -> Self {
        Self {
            score: None,
            basis: None,
            as_of: None,
        }
    }
}

/// Explicit user/environment correction. `revision` is the causal ordering
/// token; wall-clock timestamps are evidence, not the ordering authority.
/// `target_id` must equal the record's non-empty `memory_id` when present,
/// otherwise the record `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionProvenance {
    pub correction_id: String,
    pub target_id: String,
    pub supersedes: Vec<String>,
    pub source_event_id: String,
    pub operator_scope: Option<String>,
    pub device_id: Option<String>,
    pub revision: u64,
    pub observed_at: String,
}

/// A single linked recognition fact. This is an in-process contract fixture;
/// persistence remains owned by the existing recognition instrumentation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecognitionRecord {
    pub id: String,
    pub kind: RecognitionKind,
    pub stage: EvidenceStage,
    pub provenance: RecognitionProvenance,
    pub capture: CaptureBoundary,
    pub uncertainty: Uncertainty,
    pub correction: Option<CorrectionProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingMetadata {
    OperatorScope,
    SessionId,
    PhysicalInvocationId,
    Provider,
    Model,
    ModelVersion,
    ContributionId,
    RetrievalId,
    MemoryId,
    MemoryVersion,
    ExpectedOutcome,
    ObservedOutcome,
    CorrectionProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractStatus {
    Complete,
    Partial(Vec<MissingMetadata>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    InvalidContractVersion,
    MissingIdentity,
    InvalidSource,
    InvalidObservedAt,
    InvalidUncertainty,
    AmbientConsentRequired,
    InvalidCorrection,
    VerifiedLessonStageRequired,
    VerifiedLessonEvidenceRequired,
}

fn nonempty(value: Option<&String>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn valid_timestamp(value: &str) -> bool {
    DateTime::parse_from_rfc3339(value).is_ok()
}

impl RecognitionRecord {
    /// Structural checks are fail-closed. A record can be structurally valid
    /// but partial when an upstream producer lacks join metadata.
    pub fn validate(&self) -> Result<(), ContractError> {
        self.validate_structural()?;
        if !self.capture.allows_recognition() {
            return Err(ContractError::AmbientConsentRequired);
        }
        Ok(())
    }

    /// Validate against the trusted current consent policy. The serialized
    /// capture-time consent snapshot is never an authorization source.
    pub fn validate_with_current_consent(
        &self,
        current_consent: &RecognitionConsentConfig,
    ) -> Result<(), ContractError> {
        self.validate_structural()?;
        if !self
            .capture
            .allows_recognition_with_current_consent(current_consent)
        {
            return Err(ContractError::AmbientConsentRequired);
        }
        Ok(())
    }

    fn validate_structural(&self) -> Result<(), ContractError> {
        if self.provenance.contract_version != RECOGNITION_CONTRACT_VERSION {
            return Err(ContractError::InvalidContractVersion);
        }
        if self.id.trim().is_empty() || self.provenance.source.trim().is_empty() {
            return Err(ContractError::InvalidSource);
        }
        if !valid_timestamp(&self.provenance.observed_at) {
            return Err(ContractError::InvalidObservedAt);
        }
        if let Some(score) = self.uncertainty.score {
            if !score.is_finite() || !(0.0..=1.0).contains(&score) {
                return Err(ContractError::InvalidUncertainty);
            }
            if self
                .uncertainty
                .basis
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
                || self.uncertainty.as_of.as_deref().is_none_or(str::is_empty)
                || !valid_timestamp(self.uncertainty.as_of.as_deref().unwrap_or_default())
            {
                return Err(ContractError::InvalidUncertainty);
            }
        }
        if let Some(correction) = &self.correction {
            if correction.correction_id.trim().is_empty()
                || correction.target_id.trim().is_empty()
                || correction.source_event_id.trim().is_empty()
                || !nonempty(correction.operator_scope.as_ref())
                || correction.operator_scope.as_deref().map(str::trim)
                    != self.provenance.operator_scope.as_deref().map(str::trim)
                || correction.target_id.trim()
                    != self
                        .provenance
                        .memory_id
                        .as_deref()
                        .unwrap_or(self.id.as_str())
                        .trim()
                || !valid_timestamp(&correction.observed_at)
            {
                return Err(ContractError::InvalidCorrection);
            }
        }
        if self.kind == RecognitionKind::VerifiedLesson && self.stage != EvidenceStage::Verified {
            return Err(ContractError::VerifiedLessonStageRequired);
        }
        if self.kind == RecognitionKind::VerifiedLesson
            && (!nonempty(self.provenance.observed_outcome.as_ref())
                || (!nonempty(self.provenance.source_event_id.as_ref())
                    && !self
                        .provenance
                        .derived_from
                        .iter()
                        .any(|id| !id.trim().is_empty())))
        {
            return Err(ContractError::VerifiedLessonEvidenceRequired);
        }
        Ok(())
    }

    /// Reports exactly which metadata is absent. This never upgrades a
    /// partial observation into an interpretation or identity claim.
    pub fn status(&self) -> ContractStatus {
        let p = &self.provenance;
        let mut missing = Vec::new();
        if !nonempty(p.operator_scope.as_ref()) {
            missing.push(MissingMetadata::OperatorScope);
        }
        if !nonempty(p.session_id.as_ref()) {
            missing.push(MissingMetadata::SessionId);
        }
        if !nonempty(p.physical_invocation_id.as_ref()) {
            missing.push(MissingMetadata::PhysicalInvocationId);
        }
        if !nonempty(p.provider.as_ref()) {
            missing.push(MissingMetadata::Provider);
        }
        if !nonempty(p.model.as_ref()) {
            missing.push(MissingMetadata::Model);
        }
        if !nonempty(p.model_version.as_ref()) {
            missing.push(MissingMetadata::ModelVersion);
        }
        if !nonempty(p.contribution_id.as_ref()) {
            missing.push(MissingMetadata::ContributionId);
        }
        if !nonempty(p.retrieval_id.as_ref()) {
            missing.push(MissingMetadata::RetrievalId);
        }
        if self.stage != EvidenceStage::Retrieved && !nonempty(p.memory_id.as_ref()) {
            missing.push(MissingMetadata::MemoryId);
        }
        if self.stage == EvidenceStage::Verified && !nonempty(p.memory_version.as_ref()) {
            missing.push(MissingMetadata::MemoryVersion);
        }
        if self.stage == EvidenceStage::Acted && !nonempty(p.expected_outcome.as_ref()) {
            missing.push(MissingMetadata::ExpectedOutcome);
        }
        if matches!(self.stage, EvidenceStage::Verified) && !nonempty(p.observed_outcome.as_ref()) {
            missing.push(MissingMetadata::ObservedOutcome);
        }
        if missing.is_empty() {
            ContractStatus::Complete
        } else {
            ContractStatus::Partial(missing)
        }
    }
}

/// Returns true only when both corrections target the same fact, are scoped to
/// the same non-empty operator, and the candidate explicitly names the current
/// correction as superseded at a newer causal revision. Concurrent equal-
/// revision corrections and cross-operator updates remain unresolved.
pub fn correction_supersedes(
    current: &CorrectionProvenance,
    candidate: &CorrectionProvenance,
) -> bool {
    let Some(current_operator) = current
        .operator_scope
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return false;
    };
    let Some(candidate_operator) = candidate
        .operator_scope
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return false;
    };
    !current.correction_id.trim().is_empty()
        && !candidate.correction_id.trim().is_empty()
        && !current.target_id.trim().is_empty()
        && !candidate.target_id.trim().is_empty()
        && current_operator == candidate_operator
        && current.target_id == candidate.target_id
        && candidate.revision > current.revision
        && candidate
            .supersedes
            .iter()
            .any(|id| id == &current.correction_id)
}

/// The existing recognition instrumentation's outcome, kept sparse when the
/// write-back has not attributed an outcome yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExistingOutcomeEvidence {
    pub retrieved_at: String,
    pub session_id: Option<String>,
    pub outcome_kind: Option<String>,
    pub outcome_polarity: Option<String>,
    pub bounced: bool,
    pub provider_invocation_ids: Option<Vec<String>>,
    pub attribution_status: Option<String>,
    pub provider_invocations: Vec<ProviderInvocationSeen>,
}

/// Adapt the existing `RecognitionSeen` read model without converting missing
/// outcome fields into a synthetic interpretation.
pub fn outcome_from_seen(seen: &RecognitionSeen) -> ExistingOutcomeEvidence {
    ExistingOutcomeEvidence {
        retrieved_at: seen.retrieved_at.clone(),
        session_id: seen.session_id.clone(),
        outcome_kind: seen.outcome_kind.clone(),
        outcome_polarity: seen.outcome_polarity.clone(),
        bounced: seen.was_bounced(),
        provider_invocation_ids: seen.provider_invocation_ids.clone(),
        attribution_status: seen.attribution_status.clone(),
        provider_invocations: seen.provider_invocations.clone(),
    }
}

/// Adapt an in-flight existing recall into an observation. The adapter only
/// carries the durable retrieval join key; identity, memory, and outcome are
/// left unknown until the existing instrumentation supplies them.
pub fn observation_from_pending(
    pending: &PendingRecognition,
    session_id: Option<String>,
    source: impl Into<String>,
    observed_at: impl Into<String>,
) -> RecognitionRecord {
    let retrieval_id = pending.retrieval_id().to_string();
    RecognitionRecord {
        id: retrieval_id.clone(),
        kind: RecognitionKind::Observation,
        stage: EvidenceStage::Retrieved,
        provenance: RecognitionProvenance {
            contract_version: RECOGNITION_CONTRACT_VERSION.into(),
            source: source.into(),
            observed_at: observed_at.into(),
            session_id,
            retrieval_id: Some(retrieval_id),
            ..Default::default()
        },
        capture: CaptureBoundary::query(),
        uncertainty: Uncertainty::default(),
        correction: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognition_consent::WingScope;

    fn observation() -> RecognitionRecord {
        RecognitionRecord {
            id: "recognition-1".into(),
            kind: RecognitionKind::Observation,
            stage: EvidenceStage::Retrieved,
            provenance: RecognitionProvenance {
                contract_version: RECOGNITION_CONTRACT_VERSION.into(),
                source: "query_recall".into(),
                observed_at: "2026-09-05T12:00:00Z".into(),
                session_id: Some("session-1".into()),
                retrieval_id: Some("retrieval-1".into()),
                memory_id: Some("memory-1".into()),
                ..Default::default()
            },
            capture: CaptureBoundary::query(),
            uncertainty: Uncertainty::default(),
            correction: None,
        }
    }

    #[test]
    fn observation_and_interpretation_are_not_interchangeable() {
        let record = observation();
        assert_eq!(record.kind, RecognitionKind::Observation);
        assert!(matches!(record.status(), ContractStatus::Partial(_)));

        let mut interpretation = record.clone();
        interpretation.kind = RecognitionKind::Interpretation;
        interpretation.provenance.operator_scope = Some("operator-1".into());
        assert_eq!(interpretation.kind, RecognitionKind::Interpretation);
        assert_ne!(interpretation.kind, record.kind);
        assert!(interpretation.validate().is_ok());
    }

    #[test]
    fn missing_identity_and_metadata_stay_partial() {
        let record = observation();
        let ContractStatus::Partial(missing) = record.status() else {
            panic!("sparse source observation must not look complete")
        };
        assert!(missing.contains(&MissingMetadata::OperatorScope));
        assert!(missing.contains(&MissingMetadata::PhysicalInvocationId));
        assert!(missing.contains(&MissingMetadata::ModelVersion));
        assert!(record.validate().is_ok());
    }

    #[test]
    fn activity_capture_does_not_grant_ambient_recognition_consent() {
        let mut record = observation();
        record.capture = CaptureBoundary::ambient(
            RecognitionConsentConfig::default(),
            Some("permagent".into()),
            Some("browser".into()),
        )
        .with_activity_recorded(true);
        assert!(!record.capture.allows_recognition());
        assert_eq!(
            record.validate(),
            Err(ContractError::AmbientConsentRequired)
        );

        let mut consent = RecognitionConsentConfig {
            active: true,
            ..Default::default()
        };
        consent.wings.insert(
            "permagent".into(),
            WingScope {
                ambient: true,
                excluded_sources: vec![],
            },
        );
        record.capture = CaptureBoundary::ambient(
            consent.clone(),
            Some("permagent".into()),
            Some("browser".into()),
        );
        assert!(!record.capture.activity_recorded);
        assert_eq!(
            record.validate(),
            Err(ContractError::AmbientConsentRequired)
        );
        assert!(record.validate_with_current_consent(&consent).is_ok());

        let revoked = RecognitionConsentConfig::default();
        assert_eq!(
            record.validate_with_current_consent(&revoked),
            Err(ContractError::AmbientConsentRequired)
        );
    }

    #[test]
    fn revocation_blocks_ambient_replay_without_gating_query_instrumentation() {
        let mut ambient = observation();
        let mut consent = RecognitionConsentConfig {
            active: true,
            ..Default::default()
        };
        consent.wings.insert(
            "permagent".into(),
            WingScope {
                ambient: true,
                excluded_sources: vec![],
            },
        );
        ambient.capture = CaptureBoundary::ambient(
            consent.clone(),
            Some("permagent".into()),
            Some("browser".into()),
        );

        assert!(ambient.validate_with_current_consent(&consent).is_ok());
        assert_eq!(
            ambient.validate_with_current_consent(&RecognitionConsentConfig::default()),
            Err(ContractError::AmbientConsentRequired)
        );

        // Query-mode recognition is an explicit local recall observation, not
        // ambient capture. Revoking the ambient policy must not silently erase
        // or reject that separate evidence path.
        let query = observation();
        assert!(query
            .validate_with_current_consent(&RecognitionConsentConfig::default())
            .is_ok());
    }

    #[test]
    fn correction_provenance_beats_stale_wall_clock_evidence() {
        let current = CorrectionProvenance {
            correction_id: "corr-a".into(),
            target_id: "memory-1".into(),
            supersedes: vec![],
            source_event_id: "event-a".into(),
            operator_scope: Some("operator-1".into()),
            device_id: Some("device-a".into()),
            revision: 4,
            observed_at: "2026-09-05T13:00:00Z".into(),
        };
        let newer = CorrectionProvenance {
            correction_id: "corr-b".into(),
            target_id: "memory-1".into(),
            supersedes: vec!["corr-a".into()],
            source_event_id: "event-b".into(),
            operator_scope: Some("operator-1".into()),
            device_id: Some("device-b".into()),
            revision: 5,
            // Device B's clock is behind; revision is the causal authority.
            observed_at: "2026-09-05T12:30:00Z".into(),
        };
        assert!(correction_supersedes(&current, &newer));
        assert!(!correction_supersedes(&newer, &current));
        assert!(!correction_supersedes(
            &current,
            &CorrectionProvenance {
                target_id: "other-memory".into(),
                revision: 99,
                ..newer.clone()
            }
        ));
        assert!(!correction_supersedes(
            &current,
            &CorrectionProvenance {
                operator_scope: Some("other-operator".into()),
                revision: 99,
                supersedes: vec!["corr-a".into()],
                ..newer.clone()
            }
        ));
        assert!(!correction_supersedes(
            &current,
            &CorrectionProvenance {
                revision: current.revision,
                supersedes: vec!["corr-a".into()],
                ..newer.clone()
            }
        ));
        assert!(!correction_supersedes(
            &current,
            &CorrectionProvenance {
                supersedes: vec![],
                ..newer.clone()
            }
        ));
        assert!(!correction_supersedes(
            &CorrectionProvenance {
                operator_scope: Some(" operator-1 ".into()),
                ..current.clone()
            },
            &newer
        ));
    }

    #[test]
    fn correction_must_match_record_operator_and_memory_target() {
        let mut record = observation();
        record.provenance.operator_scope = Some("operator-1".into());
        record.correction = Some(CorrectionProvenance {
            correction_id: "corr-a".into(),
            target_id: "memory-1".into(),
            supersedes: vec![],
            source_event_id: "event-a".into(),
            operator_scope: Some("operator-1".into()),
            device_id: Some("device-a".into()),
            revision: 1,
            observed_at: "2026-09-05T12:00:00Z".into(),
        });
        assert!(record.validate().is_ok());

        record.correction.as_mut().unwrap().operator_scope = Some("other-operator".into());
        assert_eq!(record.validate(), Err(ContractError::InvalidCorrection));

        record.correction.as_mut().unwrap().operator_scope = Some("operator-1".into());
        record.correction.as_mut().unwrap().target_id = "record-1".into();
        assert_eq!(record.validate(), Err(ContractError::InvalidCorrection));
    }

    #[test]
    fn invalid_uncertainty_cannot_pass_as_a_score() {
        let mut record = observation();
        record.uncertainty = Uncertainty {
            score: Some(f64::NAN),
            basis: Some("fixture".into()),
            as_of: Some("2026-09-05T12:00:00Z".into()),
        };
        assert_eq!(record.validate(), Err(ContractError::InvalidUncertainty));
    }

    #[test]
    fn malformed_timestamp_and_whitespace_metadata_are_not_complete() {
        let mut record = observation();
        record.provenance.observed_at = "yesterday".into();
        assert_eq!(record.validate(), Err(ContractError::InvalidObservedAt));

        record.provenance.observed_at = "2026-09-05T12:00:00Z".into();
        record.kind = RecognitionKind::Interpretation;
        record.provenance.operator_scope = Some("   ".into());
        assert!(matches!(record.status(), ContractStatus::Partial(missing)
            if missing.contains(&MissingMetadata::OperatorScope)));
    }

    #[test]
    fn verified_lesson_requires_outcome_and_real_source_evidence() {
        let mut record = observation();
        record.kind = RecognitionKind::VerifiedLesson;
        assert_eq!(
            record.validate(),
            Err(ContractError::VerifiedLessonStageRequired)
        );

        record.stage = EvidenceStage::Verified;
        assert_eq!(
            record.validate(),
            Err(ContractError::VerifiedLessonEvidenceRequired)
        );
        record.provenance.observed_outcome = Some("resolved".into());
        record.provenance.source_event_id = Some("   ".into());
        record.provenance.derived_from = vec![" ".into()];
        assert_eq!(
            record.validate(),
            Err(ContractError::VerifiedLessonEvidenceRequired)
        );

        record.provenance.source_event_id = Some("event-1".into());
        assert!(record.validate().is_ok());
        assert!(matches!(record.status(), ContractStatus::Partial(missing)
            if !missing.contains(&MissingMetadata::CorrectionProvenance)));
    }

    #[test]
    fn seen_adapter_preserves_missing_outcome_fields() {
        let seen = RecognitionSeen {
            retrieved_at: "2026-09-05T12:00:00Z".into(),
            session_id: Some("session-1".into()),
            outcome_kind: None,
            outcome_polarity: None,
            provider_invocation_ids: None,
            attribution_status: None,
            provider_invocations: vec![],
        };
        assert_eq!(
            outcome_from_seen(&seen),
            ExistingOutcomeEvidence {
                retrieved_at: "2026-09-05T12:00:00Z".into(),
                session_id: Some("session-1".into()),
                outcome_kind: None,
                outcome_polarity: None,
                bounced: false,
                provider_invocation_ids: None,
                attribution_status: None,
                provider_invocations: vec![],
            }
        );
    }
}
