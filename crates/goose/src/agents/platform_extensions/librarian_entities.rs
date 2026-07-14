//! #387 v2 — full-coverage, evidence-grounded entity descriptions.
//!
//! The v1 entity pass (`describe_entities_batch`, removed) had three gaps:
//! it only saw the persona's 2-hop recall neighborhood, it grounded prompts
//! in name + typed fields alone (inviting invention), and it never revisited
//! a description once written. This module replaces it with a sweep that:
//!
//! 1. **Enumerates every knowledge-graph item** as the union of: people with
//!    a `graph_entity_id` bridge key, projects with an ontology graph
//!    identity, every annotated term/category from `memory_annotations.who`,
//!    and the persona-neighborhood catch-all (locations, organizations…).
//!    Everything is deduped by content-addressed `EntityId` and
//!    existence-checked against the Kuzu store — Spectral's
//!    `set_entity_description` MERGEs, so describing an unmaterialized id
//!    would mint a half-formed phantom node.
//! 2. **Grounds generation in evidence**: typed entity fields, recent memory
//!    excerpts that mention the entity (annotation join, content fallback),
//!    and project context (the project's own record, or which projects'
//!    memories mention the entity). The model is contractually forbidden to
//!    invent and must emit `INSUFFICIENT_CONTEXT` when the evidence can't
//!    support one true sentence.
//! 3. **Keeps descriptions fresh** via a sidecar JSON ledger
//!    (`~/.permagent/librarian-entity-state.json`, no DB migration): each
//!    entity's evidence fingerprint is recorded at describe time; when the
//!    live fingerprint drifts, the entity is re-described (capped per sweep).
//! 4. **Asks the user when evidence is thin**: entities that hit
//!    `INSUFFICIENT_CONTEXT` despite real signal (≥3 mentioning memories)
//!    land in the ledger's `needs_context` list, surfaced through the
//!    Librarian's live state (`… K awaiting your context`) and a descriptor
//!    sentence telling the agent to ask about them in conversation and save
//!    the answer to memory — the next sweep picks the answer up as fresh
//!    evidence and closes the loop.
//!
//! v2 asking surface (deferred): a Decision-Inbox `context_request` kind is
//! the structured follow-up, deferred until #681 (which owns `decisions.rs`)
//! lands.
//!
//! Rides the same warm-Ollama window as the memory describe batch
//! ([`super::librarian::run_batch`]) — see `warm_and_run` in
//! `goose-server/src/routes/librarian/scheduling.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::librarian::{call_ollama_streaming, OLLAMA_BASE_URL};
use super::librarian_state;
use crate::brain_handle::{GraphEntitySnapshot, OntologyEntityResolution, SafeBrain};

// ── Sweep caps ──────────────────────────────────────────────────────────────

/// New/undescribed entities described per sweep.
pub const DESCRIBE_CAP: usize = 30;
/// Stale (evidence drifted) entities re-described per sweep — bounded so
/// freshness never hogs the warm window from first-time coverage.
pub const REDESCRIBE_CAP: usize = 10;
/// Persona-neighborhood catch-all size (source d).
const NEIGHBORHOOD_CAP: usize = 40;
/// Memory excerpts included in a prompt.
const MAX_EXCERPTS: usize = 6;
/// Characters kept per memory excerpt.
const EXCERPT_CHARS: usize = 200;
/// Mention memory ids folded into the evidence fingerprint (sanity bound).
const MAX_FINGERPRINT_MENTIONS: usize = 500;
/// Minimum mentioning memories for an INSUFFICIENT_CONTEXT entity to be
/// queued for the user ("signal" — worth their attention).
const NEEDS_CONTEXT_MIN_MENTIONS: usize = 3;
/// Ledger `needs_context` list cap (keeps the ask queue and brief sane).
const NEEDS_CONTEXT_CAP: usize = 20;
/// Hard cap on a stored description.
const MAX_DESCRIPTION_CHARS: usize = 500;
/// Minimum name length for the content-LIKE excerpt fallback (short names
/// substring-match too promiscuously to be trusted as evidence).
const MIN_LIKE_NAME_LEN: usize = 4;

/// The exact refusal token the generation contract demands.
pub(crate) const INSUFFICIENT_CONTEXT_TOKEN: &str = "INSUFFICIENT_CONTEXT";

/// System prompt for the entity pass. Deliberately *not*
/// [`super::librarian::LIBRARIAN_SYSTEM_PROMPT`] (the three-field memory
/// format) — v1 inherited that and it fought the entity instruction.
const ENTITY_SYSTEM_PROMPT: &str = "You write short, factual descriptions of knowledge-graph \
     entities (people, projects, topics, tools, places) for their entity cards. Ground every \
     statement strictly in the evidence provided in the prompt. Never invent, embellish, or \
     guess. If the evidence does not support at least one true statement beyond the entity's \
     name and type, reply with exactly: INSUFFICIENT_CONTEXT";

// ── Ledger (sidecar JSON — no DB migration; the v26 lane is owned elsewhere) ─

/// Per-entity freshness record. `evidence_fingerprint` is a hash of the
/// evidence *keys* (mention memory ids, typed field pairs, project links)
/// seen at the last describe attempt; drift ⇒ stale ⇒ re-describe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerEntry {
    /// RFC3339 of the last successful description write. `None` when the last
    /// attempt ended in INSUFFICIENT_CONTEXT (attempted, not described).
    #[serde(default)]
    pub described_at: Option<String>,
    pub evidence_fingerprint: String,
}

/// An entity the Librarian could not truthfully describe despite real signal
/// — the ask-the-user queue (v1 asking surface: the capability brief; v2, a
/// Decision-Inbox `context_request`, is deferred until #681 lands).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NeedsContext {
    pub canonical: String,
    pub entity_type: String,
    /// Memories that mention it (the "signal" that makes it worth asking).
    pub mentions: usize,
    /// Why the evidence was too thin, for the agent to phrase its question.
    pub why_thin: String,
}

/// Sidecar state at `~/.permagent/librarian-entity-state.json`. Missing or
/// corrupt ⇒ start fresh (the sweep is idempotent; worst case is one round of
/// re-describes).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityLedger {
    /// Keyed by the entity's bare 64-hex `EntityId`.
    #[serde(default)]
    pub entities: HashMap<String, LedgerEntry>,
    #[serde(default)]
    pub needs_context: Vec<NeedsContext>,
}

fn ledger_path() -> Option<PathBuf> {
    // Mirrors the Watcher's Budget persistence (proactive.rs): HOME-anchored
    // dotdir file, tolerant of absence.
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".permagent")
            .join("librarian-entity-state.json")
    })
}

fn load_ledger_from(path: &Path) -> EntityLedger {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Atomic-ish save: write a sibling temp file, then rename over the target so
/// a crash mid-write can't leave a half-written ledger.
fn save_ledger_to(path: &Path, ledger: &EntityLedger) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(ledger)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Re-seed the live `K awaiting your context` count from the ledger at daemon
/// boot, so the ask-seam survives restarts instead of waiting for the next
/// nightly sweep. Called from the librarian scheduler loop's startup.
pub fn restore_awaiting_context_state() {
    let Some(path) = ledger_path() else { return };
    let ledger = load_ledger_from(&path);
    librarian_state::set_entities_awaiting_context(ledger.needs_context.len());
}

// ── Evidence fingerprint ────────────────────────────────────────────────────

/// Hash of the evidence keys (order-independent, deduped). Sixteen bytes of
/// blake3 — collision-safe for a freshness check.
fn evidence_fingerprint(mut keys: Vec<String>) -> String {
    keys.sort();
    keys.dedup();
    let mut hasher = blake3::Hasher::new();
    for k in &keys {
        hasher.update(k.as_bytes());
        hasher.update(&[0u8]);
    }
    hex::encode(&hasher.finalize().as_bytes()[..16])
}

/// The evidence keys that define freshness. Content-LIKE fallback excerpts are
/// deliberately excluded: computing them for every entity would mean a full
/// table scan per entity per sweep, and the ask→answer→memory loop closes
/// through *annotations* anyway (the answer memory gets described, its terms
/// annotated, and the mention set — which IS fingerprinted — changes).
fn fingerprint_keys(
    fields: &[(String, String)],
    mention_ids: &[String],
    project_row: Option<&ProjectRow>,
    active_projects: &[String],
) -> Vec<String> {
    let mut keys = Vec::new();
    for (name, value) in fields {
        keys.push(format!("f:{name}={value}"));
    }
    for id in mention_ids.iter().take(MAX_FINGERPRINT_MENTIONS) {
        keys.push(format!("m:{id}"));
    }
    if let Some(p) = project_row {
        keys.push(format!(
            "p:{}|{}|{}|{}",
            p.name, p.description, p.status, p.notes
        ));
    }
    for name in active_projects {
        keys.push(format!("ap:{name}"));
    }
    keys
}

// ── App-DB rows (permagent.db, read-only) ───────────────────────────────────

#[derive(Debug, Clone, Default)]
struct ProjectRow {
    name: String,
    description: String,
    status: String,
    notes: String,
}

#[derive(Debug, Default)]
struct AppRows {
    /// (display_name, graph_entity_id hex) for every bridged person.
    people: Vec<(String, String)>,
    projects: Vec<ProjectRow>,
    /// memory_id → project names it is associated with (project_memories).
    memory_projects: HashMap<String, Vec<String>>,
}

fn open_ro(path: &Path) -> Option<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

fn load_app_rows_blocking() -> AppRows {
    let mut out = AppRows::default();
    let Some(conn) = open_ro(&crate::config::paths::Paths::spectral_db()) else {
        tracing::debug!(target: "permagentd::librarian", "no permagent.db — entity sweep runs without people/projects sources");
        return out;
    };

    if let Ok(mut stmt) = conn.prepare(
        "SELECT display_name, graph_entity_id FROM people \
         WHERE graph_entity_id IS NOT NULL AND graph_entity_id != ''",
    ) {
        if let Ok(rows) =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            out.people = rows.flatten().collect();
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT name, COALESCE(description, ''), COALESCE(status, ''), COALESCE(notes, '') \
         FROM projects",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(ProjectRow {
                name: r.get(0)?,
                description: r.get(1)?,
                status: r.get(2)?,
                notes: r.get(3)?,
            })
        }) {
            out.projects = rows.flatten().collect();
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT pm.memory_id, p.name FROM project_memories pm \
         JOIN projects p ON p.id = pm.project_id",
    ) {
        if let Ok(rows) =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            for (mem_id, name) in rows.flatten() {
                out.memory_projects.entry(mem_id).or_default().push(name);
            }
        }
    }

    out
}

// ── Annotation mention index (memory.db, read-only) ─────────────────────────

/// Which memories mention which entity name, from `memory_annotations.who`
/// canonical_ids (`term:x` / `cat:x` — the Librarian's annotation pipeline is
/// the sole writer of that table). Names are normalized with
/// [`crate::identity::canonical::graph_canonical`] so they line up with graph
/// canonicals and ontology aliases.
#[derive(Debug, Default)]
struct MentionIndex {
    /// normalized name → (created_at, memory_id), sorted recent-first, deduped.
    by_name: HashMap<String, Vec<(String, String)>>,
}

impl MentionIndex {
    fn insert(&mut self, name: &str, created_at: &str, mem_id: &str) {
        self.by_name
            .entry(name.to_string())
            .or_default()
            .push((created_at.to_string(), mem_id.to_string()));
    }

    fn finalize(&mut self) {
        for v in self.by_name.values_mut() {
            // ISO-ish created_at strings sort chronologically as strings.
            v.sort_by(|a, b| b.0.cmp(&a.0));
            let mut seen = HashSet::new();
            v.retain(|(_, id)| seen.insert(id.clone()));
        }
    }

    /// Memory ids mentioning any of `names`, most recent first, deduped.
    fn ids_for(&self, names: &BTreeSet<String>) -> Vec<String> {
        let mut merged: Vec<&(String, String)> = names
            .iter()
            .filter_map(|n| self.by_name.get(n))
            .flatten()
            .collect();
        merged.sort_by(|a, b| b.0.cmp(&a.0));
        let mut seen = HashSet::new();
        merged
            .into_iter()
            .filter(|(_, id)| seen.insert(id.clone()))
            .map(|(_, id)| id.clone())
            .collect()
    }
}

/// A canonical_id that is really an opaque id, not a name (mirrors the
/// Watcher's `aggregate_subjects` filter in proactive.rs).
fn looks_like_opaque_id(name: &str) -> bool {
    name.starts_with("e:") || (name.len() >= 16 && name.chars().all(|c| c.is_ascii_hexdigit()))
}

fn load_mention_index_blocking() -> MentionIndex {
    use crate::identity::canonical::graph_canonical;
    let mut index = MentionIndex::default();
    let db = crate::config::paths::Paths::brain_dir().join("memory.db");
    let Some(conn) = open_ro(&db) else {
        tracing::debug!(target: "permagentd::librarian", "no memory.db — entity sweep runs without annotation mentions");
        return index;
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT m.id, m.created_at, a.who \
         FROM memories m JOIN memory_annotations a ON a.memory_id = m.id \
         WHERE m.created_at IS NOT NULL AND a.who IS NOT NULL",
    ) else {
        return index;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) else {
        return index;
    };
    for (mem_id, created_at, who_json) in rows.flatten() {
        let Ok(refs) = serde_json::from_str::<Vec<serde_json::Value>>(&who_json) else {
            continue;
        };
        for r in refs {
            let Some(cid) = r.get("canonical_id").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(name) = cid
                .strip_prefix("term:")
                .or_else(|| cid.strip_prefix("cat:"))
            else {
                continue;
            };
            let normalized = graph_canonical(name);
            if normalized.is_empty() || looks_like_opaque_id(&normalized) {
                continue;
            }
            index.insert(&normalized, &created_at, &mem_id);
        }
    }
    index.finalize();
    index
}

// ── Excerpt fetch (memory.db, read-only; selected entities only) ────────────

#[derive(Debug, Clone)]
struct Excerpt {
    created_at: String,
    text: String,
}

/// One line of usable evidence from a memory: single-line, char-capped.
fn trim_excerpt(content: &str) -> String {
    let joined = content.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(EXCERPT_CHARS).collect()
}

/// Fetch prompt excerpts for the selected work items. `by_ids` are annotation
/// mention hits (recent-first); items with none fall back to a bounded
/// content-LIKE scan (people/projects are named in memory *content* far more
/// often than they surface as annotation terms).
fn fetch_excerpts_blocking(
    requests: Vec<(usize, Vec<String>, Option<String>)>,
) -> HashMap<usize, Vec<Excerpt>> {
    let mut out: HashMap<usize, Vec<Excerpt>> = HashMap::new();
    let db = crate::config::paths::Paths::brain_dir().join("memory.db");
    let Some(conn) = open_ro(&db) else { return out };

    for (idx, ids, like_name) in requests {
        let mut excerpts: Vec<(String, String)> = Vec::new();
        if !ids.is_empty() {
            let take: Vec<&String> = ids.iter().take(MAX_EXCERPTS).collect();
            let placeholders = std::iter::repeat_n("?", take.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql =
                format!("SELECT content, created_at FROM memories WHERE id IN ({placeholders})");
            if let Ok(mut stmt) = conn.prepare(&sql) {
                if let Ok(rows) = stmt
                    .query_map(rusqlite::params_from_iter(take.iter().copied()), |r| {
                        Ok((r.get::<_, String>(1)?, r.get::<_, String>(0)?))
                    })
                {
                    excerpts.extend(rows.flatten());
                }
            }
        } else if let Some(name) = like_name {
            if name.chars().count() >= MIN_LIKE_NAME_LEN {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT content, created_at FROM memories \
                     WHERE content LIKE '%' || ?1 || '%' \
                     ORDER BY created_at DESC LIMIT ?2",
                ) {
                    if let Ok(rows) = stmt
                        .query_map(rusqlite::params![name, MAX_EXCERPTS as i64], |r| {
                            Ok((r.get::<_, String>(1)?, r.get::<_, String>(0)?))
                        })
                    {
                        excerpts.extend(rows.flatten());
                    }
                }
            }
        }
        excerpts.sort_by(|a, b| b.0.cmp(&a.0));
        out.insert(
            idx,
            excerpts
                .into_iter()
                .take(MAX_EXCERPTS)
                .map(|(created_at, content)| Excerpt {
                    created_at: created_at.chars().take(10).collect(), // date part
                    text: trim_excerpt(&content),
                })
                .collect(),
        );
    }
    out
}

// ── Worklist ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct WorkItem {
    snapshot: GraphEntitySnapshot,
    /// Normalized names/aliases used to join against the mention index.
    mention_names: BTreeSet<String>,
    /// Set when the entity IS a tracked project (its own record is evidence).
    project_row: Option<ProjectRow>,
}

/// Merge a candidate into the worklist, deduping by content-addressed
/// `EntityId` (hex). A collision unions mention names and keeps the first
/// project row — the same graph node reached from two sources is one entity.
fn merge_into_worklist(
    worklist: &mut BTreeMap<String, WorkItem>,
    snapshot: GraphEntitySnapshot,
    names: impl IntoIterator<Item = String>,
    project_row: Option<ProjectRow>,
) {
    use crate::identity::canonical::graph_canonical;
    let key = snapshot.id.to_string();
    let entry = worklist.entry(key).or_insert_with(|| WorkItem {
        mention_names: BTreeSet::new(),
        project_row: None,
        snapshot,
    });
    // Own the string first so the shared borrow of `entry.snapshot` is released
    // before the mutable borrow of `entry.mention_names`.
    let self_name = graph_canonical(&entry.snapshot.canonical);
    entry.mention_names.insert(self_name);
    for n in names {
        let normalized = graph_canonical(&n);
        if !normalized.is_empty() {
            entry.mention_names.insert(normalized);
        }
    }
    if entry.project_row.is_none() {
        entry.project_row = project_row;
    }
}

// ── Selection (new/undescribed first, then stale; capped) ───────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkKind {
    Describe,
    Redescribe,
}

#[derive(Debug, Clone)]
struct SelectionCandidate {
    hex: String,
    has_description: bool,
    fingerprint: String,
    mentions: usize,
}

/// Freshness rules:
/// - undescribed + (no ledger entry OR fingerprint changed) → Describe.
///   (An unchanged fingerprint after a previous attempt means the evidence
///   hasn't moved — retrying would burn the window on the same thin input.)
/// - described + (no ledger entry OR fingerprint changed) → Redescribe.
///   (No-entry covers v1's name-grounded descriptions: they upgrade to
///   evidence-grounded ones over successive sweeps.)
///
/// Both buckets are ordered most-mentioned-first (hex tiebreak, deterministic)
/// and capped independently.
fn plan_selection(
    candidates: &[SelectionCandidate],
    ledger: &EntityLedger,
    describe_cap: usize,
    redescribe_cap: usize,
) -> Vec<(usize, WorkKind)> {
    let mut describe: Vec<usize> = Vec::new();
    let mut redescribe: Vec<usize> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let entry = ledger.entities.get(&c.hex);
        let changed = entry.is_none_or(|e| e.evidence_fingerprint != c.fingerprint);
        if !changed {
            continue;
        }
        if c.has_description {
            redescribe.push(i);
        } else {
            describe.push(i);
        }
    }
    let by_signal = |a: &usize, b: &usize| {
        candidates[*b]
            .mentions
            .cmp(&candidates[*a].mentions)
            .then_with(|| candidates[*a].hex.cmp(&candidates[*b].hex))
    };
    describe.sort_by(by_signal);
    redescribe.sort_by(by_signal);
    describe
        .into_iter()
        .take(describe_cap)
        .map(|i| (i, WorkKind::Describe))
        .chain(
            redescribe
                .into_iter()
                .take(redescribe_cap)
                .map(|i| (i, WorkKind::Redescribe)),
        )
        .collect()
}

// ── Prompt + output handling ────────────────────────────────────────────────

/// The generation contract. Every claim must trace to an evidence line; a
/// thin evidence set must yield the refusal token, never a bluff.
fn build_entity_prompt(
    entity_type: &str,
    canonical: &str,
    fields: &[(String, String)],
    excerpts: &[Excerpt],
    project_row: Option<&ProjectRow>,
    active_projects: &[String],
) -> String {
    let mut evidence = String::new();
    if let Some(p) = project_row {
        evidence.push_str("Project record:\n");
        evidence.push_str(&format!("- name: {}\n", p.name));
        if !p.status.is_empty() {
            evidence.push_str(&format!("- status: {}\n", p.status));
        }
        if !p.description.is_empty() {
            evidence.push_str(&format!("- summary: {}\n", trim_excerpt(&p.description)));
        }
        if !p.notes.is_empty() {
            evidence.push_str(&format!("- notes: {}\n", trim_excerpt(&p.notes)));
        }
    }
    if !fields.is_empty() {
        evidence.push_str("Known fields:\n");
        for (name, value) in fields {
            evidence.push_str(&format!("- {name}: {value}\n"));
        }
    }
    if !active_projects.is_empty() {
        evidence.push_str(&format!(
            "Active in project(s): {}\n",
            active_projects.join(", ")
        ));
    }
    if !excerpts.is_empty() {
        evidence.push_str("Memory excerpts that mention it (most recent first):\n");
        for e in excerpts {
            evidence.push_str(&format!("- [{}] {}\n", e.created_at, e.text));
        }
    }

    format!(
        "Write 1-2 sentences describing this {entity_type} for its knowledge-graph card, \
         USING ONLY the evidence below. Do not invent facts. If the evidence is too thin to \
         say anything true beyond the name, output exactly {INSUFFICIENT_CONTEXT_TOKEN}.\n\n\
         Name: {canonical}\nType: {entity_type}\n\nEvidence:\n{evidence}"
    )
}

/// Did the model decline (or effectively decline)? Conservative on purpose:
/// a hedging non-answer stored as a card is worse than no card.
fn is_insufficient_context(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    let upper = trimmed.to_uppercase();
    upper.contains(INSUFFICIENT_CONTEXT_TOKEN)
        || upper.contains("NOT ENOUGH INFORMATION")
        || upper.contains("NO EVIDENCE PROVIDED")
        || upper.contains("EVIDENCE IS INSUFFICIENT")
}

/// Single-line, unquoted, hard-capped description text.
fn sanitize_description(raw: &str) -> String {
    let joined = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = joined.trim().trim_matches('"').trim();
    trimmed.chars().take(MAX_DESCRIPTION_CHARS).collect()
}

// ── needs_context maintenance ───────────────────────────────────────────────

/// Drop entries that got resolved (a description now exists), upsert this
/// sweep's new thin-evidence entries (dedupe by canonical), order by signal,
/// cap the list.
fn update_needs_context(
    list: &mut Vec<NeedsContext>,
    resolved_canonicals: &HashSet<String>,
    new_entries: Vec<NeedsContext>,
) {
    list.retain(|e| !resolved_canonicals.contains(&e.canonical));
    for entry in new_entries {
        if let Some(existing) = list.iter_mut().find(|e| e.canonical == entry.canonical) {
            *existing = entry;
        } else {
            list.push(entry);
        }
    }
    list.sort_by(|a, b| {
        b.mentions
            .cmp(&a.mentions)
            .then_with(|| a.canonical.cmp(&b.canonical))
    });
    list.truncate(NEEDS_CONTEXT_CAP);
}

// ── Sweep summary ───────────────────────────────────────────────────────────

/// Observable outcome of one sweep (logged; asserted against on the mini).
#[derive(Debug, Default, Clone, Serialize)]
pub struct SweepSummary {
    /// Entities considered after dedupe + existence checks.
    pub worklist: usize,
    /// First-time descriptions written.
    pub described: usize,
    /// Stale descriptions refreshed.
    pub redescribed: usize,
    /// Attempts that ended in INSUFFICIENT_CONTEXT (nothing written).
    pub insufficient: usize,
    /// Projects with no ontology/graph identity (#595 tracks giving them one).
    pub skipped_no_graph_identity: usize,
    /// Ontology-known ids whose node was never materialized in Kuzu
    /// (describing them would MERGE a half-formed node — hard skip).
    pub skipped_not_in_graph: usize,
    /// Annotated terms/categories with no ontology identity (SQLite-shadow
    /// only — no graph card exists for them).
    pub unresolved_terms: usize,
    /// `needs_context` length after this sweep (the brief's K).
    pub awaiting_context: usize,
}

// ── The sweep ───────────────────────────────────────────────────────────────

/// #387 v2 entry point — replaces `describe_entities_batch` in
/// `warm_and_run`. Enumerate → evidence → select (new first, stale second)
/// → generate under the truthfulness contract → persist ledger + live state.
pub async fn run_entity_sweep(brain: &SafeBrain, model: &str) -> Result<SweepSummary, String> {
    use crate::identity::canonical::graph_canonical;

    let mut summary = SweepSummary::default();

    // 1. Enumeration inputs (read-only SQLite off the executor).
    let app_rows = tokio::task::spawn_blocking(load_app_rows_blocking)
        .await
        .map_err(|e| format!("app-db task panicked: {e}"))?;
    let mentions = tokio::task::spawn_blocking(load_mention_index_blocking)
        .await
        .map_err(|e| format!("mention task panicked: {e}"))?;

    // (a) People: bridge keys are authoritative — existence-checked in batch.
    let people_hexes: Vec<String> = app_rows.people.iter().map(|(_, hex)| hex.clone()).collect();
    let people_snaps = brain
        .entity_snapshots_by_hex(people_hexes)
        .await
        .map_err(|e| format!("Brain error (people snapshots): {e}"))?;

    // (b)+(c) Projects and annotated terms/categories resolve through the
    // ontology (exact alias match only) to the exact (type, canonical) pair
    // the graph hashed into each EntityId — the verified reconstruction path.
    let project_names: Vec<String> = app_rows.projects.iter().map(|p| p.name.clone()).collect();
    let term_names: Vec<String> = mentions.by_name.keys().cloned().collect();
    let mut names = project_names.clone();
    names.extend(term_names.iter().cloned());
    let resolutions = brain
        .resolve_ontology_entities_exact(names)
        .await
        .map_err(|e| format!("Brain error (ontology resolution): {e}"))?;
    let (project_res, term_res) = resolutions.split_at(project_names.len());

    // (d) Persona-neighborhood catch-all (locations etc. no table enumerates).
    let seed = crate::config::agent_identity::load_agent_config()
        .primary
        .first_name;
    let neighborhood = brain
        .neighborhood_entity_snapshots(&seed, NEIGHBORHOOD_CAP)
        .await
        .unwrap_or_default();

    // 2. Assemble the deduped worklist.
    let mut worklist: BTreeMap<String, WorkItem> = BTreeMap::new();
    for ((display_name, _), snap) in app_rows.people.iter().zip(people_snaps) {
        match snap {
            Some(s) => merge_into_worklist(&mut worklist, s, [display_name.clone()], None),
            None => summary.skipped_not_in_graph += 1,
        }
    }
    for (row, res) in app_rows.projects.iter().zip(project_res) {
        match res {
            OntologyEntityResolution::InGraph { snapshot, aliases } => merge_into_worklist(
                &mut worklist,
                snapshot.clone(),
                aliases.iter().cloned().chain([row.name.clone()]),
                Some(row.clone()),
            ),
            // Ontology-known but never materialized — a hard skip (describing it
            // would MERGE a half-formed node). Counted once, under its own
            // bucket; NOT under skipped_no_graph_identity (that is #595's
            // no-ontology-identity case).
            OntologyEntityResolution::NotInGraph => summary.skipped_not_in_graph += 1,
            OntologyEntityResolution::NoIdentity => summary.skipped_no_graph_identity += 1,
        }
    }
    for (name, res) in term_names.iter().zip(term_res) {
        match res {
            OntologyEntityResolution::InGraph { snapshot, aliases } => merge_into_worklist(
                &mut worklist,
                snapshot.clone(),
                aliases.iter().cloned().chain([name.clone()]),
                None,
            ),
            OntologyEntityResolution::NotInGraph => summary.skipped_not_in_graph += 1,
            OntologyEntityResolution::NoIdentity => summary.unresolved_terms += 1,
        }
    }
    for snap in neighborhood {
        merge_into_worklist(&mut worklist, snap, std::iter::empty::<String>(), None);
    }

    let items: Vec<WorkItem> = worklist.into_values().collect();
    summary.worklist = items.len();

    // 3. Evidence keys for every item (typed fields in one blocking hop;
    //    mention ids from the in-memory index) → current fingerprints.
    let all_ids: Vec<spectral::core::entity_id::EntityId> =
        items.iter().map(|i| i.snapshot.id).collect();
    let mut fields_map = brain.entity_fields_for(all_ids).await.unwrap_or_default();

    let mut fields_by_item: Vec<Vec<(String, String)>> = Vec::with_capacity(items.len());
    let mut mention_ids_by_item: Vec<Vec<String>> = Vec::with_capacity(items.len());
    let mut active_projects_by_item: Vec<Vec<String>> = Vec::with_capacity(items.len());
    let mut candidates: Vec<SelectionCandidate> = Vec::with_capacity(items.len());
    for item in &items {
        let fields: Vec<(String, String)> = fields_map
            .remove(&item.snapshot.id.to_string())
            .unwrap_or_default()
            .into_iter()
            .map(|f| (f.field_name, f.value))
            .collect();
        let mention_ids = mentions.ids_for(&item.mention_names);
        let mut active: Vec<String> = mention_ids
            .iter()
            .filter_map(|id| app_rows.memory_projects.get(id))
            .flatten()
            .cloned()
            .collect();
        active.sort();
        active.dedup();
        // An entity is not "active in" itself.
        active.retain(|p| graph_canonical(p) != graph_canonical(&item.snapshot.canonical));

        let fingerprint = evidence_fingerprint(fingerprint_keys(
            &fields,
            &mention_ids,
            item.project_row.as_ref(),
            &active,
        ));
        candidates.push(SelectionCandidate {
            hex: item.snapshot.id.to_string(),
            has_description: item.snapshot.description.is_some(),
            fingerprint,
            mentions: mention_ids.len(),
        });
        fields_by_item.push(fields);
        mention_ids_by_item.push(mention_ids);
        active_projects_by_item.push(active);
    }

    // 4. Plan against the ledger; fetch excerpts for the selected only.
    let ledger_file = ledger_path();
    let mut ledger = ledger_file
        .as_deref()
        .map(load_ledger_from)
        .unwrap_or_default();
    let selection = plan_selection(&candidates, &ledger, DESCRIBE_CAP, REDESCRIBE_CAP);

    let excerpt_requests: Vec<(usize, Vec<String>, Option<String>)> = selection
        .iter()
        .map(|(i, _)| {
            (
                *i,
                mention_ids_by_item[*i]
                    .iter()
                    .take(MAX_EXCERPTS)
                    .cloned()
                    .collect(),
                Some(items[*i].snapshot.canonical.clone()),
            )
        })
        .collect();
    let mut excerpts_map =
        tokio::task::spawn_blocking(move || fetch_excerpts_blocking(excerpt_requests))
            .await
            .map_err(|e| format!("excerpt task panicked: {e}"))?;

    // 5. Generate under the truthfulness contract.
    let mut resolved_canonicals: HashSet<String> = HashSet::new();
    let mut new_needs_context: Vec<NeedsContext> = Vec::new();
    'generation: for (i, kind) in selection {
        let item = &items[i];
        let fields = &fields_by_item[i];
        let excerpts = excerpts_map.remove(&i).unwrap_or_default();
        let active = &active_projects_by_item[i];
        let mentions_count = candidates[i].mentions;
        let fingerprint = candidates[i].fingerprint.clone();
        let hex = candidates[i].hex.clone();

        let no_evidence = fields.is_empty()
            && excerpts.is_empty()
            && item.project_row.is_none()
            && active.is_empty();

        let outcome: Option<String> = if no_evidence {
            None // nothing true can be said — skip the model call entirely
        } else {
            let prompt = build_entity_prompt(
                &item.snapshot.entity_type,
                &item.snapshot.canonical,
                fields,
                &excerpts,
                item.project_row.as_ref(),
                active,
            );
            match call_ollama_streaming(
                OLLAMA_BASE_URL,
                ENTITY_SYSTEM_PROMPT,
                &prompt,
                model,
                false,
                &item.snapshot.canonical,
            )
            .await
            {
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::librarian",
                        entity = %item.snapshot.canonical,
                        error = %e,
                        "entity sweep: Ollama unavailable — stopping generation for this sweep"
                    );
                    break 'generation;
                }
                Ok(raw) if is_insufficient_context(&raw) => None,
                Ok(raw) => {
                    let d = sanitize_description(&raw);
                    if d.is_empty() {
                        None
                    } else {
                        Some(d)
                    }
                }
            }
        };

        match outcome {
            Some(description) => {
                if let Err(e) = brain
                    .set_entity_description(item.snapshot.id, &description)
                    .await
                {
                    tracing::warn!(
                        target: "permagentd::librarian",
                        entity = %item.snapshot.canonical,
                        error = %e,
                        "entity sweep: description write failed — stopping (Spectral issue)"
                    );
                    break 'generation;
                }
                ledger.entities.insert(
                    hex,
                    LedgerEntry {
                        described_at: Some(chrono::Utc::now().to_rfc3339()),
                        evidence_fingerprint: fingerprint,
                    },
                );
                resolved_canonicals.insert(item.snapshot.canonical.clone());
                match kind {
                    WorkKind::Describe => summary.described += 1,
                    WorkKind::Redescribe => summary.redescribed += 1,
                }
                tracing::info!(
                    target: "permagentd::librarian",
                    entity = %item.snapshot.canonical,
                    entity_type = %item.snapshot.entity_type,
                    refreshed = matches!(kind, WorkKind::Redescribe),
                    "entity description written (#387 v2)"
                );
            }
            None => {
                summary.insufficient += 1;
                // Record the fingerprint so the same thin evidence isn't
                // retried every sweep; the entity re-queues the moment its
                // evidence changes (e.g. the user's answer becomes a memory).
                let prior_described_at = ledger
                    .entities
                    .get(&hex)
                    .and_then(|e| e.described_at.clone());
                ledger.entities.insert(
                    hex,
                    LedgerEntry {
                        described_at: prior_described_at,
                        evidence_fingerprint: fingerprint,
                    },
                );
                if matches!(kind, WorkKind::Describe)
                    && mentions_count >= NEEDS_CONTEXT_MIN_MENTIONS
                {
                    let why_thin = if no_evidence {
                        format!(
                            "mentioned in {mentions_count} memories but none carry usable \
                             detail — no typed fields, excerpts, or project links"
                        )
                    } else {
                        format!(
                            "{} typed field(s), {} excerpt(s), {} project link(s) — still \
                             not enough to say anything true beyond the name",
                            fields.len(),
                            excerpts.len(),
                            active.len() + usize::from(item.project_row.is_some()),
                        )
                    };
                    new_needs_context.push(NeedsContext {
                        canonical: item.snapshot.canonical.clone(),
                        entity_type: item.snapshot.entity_type.clone(),
                        mentions: mentions_count,
                        why_thin,
                    });
                }
            }
        }
    }

    // 6. Persist ledger + live state. Entities that already carry a
    //    description also clear any lingering needs_context entry.
    for item in &items {
        if item.snapshot.description.is_some() {
            resolved_canonicals.insert(item.snapshot.canonical.clone());
        }
    }
    update_needs_context(
        &mut ledger.needs_context,
        &resolved_canonicals,
        new_needs_context,
    );
    summary.awaiting_context = ledger.needs_context.len();
    if let Some(path) = ledger_file {
        if let Err(e) = save_ledger_to(&path, &ledger) {
            tracing::warn!(
                target: "permagentd::librarian",
                error = %e,
                "entity sweep: failed to save freshness ledger"
            );
        }
    }
    librarian_state::set_entities_awaiting_context(summary.awaiting_context);

    tracing::info!(
        target: "permagentd::librarian",
        worklist = summary.worklist,
        described = summary.described,
        redescribed = summary.redescribed,
        insufficient = summary.insufficient,
        skipped_no_graph_identity = summary.skipped_no_graph_identity,
        skipped_not_in_graph = summary.skipped_not_in_graph,
        unresolved_terms = summary.unresolved_terms,
        awaiting_context = summary.awaiting_context,
        "Librarian entity sweep complete (#387 v2)"
    );
    Ok(summary)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(entity_type: &str, canonical: &str, description: Option<&str>) -> GraphEntitySnapshot {
        GraphEntitySnapshot {
            id: spectral::core::entity_id::entity_id(entity_type, canonical),
            entity_type: entity_type.to_string(),
            canonical: canonical.to_string(),
            description: description.map(String::from),
        }
    }

    #[test]
    fn fingerprint_is_order_independent_and_change_sensitive() {
        let a = evidence_fingerprint(vec!["m:1".into(), "f:role=eng".into()]);
        let b = evidence_fingerprint(vec!["f:role=eng".into(), "m:1".into()]);
        assert_eq!(a, b, "key order must not matter");

        let c = evidence_fingerprint(vec!["m:1".into(), "m:2".into(), "f:role=eng".into()]);
        assert_ne!(a, c, "a new mention must change the fingerprint");

        // Dedup: the same key twice is the same evidence.
        let d = evidence_fingerprint(vec!["m:1".into(), "m:1".into(), "f:role=eng".into()]);
        assert_eq!(a, d);
    }

    #[test]
    fn fingerprint_keys_cover_all_evidence_kinds() {
        let row = ProjectRow {
            name: "Ladle".into(),
            description: "an app".into(),
            status: "active".into(),
            notes: "".into(),
        };
        let keys = fingerprint_keys(
            &[("role".into(), "engineer".into())],
            &["mem-1".into()],
            Some(&row),
            &["Permagent".into()],
        );
        assert!(keys.contains(&"f:role=engineer".to_string()));
        assert!(keys.contains(&"m:mem-1".to_string()));
        assert!(keys.iter().any(|k| k.starts_with("p:Ladle|")));
        assert!(keys.contains(&"ap:Permagent".to_string()));
    }

    #[test]
    fn plan_selection_applies_staleness_rules_and_caps() {
        let mut ledger = EntityLedger::default();
        // Attempted before on identical evidence → must NOT be retried.
        ledger.entities.insert(
            "attempted-same".into(),
            LedgerEntry {
                described_at: None,
                evidence_fingerprint: "fp-same".into(),
            },
        );
        // Described on old evidence → stale → redescribe.
        ledger.entities.insert(
            "described-stale".into(),
            LedgerEntry {
                described_at: Some("2026-01-01T00:00:00Z".into()),
                evidence_fingerprint: "fp-old".into(),
            },
        );
        // Described on identical evidence → fresh → skip.
        ledger.entities.insert(
            "described-fresh".into(),
            LedgerEntry {
                described_at: Some("2026-01-01T00:00:00Z".into()),
                evidence_fingerprint: "fp-fresh".into(),
            },
        );

        let candidates = vec![
            SelectionCandidate {
                hex: "new-undescribed".into(),
                has_description: false,
                fingerprint: "fp-a".into(),
                mentions: 1,
            },
            SelectionCandidate {
                hex: "attempted-same".into(),
                has_description: false,
                fingerprint: "fp-same".into(),
                mentions: 9,
            },
            SelectionCandidate {
                hex: "described-stale".into(),
                has_description: true,
                fingerprint: "fp-new".into(),
                mentions: 2,
            },
            SelectionCandidate {
                hex: "described-fresh".into(),
                has_description: true,
                fingerprint: "fp-fresh".into(),
                mentions: 5,
            },
            SelectionCandidate {
                hex: "v1-described-no-ledger".into(),
                has_description: true,
                fingerprint: "fp-b".into(),
                mentions: 7,
            },
        ];

        let plan = plan_selection(&candidates, &ledger, 10, 10);
        let picked: Vec<(&str, WorkKind)> = plan
            .iter()
            .map(|(i, k)| (candidates[*i].hex.as_str(), *k))
            .collect();
        assert_eq!(
            picked,
            vec![
                ("new-undescribed", WorkKind::Describe),
                // v1 descriptions (no ledger entry) count as stale — they
                // upgrade to evidence-grounded text over successive sweeps.
                ("v1-described-no-ledger", WorkKind::Redescribe),
                ("described-stale", WorkKind::Redescribe),
            ]
        );

        // Caps bind per bucket, describes ranked by mention signal first.
        let many: Vec<SelectionCandidate> = (0..5)
            .map(|i| SelectionCandidate {
                hex: format!("u{i}"),
                has_description: false,
                fingerprint: "fp".into(),
                mentions: i,
            })
            .collect();
        let capped = plan_selection(&many, &EntityLedger::default(), 2, 1);
        assert_eq!(capped.len(), 2);
        assert_eq!(many[capped[0].0].hex, "u4", "most-mentioned first");
        assert_eq!(many[capped[1].0].hex, "u3");
    }

    #[test]
    fn insufficient_context_parsing() {
        assert!(is_insufficient_context("INSUFFICIENT_CONTEXT"));
        assert!(is_insufficient_context("  \"INSUFFICIENT_CONTEXT\"  "));
        assert!(is_insufficient_context("INSUFFICIENT_CONTEXT.")); // trailing punctuation
        assert!(is_insufficient_context(
            "Output: insufficient_context because the evidence is thin"
        ));
        assert!(is_insufficient_context("")); // empty stream = nothing to store
        assert!(is_insufficient_context(
            "There is not enough information to describe this entity."
        ));
        assert!(!is_insufficient_context(
            "Jesse Sharratt is a software engineer who works on Permagent."
        ));
    }

    #[test]
    fn sanitize_description_flattens_and_caps() {
        assert_eq!(
            sanitize_description("\"A project.\n  Second   line.\"\n"),
            "A project. Second line."
        );
        let long = "x".repeat(2 * MAX_DESCRIPTION_CHARS);
        assert_eq!(
            sanitize_description(&long).chars().count(),
            MAX_DESCRIPTION_CHARS
        );
    }

    #[test]
    fn ledger_round_trips_and_tolerates_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("ledger.json");

        let mut ledger = EntityLedger::default();
        ledger.entities.insert(
            "abc123".into(),
            LedgerEntry {
                described_at: Some("2026-07-13T00:00:00Z".into()),
                evidence_fingerprint: "fp".into(),
            },
        );
        ledger.needs_context.push(NeedsContext {
            canonical: "mel schembri".into(),
            entity_type: "person".into(),
            mentions: 4,
            why_thin: "no usable detail".into(),
        });

        save_ledger_to(&path, &ledger).unwrap();
        let loaded = load_ledger_from(&path);
        assert_eq!(loaded.entities.get("abc123"), ledger.entities.get("abc123"));
        assert_eq!(loaded.needs_context, ledger.needs_context);

        // Corrupt file → fresh default, no panic.
        std::fs::write(&path, "{ not json").unwrap();
        let recovered = load_ledger_from(&path);
        assert!(recovered.entities.is_empty());
        assert!(recovered.needs_context.is_empty());

        // Missing file → default.
        let missing = load_ledger_from(&dir.path().join("nope.json"));
        assert!(missing.entities.is_empty());
    }

    #[test]
    fn worklist_dedupes_by_entity_id_and_unions_mention_names() {
        let mut worklist = BTreeMap::new();
        // Same graph node from two sources: annotation term + project row.
        merge_into_worklist(
            &mut worklist,
            snap("project", "permagent", None),
            ["Permagent".to_string()],
            None,
        );
        merge_into_worklist(
            &mut worklist,
            snap("project", "permagent", None),
            ["permagent runtime".to_string()],
            Some(ProjectRow {
                name: "Permagent".into(),
                ..Default::default()
            }),
        );
        // A different entity stays separate.
        merge_into_worklist(
            &mut worklist,
            snap("person", "jesse sharratt", None),
            std::iter::empty::<String>(),
            None,
        );

        assert_eq!(worklist.len(), 2, "same EntityId must collapse to one item");
        let project_key = snap("project", "permagent", None).id.to_string();
        let item = &worklist[&project_key];
        assert!(item.mention_names.contains("permagent"));
        assert!(item.mention_names.contains("permagent runtime"));
        assert!(
            item.project_row.is_some(),
            "project evidence must survive the merge"
        );
    }

    #[test]
    fn mention_index_orders_recent_first_and_dedupes_across_names() {
        let mut idx = MentionIndex::default();
        idx.insert("rust", "2026-01-01T00:00:00Z", "old");
        idx.insert("rust", "2026-06-01T00:00:00Z", "newer");
        idx.insert("rustlang", "2026-07-01T00:00:00Z", "newest");
        idx.insert("rustlang", "2026-06-01T00:00:00Z", "newer"); // same memory, two aliases
        idx.finalize();

        let names: BTreeSet<String> = ["rust".to_string(), "rustlang".to_string()].into();
        assert_eq!(idx.ids_for(&names), vec!["newest", "newer", "old"]);
    }

    #[test]
    fn needs_context_resolves_upserts_sorts_and_caps() {
        let nc = |canonical: &str, mentions: usize| NeedsContext {
            canonical: canonical.into(),
            entity_type: "concept".into(),
            mentions,
            why_thin: "thin".into(),
        };
        let mut list = vec![nc("described-now", 9), nc("still-thin", 2)];
        let resolved: HashSet<String> = ["described-now".to_string()].into();
        update_needs_context(
            &mut list,
            &resolved,
            vec![nc("still-thin", 5), nc("brand-new", 3)],
        );
        assert_eq!(
            list.iter()
                .map(|e| e.canonical.as_str())
                .collect::<Vec<_>>(),
            vec!["still-thin", "brand-new"],
            "resolved entries drop, upsert refreshes mentions, sorted by signal"
        );
        assert_eq!(list[0].mentions, 5);

        let mut big: Vec<NeedsContext> = (0..NEEDS_CONTEXT_CAP + 10)
            .map(|i| nc(&format!("e{i}"), i))
            .collect();
        update_needs_context(&mut big, &HashSet::new(), vec![]);
        assert_eq!(big.len(), NEEDS_CONTEXT_CAP);
    }

    #[test]
    fn prompt_carries_the_contract_and_the_evidence() {
        let prompt = build_entity_prompt(
            "person",
            "mel schembri",
            &[("role".into(), "designer".into())],
            &[Excerpt {
                created_at: "2026-07-01".into(),
                text: "Mel reviewed the Ladle onboarding flow".into(),
            }],
            None,
            &["Ladle".to_string()],
        );
        assert!(prompt.contains("USING ONLY the evidence below"));
        assert!(prompt.contains("Do not invent facts"));
        assert!(prompt.contains("output exactly INSUFFICIENT_CONTEXT"));
        assert!(prompt.contains("Name: mel schembri"));
        assert!(prompt.contains("- role: designer"));
        assert!(prompt.contains("Active in project(s): Ladle"));
        assert!(prompt.contains("[2026-07-01] Mel reviewed the Ladle onboarding flow"));
    }

    #[test]
    fn excerpt_trim_is_single_line_and_char_capped() {
        let noisy = format!("line one\n\n  line   two {}", "y".repeat(EXCERPT_CHARS * 2));
        let trimmed = trim_excerpt(&noisy);
        assert!(!trimmed.contains('\n'));
        assert!(trimmed.starts_with("line one line two"));
        assert_eq!(trimmed.chars().count(), EXCERPT_CHARS);
    }

    #[test]
    fn opaque_id_filter_matches_watcher_semantics() {
        assert!(looks_like_opaque_id("e:whatever"));
        assert!(looks_like_opaque_id("0123456789abcdef")); // 16 hex chars
        assert!(!looks_like_opaque_id("rust"));
        assert!(!looks_like_opaque_id("web browsing"));
        assert!(!looks_like_opaque_id("cafe")); // hex-y but short = a word
    }
}
