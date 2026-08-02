//! Grounded failure incident capture (failure-learning loop, Phase 1 only).
//!
//! An incident records that something went wrong; it is neither a lesson nor a
//! fix. Evidence is enforced here because model instructions are not a trusted
//! boundary: an agent's narration alone cannot ground an incident.

use chrono::Utc;
use sqlx::{Pool, Sqlite};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("unknown incident enum value")]
pub struct ParseIncidentEnumError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    Environment,
    DesignAssumption,
    ErrorSwallowing,
    FailPlausible,
    OperationalOmission,
    Unclassified,
}

impl Mechanism {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "A_environment",
            Self::DesignAssumption => "B_design_assumption",
            Self::ErrorSwallowing => "C_error_swallowing",
            Self::FailPlausible => "D_fail_plausible",
            Self::OperationalOmission => "E_operational_omission",
            Self::Unclassified => "unclassified",
        }
    }
}

impl FromStr for Mechanism {
    type Err = ParseIncidentEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "A_environment" => Ok(Self::Environment),
            "B_design_assumption" => Ok(Self::DesignAssumption),
            "C_error_swallowing" => Ok(Self::ErrorSwallowing),
            "D_fail_plausible" => Ok(Self::FailPlausible),
            "E_operational_omission" => Ok(Self::OperationalOmission),
            "unclassified" => Ok(Self::Unclassified),
            _ => Err(ParseIncidentEnumError),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    UserReport,
    ToolError,
    ExitCode,
    HttpStatus,
    RunDiff,
    RecognitionRecord,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserReport => "user_report",
            Self::ToolError => "tool_error",
            Self::ExitCode => "exit_code",
            Self::HttpStatus => "http_status",
            Self::RunDiff => "run_diff",
            Self::RecognitionRecord => "recognition_record",
        }
    }
}

impl FromStr for ArtifactKind {
    type Err = ParseIncidentEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user_report" => Ok(Self::UserReport),
            "tool_error" => Ok(Self::ToolError),
            "exit_code" => Ok(Self::ExitCode),
            "http_status" => Ok(Self::HttpStatus),
            "run_diff" => Ok(Self::RunDiff),
            "recognition_record" => Ok(Self::RecognitionRecord),
            _ => Err(ParseIncidentEnumError),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIncident {
    pub session_id: Option<String>,
    pub surface: String,
    pub user_goal: String,
    pub observation: String,
    pub mechanism: Mechanism,
    pub artifact_kind: ArtifactKind,
    pub artifact_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incident {
    pub id: String,
    pub created_at: String,
    pub session_id: Option<String>,
    pub surface: String,
    pub user_goal: String,
    pub observation: String,
    pub mechanism: Mechanism,
    pub artifact_kind: ArtifactKind,
    pub artifact_ref: String,
    pub status: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum IncidentError {
    #[error("artifact_ref is required evidence and must not be empty or whitespace")]
    MissingArtifactRef,
    #[error("database contains an unknown incident mechanism: {0}")]
    InvalidStoredMechanism(String),
    #[error("database contains an unknown incident artifact kind: {0}")]
    InvalidStoredArtifactKind(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

type IncidentRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
);

fn incident_from_row(row: IncidentRow) -> Result<Incident, IncidentError> {
    let (
        id,
        created_at,
        session_id,
        surface,
        user_goal,
        observation,
        mechanism,
        artifact_kind,
        artifact_ref,
        status,
        resolved_at,
    ) = row;
    let mechanism = Mechanism::from_str(&mechanism)
        .map_err(|_| IncidentError::InvalidStoredMechanism(mechanism))?;
    let artifact_kind = ArtifactKind::from_str(&artifact_kind)
        .map_err(|_| IncidentError::InvalidStoredArtifactKind(artifact_kind))?;
    Ok(Incident {
        id,
        created_at,
        session_id,
        surface,
        user_goal,
        observation,
        mechanism,
        artifact_kind,
        artifact_ref,
        status,
        resolved_at,
    })
}

pub async fn create_incident(
    pool: &Pool<Sqlite>,
    incident: NewIncident,
) -> Result<Incident, IncidentError> {
    if incident.artifact_ref.trim().is_empty() {
        return Err(IncidentError::MissingArtifactRef);
    }

    let id = Uuid::now_v7().to_string();
    let created_at = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    sqlx::query(
        "INSERT INTO incidents
            (id, created_at, session_id, surface, user_goal, observation,
             mechanism, artifact_kind, artifact_ref)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&created_at)
    .bind(&incident.session_id)
    .bind(&incident.surface)
    .bind(&incident.user_goal)
    .bind(&incident.observation)
    .bind(incident.mechanism.as_str())
    .bind(incident.artifact_kind.as_str())
    .bind(&incident.artifact_ref)
    .execute(pool)
    .await?;

    Ok(Incident {
        id,
        created_at,
        session_id: incident.session_id,
        surface: incident.surface,
        user_goal: incident.user_goal,
        observation: incident.observation,
        mechanism: incident.mechanism,
        artifact_kind: incident.artifact_kind,
        artifact_ref: incident.artifact_ref,
        status: "open".to_string(),
        resolved_at: None,
    })
}

pub async fn list_open_incidents(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<Vec<Incident>, IncidentError> {
    let rows = sqlx::query_as::<_, IncidentRow>(
        "SELECT id, created_at, session_id, surface, user_goal, observation,
                mechanism, artifact_kind, artifact_ref, status, resolved_at
           FROM incidents
          WHERE status = 'open'
          ORDER BY created_at DESC, id DESC
          LIMIT ?",
    )
    .bind(limit.max(0))
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(incident_from_row).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    fn new_incident(mechanism: Mechanism, artifact_ref: &str) -> NewIncident {
        NewIncident {
            session_id: None,
            surface: "chat".to_string(),
            user_goal: "get the weather".to_string(),
            observation: "the agent said search was disabled".to_string(),
            mechanism,
            artifact_kind: ArtifactKind::UserReport,
            artifact_ref: artifact_ref.to_string(),
        }
    }

    #[tokio::test]
    async fn create_incident_rejects_empty_or_whitespace_artifact_ref() {
        let pool = test_pool().await;
        for artifact_ref in ["", "   ", "\n\t"] {
            let error = create_incident(&pool, new_incident(Mechanism::Unclassified, artifact_ref))
                .await
                .unwrap_err();
            assert!(matches!(error, IncidentError::MissingArtifactRef));
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM incidents")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn create_incident_round_trips_all_mechanisms() {
        let pool = test_pool().await;
        let mechanisms = [
            Mechanism::Environment,
            Mechanism::DesignAssumption,
            Mechanism::ErrorSwallowing,
            Mechanism::FailPlausible,
            Mechanism::OperationalOmission,
            Mechanism::Unclassified,
        ];

        for mechanism in mechanisms {
            let created = create_incident(&pool, new_incident(mechanism, "user message 42"))
                .await
                .unwrap();
            assert_eq!(created.mechanism, mechanism);
        }

        let listed = list_open_incidents(&pool, 10).await.unwrap();
        assert_eq!(listed.len(), mechanisms.len());
        for mechanism in mechanisms {
            assert!(listed
                .iter()
                .any(|incident| incident.mechanism == mechanism));
        }
    }

    #[tokio::test]
    async fn incidents_check_rejects_an_invalid_mechanism_string() {
        let pool = test_pool().await;
        let error = sqlx::query(
            "INSERT INTO incidents
                (id, created_at, surface, user_goal, observation, mechanism,
                 artifact_kind, artifact_ref)
             VALUES ('bad', '2026-08-02T00:00:00.000Z', 'chat', 'goal',
                     'observation', 'F_invented', 'user_report', 'message 42')",
        )
        .execute(&pool)
        .await
        .unwrap_err();
        assert!(error.to_string().contains("CHECK constraint failed"));
    }

    #[tokio::test]
    async fn list_open_incidents_returns_only_open_newest_first() {
        let pool = test_pool().await;
        for (id, created_at, status) in [
            ("old-open", "2026-08-01T00:00:00.000Z", "open"),
            ("triaged", "2026-08-03T00:00:00.000Z", "triaged"),
            ("new-open", "2026-08-02T00:00:00.000Z", "open"),
        ] {
            sqlx::query(
                "INSERT INTO incidents
                    (id, created_at, surface, user_goal, observation, mechanism,
                     artifact_kind, artifact_ref, status)
                 VALUES (?, ?, 'chat', 'goal', 'observation', 'unclassified',
                         'user_report', 'message 42', ?)",
            )
            .bind(id)
            .bind(created_at)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        }

        let incidents = list_open_incidents(&pool, 10).await.unwrap();
        assert_eq!(
            incidents
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new-open", "old-open"]
        );
    }
}
