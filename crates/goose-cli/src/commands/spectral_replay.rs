//! Read-only Spectral production-replay export.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use permagent::config::paths::Paths;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
struct EventRow {
    retrieval_id: String,
    ts: String,
    query: String,
    outcome: Option<String>,
    outcome_ts: Option<String>,
    verdict: Option<String>,
    familiarity: Option<f64>,
    cited_memory_ids: Vec<String>,
    injected_memory_ids: Option<Vec<String>>,
    injected_memory_ids_source: Option<String>,
    citation_checked_at: Option<String>,
}

#[derive(Debug, Clone)]
struct MemberRow {
    memory_id: String,
    rank: Option<i64>,
    score: Option<f64>,
}

#[derive(Serialize)]
struct RecognitionLine<'a> {
    ts: &'a str,
    event: &'static str,
    verdict: Option<&'a str>,
    familiarity: Option<f64>,
    memory_id: Option<&'a str>,
    probe_hash: &'a str,
    outcome: Option<&'a str>,
    outcome_ts: Option<&'a str>,
    retrieval_id: &'a str,
}

#[derive(Serialize)]
struct RecallLine<'a> {
    ts: &'a str,
    event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<&'a str>,
    probe_hash: &'a str,
    returned: Vec<&'a str>,
    ranks: Vec<Option<i64>>,
    scores: Vec<Option<f64>>,
    injected: Option<&'a [String]>,
    injected_source: Option<&'a str>,
    used: Option<&'a [String]>,
    citation_checked_at: Option<&'a str>,
    retrieval_id: &'a str,
}

#[derive(Serialize)]
struct DateRange<'a> {
    first_ts: Option<&'a str>,
    last_ts: Option<&'a str>,
}

#[derive(Serialize)]
struct ExportCounts {
    recognition_events: usize,
    recognition_jsonl_rows: usize,
    recall_jsonl_rows: usize,
    recognition_set_members: usize,
    outcome_labels: BTreeMap<String, usize>,
    injected_memory_ids_sources: BTreeMap<String, usize>,
    citation_checked_events: usize,
}

#[derive(Serialize)]
struct NullField {
    field: &'static str,
    null_rows: usize,
    total_rows: usize,
    null_by_construction: bool,
    reason: &'static str,
}

#[derive(Serialize)]
struct Manifest<'a> {
    format: &'static str,
    format_version: u32,
    generated_at: String,
    source_schema_version: i64,
    since: Option<&'a str>,
    queries_redacted: bool,
    counts: ExportCounts,
    date_range: DateRange<'a>,
    null_fields: Vec<NullField>,
    notes: BTreeMap<&'static str, &'static str>,
}

pub fn handle_spectral_replay(
    out: PathBuf,
    redact_queries: bool,
    since: Option<String>,
) -> Result<()> {
    let since = since
        .as_deref()
        .map(normalize_since)
        .transpose()
        .context("invalid --since timestamp")?;
    let db_path = Paths::spectral_db();
    let counts = export_spectral_replay(&db_path, &out, redact_queries, since.as_deref())?;
    println!(
        "Exported {} recognition rows and {} recall rows to {}",
        counts.0,
        counts.1,
        out.display()
    );
    Ok(())
}

fn normalize_since(value: &str) -> Result<String> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("expected RFC 3339, got {value:?}"))?;
    Ok(parsed
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn probe_hash(query: &str) -> String {
    hex::encode(Sha256::digest(query.as_bytes()))
}

fn parse_id_array(raw: &str, field: &str, retrieval_id: &str) -> Result<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .with_context(|| format!("invalid {field} JSON for recognition event {retrieval_id}"))?;
    let Some(values) = value.as_array() else {
        bail!("{field} is not an array for recognition event {retrieval_id}");
    };
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("{field} contains a non-string for {retrieval_id}"))
        })
        .collect()
}

fn write_json_line<T: Serialize>(writer: &mut BufWriter<File>, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

/// Export from `db_path` without ever opening it writable. Kept separate from
/// the CLI wrapper so tests can prove the contract against a fixture database.
fn export_spectral_replay(
    db_path: &Path,
    out: &Path,
    redact_queries: bool,
    since: Option<&str>,
) -> Result<(usize, usize)> {
    if !db_path.is_file() {
        bail!("Permagent database not found at {}", db_path.display());
    }
    fs::create_dir_all(out)
        .with_context(|| format!("create export directory {}", out.display()))?;

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {} read-only", db_path.display()))?;
    conn.pragma_update(None, "query_only", true)?;

    for column in [
        "recognition_verdict",
        "familiarity",
        "injected_memory_ids",
        "injected_memory_ids_source",
        "citation_checked_at",
        "outcome_label",
    ] {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('recognition_events') WHERE name = ?1",
            [column],
            |row| row.get(0),
        )?;
        if exists == 0 {
            bail!(
                "recognition_events.{column} is missing; start Permagent once to apply the additive schema repair"
            );
        }
    }

    let schema_version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .context("read schema_version")?;
    let where_sql = if since.is_some() {
        " WHERE retrieved_at >= ?1"
    } else {
        ""
    };
    let event_sql = format!(
        "SELECT retrieval_id, retrieved_at, query, outcome_label, outcome_observed_at,
                recognition_verdict, familiarity, cited_memory_ids, injected_memory_ids,
                injected_memory_ids_source, citation_checked_at
           FROM recognition_events{where_sql}
          ORDER BY retrieved_at, retrieval_id"
    );
    let mut event_stmt = conn.prepare(&event_sql)?;
    let mut rows = if let Some(since) = since {
        event_stmt.query([since])?
    } else {
        event_stmt.query([])?
    };
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let retrieval_id: String = row.get(0)?;
        let cited_raw: String = row.get(7)?;
        let injected_raw: Option<String> = row.get(8)?;
        events.push(EventRow {
            retrieval_id: retrieval_id.clone(),
            ts: row.get(1)?,
            query: row.get(2)?,
            outcome: row.get(3)?,
            outcome_ts: row.get(4)?,
            verdict: row.get(5)?,
            familiarity: row.get(6)?,
            cited_memory_ids: parse_id_array(&cited_raw, "cited_memory_ids", &retrieval_id)?,
            injected_memory_ids: injected_raw
                .as_deref()
                .map(|raw| parse_id_array(raw, "injected_memory_ids", &retrieval_id))
                .transpose()?,
            injected_memory_ids_source: row.get(9)?,
            citation_checked_at: row.get(10)?,
        });
    }
    drop(rows);
    drop(event_stmt);

    let member_sql = format!(
        "SELECT members.retrieval_id, members.memory_id, members.rank, members.signal_score
           FROM recognition_set_members members
           JOIN recognition_events events ON events.retrieval_id = members.retrieval_id{where_sql}
          ORDER BY events.retrieved_at, events.retrieval_id,
                   members.rank IS NULL, members.rank, members.memory_id"
    );
    let mut member_stmt = conn.prepare(&member_sql)?;
    let mut member_rows = if let Some(since) = since {
        member_stmt.query([since])?
    } else {
        member_stmt.query([])?
    };
    let mut members: BTreeMap<String, Vec<MemberRow>> = BTreeMap::new();
    while let Some(row) = member_rows.next()? {
        members.entry(row.get(0)?).or_default().push(MemberRow {
            memory_id: row.get(1)?,
            rank: row.get(2)?,
            score: row.get(3)?,
        });
    }

    let recognition_path = out.join("recognition.jsonl");
    let recall_path = out.join("recall.jsonl");
    let mut recognition_out = BufWriter::new(File::create(&recognition_path)?);
    let mut recall_out = BufWriter::new(File::create(&recall_path)?);
    let mut recognition_rows = 0usize;
    let mut set_member_rows = 0usize;
    let mut outcome_labels = BTreeMap::new();
    let mut injection_sources = BTreeMap::new();
    let mut citation_checked_events = 0usize;
    let mut verdict_null_rows = 0usize;
    let mut familiarity_null_rows = 0usize;
    let mut used_null_rows = 0usize;

    for event in &events {
        let hash = probe_hash(&event.query);
        let event_members = members
            .get(&event.retrieval_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        set_member_rows += event_members.len();
        write_json_line(
            &mut recognition_out,
            &RecognitionLine {
                ts: &event.ts,
                event: "recognition",
                verdict: event.verdict.as_deref(),
                familiarity: event.familiarity,
                // The trace id belongs only to Spectral's Recognized verdict
                // variant. recognize() is not wired, so no honest id exists.
                memory_id: None,
                probe_hash: &hash,
                outcome: event.outcome.as_deref(),
                outcome_ts: event.outcome_ts.as_deref(),
                retrieval_id: &event.retrieval_id,
            },
        )?;
        recognition_rows += 1;

        write_json_line(
            &mut recall_out,
            &RecallLine {
                ts: &event.ts,
                event: "recall",
                query: (!redact_queries).then_some(event.query.as_str()),
                probe_hash: &hash,
                returned: event_members
                    .iter()
                    .map(|member| member.memory_id.as_str())
                    .collect(),
                ranks: event_members.iter().map(|member| member.rank).collect(),
                scores: event_members.iter().map(|member| member.score).collect(),
                injected: event.injected_memory_ids.as_deref(),
                injected_source: event.injected_memory_ids_source.as_deref(),
                used: event
                    .citation_checked_at
                    .as_ref()
                    .map(|_| event.cited_memory_ids.as_slice()),
                citation_checked_at: event.citation_checked_at.as_deref(),
                retrieval_id: &event.retrieval_id,
            },
        )?;

        *outcome_labels
            .entry(event.outcome.as_deref().unwrap_or("null").to_string())
            .or_insert(0) += 1;
        *injection_sources
            .entry(
                event
                    .injected_memory_ids_source
                    .as_deref()
                    .unwrap_or("unknown")
                    .to_string(),
            )
            .or_insert(0) += 1;
        citation_checked_events += usize::from(event.citation_checked_at.is_some());
        verdict_null_rows += usize::from(event.verdict.is_none());
        familiarity_null_rows += usize::from(event.familiarity.is_none());
        used_null_rows += usize::from(event.citation_checked_at.is_none());
    }
    recognition_out.flush()?;
    recall_out.flush()?;

    let event_count = events.len();
    let mut notes = BTreeMap::new();
    notes.insert(
        "recognition_rows",
        "recognition.jsonl has exactly one row per recognition event; returned candidates live only in recall.jsonl",
    );
    notes.insert(
        "returned_alignment",
        "recall returned, ranks, and scores are parallel arrays in ascending persisted rank order",
    );
    notes.insert(
        "historical_outcomes",
        "historical outcome labels stay null because citation detection never ran; cited_memory_ids='[]' alone is not a measurement",
    );
    notes.insert(
        "historical_injection",
        "reconstructed injection is exact deterministic replay of the stable score>=0.7, rank-order, top-3 filter; recorded means captured forward at prompt injection",
    );
    let manifest = Manifest {
        format: "spectral-replay",
        format_version: FORMAT_VERSION,
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        source_schema_version: schema_version,
        since,
        queries_redacted: redact_queries,
        counts: ExportCounts {
            recognition_events: event_count,
            recognition_jsonl_rows: recognition_rows,
            recall_jsonl_rows: event_count,
            recognition_set_members: set_member_rows,
            outcome_labels,
            injected_memory_ids_sources: injection_sources,
            citation_checked_events,
        },
        date_range: DateRange {
            first_ts: events.first().map(|event| event.ts.as_str()),
            last_ts: events.last().map(|event| event.ts.as_str()),
        },
        null_fields: vec![
            NullField {
                field: "recognition.verdict",
                null_rows: verdict_null_rows,
                total_rows: event_count,
                null_by_construction: event_count > 0 && verdict_null_rows == event_count,
                reason: "Spectral recognize() is not wired to a production caller yet",
            },
            NullField {
                field: "recognition.familiarity",
                null_rows: familiarity_null_rows,
                total_rows: event_count,
                null_by_construction: event_count > 0 && familiarity_null_rows == event_count,
                reason: "Spectral recognize() is not wired to a production caller yet",
            },
            NullField {
                field: "recognition.memory_id",
                null_rows: event_count,
                total_rows: event_count,
                null_by_construction: event_count > 0,
                reason: "memory_id is populated only for a Recognized verdict; Spectral recognize() is not wired yet",
            },
            NullField {
                field: "recall.used",
                null_rows: used_null_rows,
                total_rows: event_count,
                null_by_construction: event_count > 0 && used_null_rows == event_count,
                reason: "used is null when citation detection never ran; [] means measured with no citation",
            },
        ],
        notes,
    };
    let manifest_path = out.join("manifest.json");
    let mut manifest_out = BufWriter::new(File::create(&manifest_path)?);
    serde_json::to_writer_pretty(&mut manifest_out, &manifest)?;
    manifest_out.write_all(b"\n")?;
    manifest_out.flush()?;

    Ok((recognition_rows, event_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("permagent.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
             INSERT INTO schema_version VALUES (40);
             CREATE TABLE recognition_events (
                retrieval_id TEXT PRIMARY KEY, retrieved_at TEXT NOT NULL, query TEXT NOT NULL,
                outcome_label TEXT, outcome_observed_at TEXT, recognition_verdict TEXT,
                familiarity REAL, cited_memory_ids TEXT NOT NULL DEFAULT '[]',
                injected_memory_ids TEXT, injected_memory_ids_source TEXT,
                citation_checked_at TEXT
             );
             CREATE TABLE recognition_set_members (
                retrieval_id TEXT NOT NULL, memory_id TEXT NOT NULL,
                rank INTEGER, signal_score REAL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recognition_events VALUES
             ('r1', '2026-07-01T00:00:00.000Z', 'private query', 'useful',
              '2026-07-01T00:01:00.000Z', NULL, NULL, '[\"m2\"]', '[\"m2\"]',
              'recorded', '2026-07-01T00:00:30.000Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recognition_set_members VALUES ('r1', 'm2', 1, 0.8)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recognition_set_members VALUES ('r1', 'm1', 0, 0.9)",
            [],
        )
        .unwrap();
        (temp, db)
    }

    #[test]
    fn exports_ranked_contract_and_redacts_query() {
        let (temp, db) = fixture();
        let out = temp.path().join("out");
        let counts = export_spectral_replay(&db, &out, true, None).unwrap();
        assert_eq!(counts, (1, 1));

        let recognition =
            std::io::BufReader::new(File::open(out.join("recognition.jsonl")).unwrap())
                .lines()
                .next()
                .unwrap()
                .unwrap();
        let recognition: serde_json::Value = serde_json::from_str(&recognition).unwrap();
        assert!(recognition["memory_id"].is_null());
        assert!(recognition.get("rank").is_none());
        assert!(recognition.get("signal_score").is_none());

        let recall = std::io::BufReader::new(File::open(out.join("recall.jsonl")).unwrap())
            .lines()
            .next()
            .unwrap()
            .unwrap();
        let recall: serde_json::Value = serde_json::from_str(&recall).unwrap();
        assert!(recall.get("query").is_none());
        assert_eq!(recall["returned"], serde_json::json!(["m1", "m2"]));
        assert_eq!(recall["ranks"], serde_json::json!([0, 1]));
        assert_eq!(recall["used"], serde_json::json!(["m2"]));
        assert_eq!(recall["injected_source"], "recorded");
        assert_eq!(recall["citation_checked_at"], "2026-07-01T00:00:30.000Z");
        assert_eq!(recall["probe_hash"], probe_hash("private query"));

        let manifest: serde_json::Value =
            serde_json::from_reader(File::open(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["counts"]["recognition_events"], 1);
        assert_eq!(manifest["counts"]["recognition_jsonl_rows"], 1);
        assert_eq!(manifest["counts"]["recognition_set_members"], 2);
        assert_eq!(
            manifest["counts"]["injected_memory_ids_sources"]["recorded"],
            1
        );
        assert_eq!(manifest["counts"]["citation_checked_events"], 1);
        assert_eq!(manifest["null_fields"][0]["null_by_construction"], true);
    }

    #[test]
    fn since_is_inclusive_and_query_is_present_by_default() {
        let (temp, db) = fixture();
        let out = temp.path().join("out");
        export_spectral_replay(&db, &out, false, Some("2026-07-01T00:00:00.000Z")).unwrap();
        let recall = fs::read_to_string(out.join("recall.jsonl")).unwrap();
        assert!(recall.contains("\"query\":\"private query\""));

        let empty = temp.path().join("empty");
        export_spectral_replay(&db, &empty, false, Some("2026-07-02T00:00:00.000Z")).unwrap();
        assert_eq!(fs::read_to_string(empty.join("recall.jsonl")).unwrap(), "");
    }

    #[test]
    fn unmeasured_usage_exports_as_null_and_is_reported_in_manifest() {
        let (temp, db) = fixture();
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE recognition_events
                SET cited_memory_ids = '[]', citation_checked_at = NULL",
            [],
        )
        .unwrap();
        drop(conn);

        let out = temp.path().join("out");
        export_spectral_replay(&db, &out, false, None).unwrap();

        let recall = std::io::BufReader::new(File::open(out.join("recall.jsonl")).unwrap())
            .lines()
            .next()
            .unwrap()
            .unwrap();
        let recall: serde_json::Value = serde_json::from_str(&recall).unwrap();
        assert!(recall["used"].is_null());
        assert!(recall["citation_checked_at"].is_null());

        let manifest: serde_json::Value =
            serde_json::from_reader(File::open(out.join("manifest.json")).unwrap()).unwrap();
        let used = manifest["null_fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["field"] == "recall.used")
            .unwrap();
        assert_eq!(used["null_rows"], 1);
        assert_eq!(used["total_rows"], 1);
        assert_eq!(used["null_by_construction"], true);
    }

    #[test]
    fn validates_since_as_rfc3339() {
        assert!(normalize_since("last-week").is_err());
        assert_eq!(
            normalize_since("2026-07-01T01:00:00+01:00").unwrap(),
            "2026-07-01T00:00:00.000Z"
        );
    }
}
