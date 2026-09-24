use super::inventory::{Artifact, ConsumerState, Inventory};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingTier {
    UpdateAvailable,
    CandidateWorthEvaluating,
    // Contract only. W1a has no benchmark receipt and cannot emit this tier.
    ValidatedSwitchRecommendation,
    PolicyBlocked,
    UpstreamUnknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownIdentity {
    pub identity: String,
    pub released_at: Option<String>,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownModel {
    pub family: String,
    pub upstream_source: String,
    pub known_latest: Option<KnownIdentity>,
    pub snapshot_taken_at: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub source: String,
    pub snapshot_taken_at: Option<String>,
    pub incumbent_identity: String,
    pub candidate_identity: Option<String>,
    pub quantization: Option<String>,
    pub candidate_quantization: Option<String>,
    pub compatibility: String,
    pub size: Option<u64>,
    pub candidate_size: Option<u64>,
    pub uncertainty: String,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub consumer: String,
    pub tier: FindingTier,
    pub action: String,
    pub discovery: Discovery,
}

pub fn snapshot() -> anyhow::Result<Vec<KnownModel>> {
    Ok(serde_json::from_str(include_str!("known_models.json"))?)
}

fn discovery(artifact: &Artifact, known: Option<&KnownModel>, now: DateTime<Utc>) -> Discovery {
    let stale = known
        .and_then(|k| NaiveDate::parse_from_str(&k.snapshot_taken_at, "%Y-%m-%d").ok())
        .map(|date| (now.date_naive() - date).num_days() > 30 || date > now.date_naive())
        .unwrap_or(true);
    Discovery {
        source: known.map(|k| k.upstream_source.clone()).unwrap_or_else(|| artifact.source.clone()),
        snapshot_taken_at: known.map(|k| k.snapshot_taken_at.clone()),
        incumbent_identity: format!("{}|{}", artifact.identity, artifact.digest.as_deref().unwrap_or("digest unknown")),
        candidate_identity: known.and_then(|k| k.known_latest.as_ref()).map(|k| k.identity.clone()),
        quantization: artifact.quantization.clone(), candidate_quantization: None,
        compatibility: "unknown; no runtime load or benchmark performed".into(),
        size: Some(artifact.size), candidate_size: None,
        uncertainty: format!("{}; changed digests mean changed bytes, not newer or better models; release chronology and incumbent provenance unverified{}",
            known.map(|k| k.notes.as_str()).unwrap_or("upstream unknown"), if stale { "; snapshot stale or undated" } else { "" }),
        stale,
    }
}

/// Pure comparison: it receives only local values and has no transport argument
/// or transport factory. A pin is provenance, NOT proof it is the newest release.
/// Consequently this seed snapshot can produce evaluation candidates, never an
/// update/switch claim. Authoritative incumbent chronology is not yet available.
pub fn discover(inv: &Inventory, known: &[KnownModel], now: DateTime<Utc>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for artifact in &inv.artifacts {
        let record = known
            .iter()
            .find(|k| k.family == artifact.family || k.family == artifact.identity);
        let evidence = discovery(artifact, record, now);
        let consumers: Vec<_> = inv
            .consumers
            .iter()
            .filter(|c| c.artifacts.contains(&artifact.path))
            .collect();
        let ids: Vec<_> = if consumers.is_empty() {
            vec![(format!("artifact:{}", artifact.path.display()), false)]
        } else {
            consumers
                .iter()
                .map(|c| (c.id.clone(), c.state == ConsumerState::Paused))
                .collect()
        };
        for (consumer, paused) in ids {
            let tier = if evidence.candidate_identity.is_some()
                && evidence.candidate_identity.as_deref() != artifact.digest.as_deref()
                && !evidence.stale
                && !paused
            {
                FindingTier::CandidateWorthEvaluating
            } else {
                FindingTier::UpstreamUnknown
            };
            findings.push(Finding {
                consumer,
                tier,
                action: "review_local_provenance".into(),
                discovery: evidence.clone(),
            });
        }
    }
    // Configured but missing/unmapped models still receive a ledger finding.
    for c in &inv.consumers {
        if !inv.artifacts.iter().any(|a| c.artifacts.contains(&a.path)) {
            findings.push(Finding {
                consumer: c.id.clone(),
                tier: FindingTier::UpstreamUnknown,
                action: "inventory_only".into(),
                discovery: Discovery {
                    source: "local consumer configuration/reference".into(),
                    snapshot_taken_at: None,
                    incumbent_identity: c.identity.clone().unwrap_or_else(|| "unknown".into()),
                    candidate_identity: None,
                    quantization: None,
                    candidate_quantization: None,
                    compatibility: "unknown".into(),
                    size: None,
                    candidate_size: None,
                    uncertainty: c.uncertainty.clone(),
                    stale: false,
                },
            });
        }
    }
    findings.push(Finding {
        consumer: "model_review".into(),
        tier: FindingTier::PolicyBlocked,
        action: "live_metadata_discovery_blocked".into(),
        discovery: Discovery {
            source: "W1_ASTRA_RULING.md rulings 4–5".into(),
            snapshot_taken_at: None,
            incumbent_identity: "local-only".into(),
            candidate_identity: None,
            quantization: None,
            candidate_quantization: None,
            compatibility: "not evaluated".into(),
            size: None,
            candidate_size: None,
            uncertainty: super::LOCAL_ONLY.into(),
            stale: false,
        },
    });
    findings.sort_by_key(key);
    findings.dedup_by(|a, b| key(a) == key(b));
    findings
}

/// JSON array encoding is unambiguous even for custom aliases with delimiters.
pub fn key(f: &Finding) -> String {
    serde_json::json!([
        f.consumer,
        f.discovery.incumbent_identity,
        f.discovery.candidate_identity,
        f.action
    ])
    .to_string()
}
