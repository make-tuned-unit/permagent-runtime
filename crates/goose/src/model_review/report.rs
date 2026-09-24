use super::{
    findings::{self, Finding},
    inventory::Inventory,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub created_at: String,
    pub discovery_mode: String,
    pub summary: String,
    pub partial: bool,
    pub stale: bool,
    pub policy_blocked: bool,
    pub inventory: Inventory,
    pub findings: Vec<Finding>,
}

pub fn build(mut inventory: Inventory, now: DateTime<Utc>) -> anyhow::Result<Report> {
    let known = match findings::snapshot() {
        Ok(known) => known,
        Err(error) => {
            inventory
                .issues
                .push(format!("Local snapshot unavailable: {error}"));
            Vec::new()
        }
    };
    let findings = findings::discover(&inventory, &known, now);
    Ok(Report {
        id: uuid::Uuid::new_v4().to_string(), created_at: now.to_rfc3339(),
        discovery_mode: "local-only".into(),
        summary: format!("{} {} local artifacts; {} consumers. Runtime activity, signed application bundles and nonstandard deployment paths are unverified.",
            super::LOCAL_ONLY, inventory.artifacts.len(), inventory.consumers.len()),
        // Offline manifest inventory cannot establish runtime activity or discover
        // arbitrary CLI deployment paths. Coverage stays partial even with no IO errors.
        partial: true,
        stale: findings.iter().any(|f| f.discovery.stale), policy_blocked: true,
        inventory, findings,
    })
}

/// One transaction makes an empty/partial/blocked report as durable as a report
/// with findings. The report retains its full historical findings; the ledger
/// has one current row per consumer/incumbent/candidate/action, never Trash/Keep.
pub async fn persist_report(pool: &SqlitePool, report: &Report) -> anyhow::Result<()> {
    anyhow::ensure!(
        !report
            .findings
            .iter()
            .any(|f| f.tier == findings::FindingTier::ValidatedSwitchRecommendation),
        "W1a cannot persist a validated switch recommendation"
    );
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO model_review_reports(id, created_at, report_json) VALUES (?, ?, ?)")
        .bind(&report.id)
        .bind(&report.created_at)
        .bind(serde_json::to_string(report)?)
        .execute(&mut *tx)
        .await?;
    for finding in &report.findings {
        sqlx::query("INSERT INTO model_review_findings(finding_key, report_id, consumer, incumbent_identity, candidate_identity, action, finding_json)
            VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(finding_key) DO UPDATE SET report_id=excluded.report_id, finding_json=excluded.finding_json")
            .bind(findings::key(finding)).bind(&report.id).bind(&finding.consumer)
            .bind(&finding.discovery.incumbent_identity).bind(finding.discovery.candidate_identity.as_deref().unwrap_or(""))
            .bind(&finding.action).bind(serde_json::to_string(finding)?)
            .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn latest(pool: &SqlitePool) -> anyhow::Result<Option<Report>> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT report_json FROM model_review_reports ORDER BY created_at DESC, rowid DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    value
        .map(|v| serde_json::from_str(&v).map_err(Into::into))
        .transpose()
}

pub async fn ledger(pool: &SqlitePool) -> anyhow::Result<Vec<Finding>> {
    let values: Vec<String> = sqlx::query_scalar(
        "SELECT finding_json FROM model_review_findings ORDER BY consumer, finding_key",
    )
    .fetch_all(pool)
    .await?;
    values
        .into_iter()
        .map(|v| serde_json::from_str(&v).map_err(Into::into))
        .collect()
}
