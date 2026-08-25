//! Turning a chat session's open project into a wing — only where the turn
//! itself corroborates it.
//!
//! # Why this is not just "use the open project"
//!
//! A memory's `wing` is its project scope. It is not decoration: it is the
//! recognition-validation ground truth, it gates Spectral's TACT fast path, and
//! it is part of the constellation fingerprint. So a WRONG wing is strictly
//! worse than no wing — an empty wing is visible and countable, a wrong one is
//! invisible and quietly poisons the routes that read it.
//!
//! Spectral measured our live brain (3,160 memories) and found the chat writer
//! is 65% of the whole unwinged problem: 1,002 of 1,049 chat turns carry no
//! wing, because `chat_turn_opts` passed `wing: None` unconditionally. The
//! obvious repair — stamp whatever project the UI has open — was measured too,
//! and it fails:
//!
//! * per turn, 19% of turns would be filed under a project last touched more
//!   than seven days earlier;
//! * per session (pin at session start, 2-hour staleness bound), of the 334
//!   turns it would assign, the turn's own content names the SAME project in
//!   31, a DIFFERENT project in 114, and no project at all in 189.
//!
//! That is 21% verified precision. Where UI context and conversation subject
//! are both observable they disagree 79% of the time. The room you have open is
//! not the subject you are talking about.
//!
//! # What this module does instead
//!
//! The open project is kept, but as a **hypothesis, never a fact**:
//!
//! * the project the UI had open at session start is persisted on the session
//!   row (`sessions.project_hint_id` / `project_hint_wing`) — see
//!   [`crate::session::session_manager::SessionManager::set_project_hint`];
//! * every turn of that session consults the hint, but only sets
//!   `RememberOpts.wing` when the turn's OWN evidence corroborates it;
//! * when it does not, the wing stays `None` — an honest `general` — and the
//!   hint survives on the session row plus the per-turn provenance row, so a
//!   later consolidation pass can revisit it with more evidence.
//!
//! Corroboration is deterministic and cheap: no model, no embedding, no
//! inference call. Three sources, and we record WHICH one fired so the yield of
//! each is measurable rather than assumed:
//!
//! | source | meaning |
//! |---|---|
//! | [`CorroborationSource::ContentName`] | the turn names the project's display name |
//! | [`CorroborationSource::Alias`] | the turn names its slug form (`atlas-atlantic`, `atlasatlantic`) but not the display name |
//! | [`CorroborationSource::ToolPath`] | the turn touched a path under the project's `root_path` |
//!
//! Tool-call arguments are checked at write time deliberately: retrospectively
//! only ~1% of stored turns show a project path, because the stored content is
//! `User: …\nAssistant: …` and the paths lived in the tool calls in between.
//! At write time those arguments are still in hand.
//!
//! # Projects whose names nest
//!
//! `permagent` (the marketing site) and `permagent-runtime` (this codebase) are
//! two of the largest wings, and one name contains the other. Word-boundary
//! anchoring does not separate them on its own — `\b` sits happily against the
//! hyphen, so a bare `permagent` pattern matches inside `permagent-runtime`.
//! The rule is therefore **longest match wins**, evaluated across every
//! project's every spelling, with shorter nested matches discarded. A bare
//! `permagent` corroborates the site only when the runtime is not also named,
//! and a path under a project's root outranks any name match.
//!
//! # A known gap this cannot close
//!
//! Lexical matching cannot recognise a project the user SAID rather than typed:
//! a voice turn transcribed "Loft" for the project LAUFT names the project to a
//! human and nothing at all to a regex. Those turns fall into `unverifiable`
//! and stay honestly `general` — which is the right failure, but it is a recall
//! gap, not an absence of signal. Spectral is assembling a labelled calibration
//! set; the fix belongs there, measured, rather than in a fuzzy-stem heuristic
//! here that would reintroduce exactly the collisions the retired demo fixtures
//! caused.
//!
//! Two sources are deliberately NOT consulted:
//!
//! * **the Librarian's description** — measured: across 987 described memories
//!   it never once named a project the content did not. It adds no signal and
//!   would add a dependency on an enrichment pass that may not have run.
//! * **the episode** — episodes are created per wing (`find_recent_episode(&wing, …)`),
//!   so a `general` memory's episode is a `general` episode. The signal is
//!   circular by construction; measured, 3% and all ambiguous.
//!
//! # Expected yield
//!
//! Roughly 30–37% of chat turns winged, correctly, and the rest left honestly
//! in `general`. That is the intended trade, not a shortfall: the residual
//! includes personal conversation that belongs to no project at all, and the
//! target was never zero.

use crate::wing_rules;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use tracing::warn;

/// How long a conversation may go quiet before its project hint is no longer
/// evidence about the next turn.
///
/// Of the twelve sessions Jesse labelled by hand, two change subject entirely
/// mid-session, and the boundaries fall exactly on long silences: +152 min,
/// +67 min, +1068 min. A session is not one topic — it is one *window*, and a
/// window left open overnight is reopened for something else. So the hint is
/// dropped after a gap this long and stays dropped until a new
/// `project_selected` event arrives to re-establish it.
///
/// Ten minutes is deliberately shorter than the shortest observed boundary
/// (67 min). The cost of dropping a hint too eagerly is a turn left honestly
/// `general`; the cost of keeping one too long is a turn filed under the wrong
/// project, which is invisible. The asymmetry decides the direction.
///
/// This does NOT touch episode boundaries (R45): the session is still the
/// episode. What expires is the scope hypothesis, not the conversation's
/// identity.
pub const HINT_GAP_SECONDS: i64 = 10 * 60;

/// The implicit catch-all project. Never a wing: its display name would swallow
/// any content containing the word "personal". [`wing_rules::project_wing_rules`]
/// skips it for the same reason, and the two must not disagree.
pub const PERSONAL_SLUG: &str = "personal";

/// The project a chat session was opened in, as the UI reported it at session
/// start.
///
/// A hypothesis about scope, never an assertion of it. Held on the session row
/// so every turn of the conversation sees the same one — a per-turn re-read of
/// "what is open right now" would re-introduce exactly the staleness the
/// per-session pin exists to remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectHint {
    /// Canonical project id as the UI sends it: `project:<slug>`.
    pub project_id: String,
    /// The wing slug this project would contribute — `projects.slug`.
    pub slug: String,
    /// Display name, matched separately from the slug so the two spellings can
    /// be told apart in the provenance record.
    pub name: String,
    /// Absolute path to the project's tree, when it has one. The tool-path
    /// corroboration source; `None` for projects that are not a directory.
    pub root_path: Option<String>,
}

impl ProjectHint {
    /// Strip the `project:` prefix a canonical project id carries, yielding the
    /// wing slug. Mirrors `activity::ingestion::derive_wing_slug` exactly, so a
    /// wing set here and a wing set on the ambient path name the same thing.
    /// `None` when the id is not in canonical form or the slug is empty.
    pub fn slug_from_canonical(canonical_project_id: &str) -> Option<&str> {
        let slug = canonical_project_id.strip_prefix("project:")?;
        (!slug.is_empty()).then_some(slug)
    }
}

/// Which signal corroborated the hint. Recorded per turn so the yield of each
/// source is measurable — an assumed yield is how a signal nobody checks stays
/// in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CorroborationSource {
    /// The turn names the project's display name.
    ContentName,
    /// The turn names the project's slug form, but not its display name.
    Alias,
    /// The turn touched a path under the project's root.
    ToolPath,
}

impl CorroborationSource {
    /// The stable string written to the provenance row and reported by the
    /// backfill. Kebab-case, matching the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContentName => "content-name",
            Self::Alias => "alias",
            Self::ToolPath => "tool-path",
        }
    }
}

/// What one turn's evidence says about its session's project hint.
///
/// The three variants are also the three buckets the backfill reports, on
/// purpose: the write path and the repair path must classify a turn the same
/// way or the corpus measurement means nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "kebab-case")]
pub enum WingVerdict {
    /// The turn's own evidence names the hinted project. This is the ONLY
    /// verdict that sets a wing.
    Corroborated {
        wing: String,
        source: CorroborationSource,
    },
    /// The turn names a DIFFERENT known project than the one the UI had open.
    /// No wing is written: we have two candidates and no way to adjudicate
    /// between "they mentioned another project in passing" and "the hint is
    /// simply stale". Counted separately because a large conflicting bucket is
    /// the falsification signal for the whole hint mechanism.
    Conflicting {
        /// The wing the turn's content points at.
        named_wing: String,
    },
    /// Nothing in the turn names any known project. No wing; the hint is kept.
    Unverifiable,
}

impl WingVerdict {
    /// The wing to write, which is `Some` for exactly one variant.
    pub fn wing(&self) -> Option<&str> {
        match self {
            Self::Corroborated { wing, .. } => Some(wing.as_str()),
            _ => None,
        }
    }

    /// Stable bucket name for reports and the provenance row.
    pub fn bucket(&self) -> &'static str {
        match self {
            Self::Corroborated { .. } => "corroborated",
            Self::Conflicting { .. } => "conflicting",
            Self::Unverifiable => "unverifiable",
        }
    }

    /// Which source corroborated, when one did.
    pub fn source(&self) -> Option<CorroborationSource> {
        match self {
            Self::Corroborated { source, .. } => Some(*source),
            _ => None,
        }
    }
}

/// One session's hint, compiled against the project registry, ready to judge
/// many turns.
///
/// Compiled once per session (or once per backfill sweep) rather than per turn:
/// the registry is ~22 projects and a turn is a hot path.
pub struct WingCorroborator {
    hint: ProjectHint,
    /// Lowercased project root, when the hint's project has one.
    root: Option<String>,
    /// Every spelling of every known project — the hinted one included. One
    /// candidate per (project, spelling), because the winner is decided by
    /// match LENGTH and the losing spellings have to be in the race to lose it.
    candidates: Vec<Candidate>,
}

/// One project, one spelling, compiled.
struct Candidate {
    wing: String,
    source: CorroborationSource,
    re: regex::Regex,
}

impl WingCorroborator {
    /// Build from a hint and the `(slug, name)` project registry.
    ///
    /// The registry should contain every project, including the hinted one —
    /// a conflict is decided by comparing the project the text names against
    /// the hint, and a registry missing the hint would make its own project
    /// look like a conflict. If the hint's project is absent (its row was
    /// deleted), its spellings are added so it can still be recognised.
    pub fn new(hint: ProjectHint, projects: &[(String, String)]) -> Self {
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut push = |slug: &str, name: &str| {
            if slug == PERSONAL_SLUG {
                return;
            }
            // Display name first, so an exact tie between the two spellings of
            // the SAME project records the more legible label.
            for (raw, source) in [
                (name, CorroborationSource::ContentName),
                (slug, CorroborationSource::Alias),
            ] {
                let Some(pattern) = wing_rules::bounded_token_pattern(raw) else {
                    continue;
                };
                if candidates
                    .iter()
                    .any(|c| c.wing == slug && c.re.as_str() == pattern)
                {
                    continue;
                }
                if let Ok(re) = regex::Regex::new(&pattern) {
                    candidates.push(Candidate {
                        wing: slug.to_string(),
                        source,
                        re,
                    });
                }
            }
        };

        for (slug, name) in projects {
            push(slug, name);
        }
        if !projects.iter().any(|(slug, _)| *slug == hint.slug) {
            push(&hint.slug, &hint.name);
        }

        let root = hint
            .root_path
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_lowercase);

        Self {
            root,
            candidates,
            hint,
        }
    }

    /// The hint this corroborator judges against.
    pub fn hint(&self) -> &ProjectHint {
        &self.hint
    }

    /// Judge one turn.
    ///
    /// `content` is what will be stored (`User: …\nAssistant: …`); `tool_text`
    /// is the turn's tool-call arguments, visible at write time and mostly gone
    /// by the time the memory is read back. Both are searched — a project path
    /// or a project name can appear in either.
    ///
    /// # Order, and why it is this order
    ///
    /// 1. **A path under the hint's project root wins outright.** A file under
    ///    a project's tree is structural evidence about what the turn was
    ///    working on; a project name in prose is a mention, and a turn can
    ///    mention any number of projects while editing exactly one.
    /// 2. **Otherwise the LONGEST name match wins**, across every project's
    ///    every spelling, and shorter nested matches are discarded. `\b`
    ///    anchoring alone does not separate nesting projects, because a word
    ///    boundary sits happily against the hyphen in `permagent-runtime` —
    ///    so a bare `permagent` pattern matches inside it. Length does
    ///    separate them: `permagent-runtime` (17) beats `permagent` (9), so a
    ///    bare `permagent` corroborates the marketing site only when the
    ///    runtime is not also named. These are two of the largest real wings;
    ///    conflating them would be scope leakage between the two projects most
    ///    likely to be confused.
    /// 3. If the winner is the hinted project, that is corroboration; if it is
    ///    a different one, that is a conflict and no wing is written; if
    ///    nothing matched at all, it is unverifiable.
    pub fn verdict(&self, content: &str, tool_text: &str) -> WingVerdict {
        // The catch-all project is never a wing. Bail before any matching so
        // this can never be reported as corroborated.
        if self.hint.slug == PERSONAL_SLUG {
            return WingVerdict::Unverifiable;
        }

        let mut haystack = String::with_capacity(content.len() + tool_text.len() + 1);
        haystack.push_str(content);
        haystack.push('\n');
        haystack.push_str(tool_text);
        let haystack = haystack.to_lowercase();

        if self
            .root
            .as_deref()
            .is_some_and(|root| haystack.contains(root))
        {
            return WingVerdict::Corroborated {
                wing: self.hint.slug.clone(),
                source: CorroborationSource::ToolPath,
            };
        }

        // Longest match wins. Ties keep the earlier candidate, which is the
        // display name of the same project before its slug, and otherwise
        // registry order — deterministic either way.
        let mut best: Option<(usize, &Candidate)> = None;
        for candidate in &self.candidates {
            let Some(len) = candidate.re.find_iter(&haystack).map(|m| m.len()).max() else {
                continue;
            };
            if best.is_none_or(|(best_len, _)| len > best_len) {
                best = Some((len, candidate));
            }
        }

        match best {
            Some((_, candidate)) if candidate.wing == self.hint.slug => WingVerdict::Corroborated {
                wing: candidate.wing.clone(),
                source: candidate.source,
            },
            Some((_, candidate)) => WingVerdict::Conflicting {
                named_wing: candidate.wing.clone(),
            },
            None => WingVerdict::Unverifiable,
        }
    }
}

// ── Persistence ──────────────────────────────────────────────────────────

/// Record the project a chat session was opened in — **once, and only once**.
///
/// Two callers reach this, and both mean "the client told us, at this moment,
/// that this session belongs to this project":
///
/// * `POST /api/sessions` when the client sent a `projectId`;
/// * `POST /activity/emit` for a `project_selected` event that carries a
///   session id. This is the seam that actually fires today: the desktop shells
///   create their chat sessions over ACP rather than through the sessions
///   route, so the activity event is where the association arrives. It is still
///   capture, not inference — the UI is telling us which chat it just opened in
///   which project, at the instant it did so.
///
/// **Once-only is the whole safety property.** The `WHERE` clause refuses to
/// overwrite an existing hint, so a later `project_selected` for the same
/// session — the user switching project with a conversation already running —
/// cannot silently re-scope turns already written under the first hypothesis.
/// A different project gets a different session; an existing session keeps the
/// project it was opened in.
///
/// Returns whether a hint was actually recorded: `false` means the session
/// already had one (or does not exist), which is a normal outcome, not an
/// error.
///
/// `canonical_project_id` is the `project:<slug>` form. A non-canonical id is
/// refused rather than coerced — a wing derived from a guess is the failure
/// this whole module exists to avoid.
pub async fn set_session_project_hint(
    pool: &Pool<Sqlite>,
    session_id: &str,
    canonical_project_id: &str,
) -> Result<bool, String> {
    let slug = ProjectHint::slug_from_canonical(canonical_project_id).ok_or_else(|| {
        format!("project id {canonical_project_id:?} is not in canonical `project:<slug>` form")
    })?;

    let result = sqlx::query(
        "UPDATE sessions
         SET project_hint_id = ?, project_hint_wing = ?
         WHERE id = ? AND (project_hint_id IS NULL OR project_hint_id = '')",
    )
    .bind(canonical_project_id)
    .bind(slug)
    .bind(session_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

/// Read a session's project hint, enriched with the project's display name and
/// root path.
///
/// `Ok(None)` means the session honestly has no project — a global chat. The
/// join is a LEFT JOIN on purpose: a hint whose project row has since been
/// deleted still names a slug, and a turn that spells that slug is still
/// corroborated. In that case the display name falls back to the slug rather
/// than the hint being discarded.
pub async fn load_session_project_hint(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Option<ProjectHint> {
    let row: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT s.project_hint_id, s.project_hint_wing, p.name, p.root_path
             FROM sessions s
             LEFT JOIN projects p ON p.slug = s.project_hint_wing
             WHERE s.id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .unwrap_or_else(|e| {
        warn!(
            target: "permagent::session_wing",
            session = %session_id,
            error = %e,
            "could not read the session project hint — the turn stays unwinged"
        );
        None
    });

    // Destructure AFTER the row option is unwrapped. `fetch_optional` already
    // returns `Option<Row>`; applying `?` before the annotation would make sqlx
    // infer the ROW type as `Option<...>` and ask for a `FromRow` impl on it.
    let (project_id, slug, name, root_path) = row?;
    let project_id = project_id?;
    let slug = slug.filter(|s| !s.is_empty())?;
    let name = name.unwrap_or_else(|| slug.clone());
    Some(ProjectHint {
        project_id,
        slug,
        name,
        root_path,
    })
}

/// Persist what was decided for one chat turn and on what evidence.
///
/// Best-effort by design: this is the measurement, not the memory. A failure
/// here is logged and swallowed so a provenance problem can never cost a user
/// their conversation — but it IS logged, because a silently missing row would
/// make the corroborated-yield number quietly wrong rather than visibly absent.
///
/// `INSERT OR REPLACE` keyed on `memory_key`: a turn re-written under the same
/// Brain key replaces its provenance instead of accumulating rows.
pub async fn record_turn_provenance(
    pool: &Pool<Sqlite>,
    memory_key: &str,
    session_id: &str,
    hint: Option<&ProjectHint>,
    verdict: &WingVerdict,
) {
    let named_wing = match verdict {
        WingVerdict::Conflicting { named_wing } => Some(named_wing.as_str()),
        _ => None,
    };

    if let Err(e) = sqlx::query(
        "INSERT OR REPLACE INTO chat_turn_wing_provenance
         (memory_key, session_id, project_hint_id, project_hint_wing,
          verdict, corroborated_by, named_wing, wing_written)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(memory_key)
    .bind(session_id)
    .bind(hint.map(|h| h.project_id.as_str()))
    .bind(hint.map(|h| h.slug.as_str()))
    .bind(verdict.bucket())
    .bind(verdict.source().map(|s| s.as_str()))
    .bind(named_wing)
    .bind(verdict.wing())
    .execute(pool)
    .await
    {
        warn!(
            target: "permagent::session_wing",
            key = %memory_key,
            error = %e,
            "could not record chat-turn wing provenance (the memory itself is unaffected)"
        );
    }
}

/// Drop a session's project hint. Called when a silence has made it stale;
/// a later `project_selected` may set a fresh one, because
/// [`set_session_project_hint`] only refuses to overwrite a hint that is
/// still there.
async fn clear_session_project_hint(pool: &Pool<Sqlite>, session_id: &str) {
    if let Err(e) = sqlx::query(
        "UPDATE sessions SET project_hint_id = NULL, project_hint_wing = NULL WHERE id = ?",
    )
    .bind(session_id)
    .execute(pool)
    .await
    {
        warn!(
            target: "permagent::session_wing",
            session = %session_id,
            error = %e,
            "could not clear a stale session project hint"
        );
    }
}

/// Record that this session just wrote a turn, so the NEXT turn can measure the
/// gap. Best-effort: a missed stamp makes the next gap look shorter, which
/// errs toward dropping the hint — the safe direction.
async fn touch_session_turn_clock(pool: &Pool<Sqlite>, session_id: &str, now: DateTime<Utc>) {
    if let Err(e) = sqlx::query("UPDATE sessions SET project_hint_last_turn_at = ? WHERE id = ?")
        .bind(now.to_rfc3339())
        .bind(session_id)
        .execute(pool)
        .await
    {
        warn!(
            target: "permagent::session_wing",
            session = %session_id,
            error = %e,
            "could not stamp the session turn clock"
        );
    }
}

/// When this session last wrote a turn, if it ever has.
async fn last_turn_at(pool: &Pool<Sqlite>, session_id: &str) -> Option<DateTime<Utc>> {
    let raw: Option<Option<String>> =
        sqlx::query_scalar("SELECT project_hint_last_turn_at FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .ok()?;
    let raw = raw??;
    DateTime::parse_from_rfc3339(&raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Decide one turn's wing from its session, end to end.
///
/// Returns the verdict plus the hint it was judged against, so the caller can
/// write both the memory and its provenance without a second lookup. When the
/// session has no project hint the verdict is [`WingVerdict::Unverifiable`] and
/// the hint is `None` — a global chat is honestly `general`, and that is a
/// result, not a failure.
///
/// A hint older than [`HINT_GAP_SECONDS`] of silence is dropped before it is
/// consulted, and this turn is judged as if the session never had one.
pub async fn decide_turn_wing(
    pool: &Pool<Sqlite>,
    session_id: &str,
    content: &str,
    tool_text: &str,
) -> (Option<ProjectHint>, WingVerdict) {
    decide_turn_wing_at(pool, session_id, content, tool_text, Utc::now()).await
}

/// [`decide_turn_wing`] with an explicit clock, so the gap rule is testable
/// without sleeping.
pub async fn decide_turn_wing_at(
    pool: &Pool<Sqlite>,
    session_id: &str,
    content: &str,
    tool_text: &str,
    now: DateTime<Utc>,
) -> (Option<ProjectHint>, WingVerdict) {
    // The gap is measured first, and the clock is stamped whatever happens —
    // a turn that inherited no hint still marks the timeline for the next one.
    let gap = last_turn_at(pool, session_id)
        .await
        .map(|last| (now - last).num_seconds());
    touch_session_turn_clock(pool, session_id, now).await;

    if gap.is_some_and(|seconds| seconds >= HINT_GAP_SECONDS) {
        clear_session_project_hint(pool, session_id).await;
        return (None, WingVerdict::Unverifiable);
    }

    let Some(hint) = load_session_project_hint(pool, session_id).await else {
        return (None, WingVerdict::Unverifiable);
    };
    let projects = wing_rules::load_project_rows(pool).await;
    let corroborator = WingCorroborator::new(hint.clone(), &projects);
    let verdict = corroborator.verdict(content, tool_text);
    (Some(hint), verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Vec<(String, String)> {
        vec![
            ("permagent".to_string(), "Permagent".to_string()),
            ("plekk".to_string(), "Plekk".to_string()),
            ("atlas-atlantic".to_string(), "Atlas Atlantic".to_string()),
            ("personal".to_string(), "Personal".to_string()),
        ]
    }

    fn hint(slug: &str, name: &str, root: Option<&str>) -> ProjectHint {
        ProjectHint {
            project_id: format!("project:{slug}"),
            slug: slug.to_string(),
            name: name.to_string(),
            root_path: root.map(str::to_string),
        }
    }

    fn corroborator(slug: &str, name: &str, root: Option<&str>) -> WingCorroborator {
        WingCorroborator::new(hint(slug, name, root), &registry())
    }

    // ── the table: what each kind of turn yields ──

    #[test]
    fn same_project_named_in_content_sets_the_wing() {
        let c = corroborator("permagent", "Permagent", None);
        let v = c.verdict("User: how is Permagent doing?\nAssistant: fine", "");
        assert_eq!(
            v,
            WingVerdict::Corroborated {
                wing: "permagent".to_string(),
                source: CorroborationSource::ContentName,
            }
        );
        assert_eq!(v.wing(), Some("permagent"));
        assert_eq!(v.bucket(), "corroborated");
    }

    #[test]
    fn the_hyphenated_slug_spelling_matches_the_display_name_pattern() {
        // Display name "Atlas Atlantic" does not appear; the collapsed slug
        // form does. Same wing, different — and honestly recorded — evidence.
        let c = corroborator("atlas-atlantic", "Atlas Atlantic", None);
        let v = c.verdict("User: deploy atlas-atlantic\nAssistant: done", "");
        assert_eq!(
            v,
            WingVerdict::Corroborated {
                wing: "atlas-atlantic".to_string(),
                source: CorroborationSource::ContentName,
            },
            "the display-name pattern also matches the hyphenated slug, and \
             that is the more legible label"
        );
    }

    #[test]
    fn a_slug_that_shares_no_tokens_with_the_display_name_is_recorded_as_an_alias() {
        // Real registry shape: slug `grocery-savings-planner`, display name
        // `Grocery Savers`. A turn spelling only the slug is corroborated, and
        // the provenance says so rather than claiming the name was named.
        let projects = vec![(
            "grocery-savings-planner".to_string(),
            "Grocery Savers".to_string(),
        )];
        let c = WingCorroborator::new(
            hint("grocery-savings-planner", "Grocery Savers", None),
            &projects,
        );
        assert_eq!(
            c.verdict("User: open grocery-savings-planner\nAssistant: ok", ""),
            WingVerdict::Corroborated {
                wing: "grocery-savings-planner".to_string(),
                source: CorroborationSource::Alias,
            }
        );
    }

    #[test]
    fn a_different_project_named_leaves_the_wing_empty_and_is_counted_as_a_conflict() {
        let c = corroborator("permagent", "Permagent", None);
        let v = c.verdict("User: what about Plekk onboarding?\nAssistant: …", "");
        assert_eq!(
            v,
            WingVerdict::Conflicting {
                named_wing: "plekk".to_string(),
            }
        );
        assert_eq!(v.wing(), None, "a conflict must never write a wing");
        assert_eq!(v.bucket(), "conflicting");
    }

    #[test]
    fn no_project_named_leaves_the_wing_empty_and_the_hint_intact() {
        let c = corroborator("permagent", "Permagent", None);
        let v = c.verdict("User: what should I cook tonight?\nAssistant: pasta", "");
        assert_eq!(v, WingVerdict::Unverifiable);
        assert_eq!(v.wing(), None);
        assert_eq!(v.bucket(), "unverifiable");
        // The hint is not consumed or cleared by a failed corroboration.
        assert_eq!(c.hint().slug, "permagent");
    }

    #[test]
    fn a_path_under_the_project_root_corroborates_and_is_labelled_as_such() {
        let c = corroborator("plekk", "Plekk", Some("/Users/j/Documents/dev/plekk"));
        // Content that never says "plekk" in prose, but a tool call touched
        // the tree. Note this only works at WRITE time — the stored content
        // alone would be unverifiable.
        let v = c.verdict(
            "User: fix the failing test\nAssistant: fixed it",
            r#"{"path":"/Users/j/Documents/dev/plekk/src/lib.rs"}"#,
        );
        assert_eq!(
            v,
            WingVerdict::Corroborated {
                wing: "plekk".to_string(),
                source: CorroborationSource::ToolPath,
            },
            "structural evidence outranks a name mention, and is recorded as \
             the structural signal it is"
        );
    }

    #[test]
    fn a_path_under_the_hint_root_beats_another_project_named_in_prose() {
        // The turn talks about Permagent while editing a file in Plekk. The
        // file is what the turn was DOING; the name is what it mentioned.
        let c = corroborator("plekk", "Plekk", Some("/Users/j/Documents/dev/plekk"));
        let v = c.verdict(
            "User: like the permagent one\nAssistant: done",
            r#"{"path":"/Users/j/Documents/dev/plekk/src/lib.rs"}"#,
        );
        assert_eq!(
            v,
            WingVerdict::Corroborated {
                wing: "plekk".to_string(),
                source: CorroborationSource::ToolPath,
            }
        );
    }

    // ── nesting names: the two largest wings, one inside the other ──

    fn nesting_registry() -> Vec<(String, String)> {
        vec![
            ("permagent".to_string(), "Permagent".to_string()),
            (
                "permagent-runtime".to_string(),
                "Permagent Runtime".to_string(),
            ),
        ]
    }

    /// Which project does a turn name, judged from each side of the nesting?
    /// Returns the wing the corroborator settled on, whichever verdict carried
    /// it — the point is WHICH project won, not whether it matched the hint.
    fn named_project(text: &str) -> Option<String> {
        let c = WingCorroborator::new(hint("permagent", "Permagent", None), &nesting_registry());
        match c.verdict(text, "") {
            WingVerdict::Corroborated { wing, .. } => Some(wing),
            WingVerdict::Conflicting { named_wing } => Some(named_wing),
            WingVerdict::Unverifiable => None,
        }
    }

    #[test]
    fn the_runtime_name_does_not_corroborate_the_site() {
        assert_eq!(
            named_project("User: fixed permagent-runtime\nAssistant: ok"),
            Some("permagent-runtime".to_string()),
            "`permagent` matches inside `permagent-runtime`; the longer match \
             must win or the two largest wings leak into each other"
        );
    }

    #[test]
    fn a_bare_site_name_corroborates_the_site() {
        assert_eq!(
            named_project("User: permagent is down\nAssistant: looking"),
            Some("permagent".to_string())
        );
        assert_eq!(
            named_project("User: the permagent app\nAssistant: ok"),
            Some("permagent".to_string())
        );
    }

    #[test]
    fn when_both_are_named_the_longer_one_wins() {
        assert_eq!(
            named_project("User: permagent and permagent-runtime\nAssistant: ok"),
            Some("permagent-runtime".to_string())
        );
        // Word order must not decide it.
        assert_eq!(
            named_project("User: permagent runtime, not permagent\nAssistant: ok"),
            Some("permagent-runtime".to_string())
        );
    }

    #[test]
    fn the_runtime_hint_is_corroborated_by_its_own_name_and_conflicted_by_the_sites() {
        let c = WingCorroborator::new(
            hint("permagent-runtime", "Permagent Runtime", None),
            &nesting_registry(),
        );
        assert_eq!(
            c.verdict("User: permagent-runtime tests\nAssistant: ok", ""),
            WingVerdict::Corroborated {
                wing: "permagent-runtime".to_string(),
                source: CorroborationSource::ContentName,
            }
        );
        assert_eq!(
            c.verdict("User: the permagent app\nAssistant: ok", ""),
            WingVerdict::Conflicting {
                named_wing: "permagent".to_string()
            }
        );
    }

    #[test]
    fn a_whole_token_is_required_so_a_name_inside_a_word_does_not_match() {
        assert_eq!(
            named_project("User: superpermagentish nonsense\nAssistant: ok"),
            None
        );
    }

    #[test]
    fn a_root_path_that_shares_no_name_with_the_project_still_corroborates() {
        // The honest tool-path case: the root does not contain the slug or the
        // display name anywhere, so only the path check can fire.
        let c = corroborator("atlas-atlantic", "Atlas Atlantic", Some("/srv/aa-site"));
        let v = c.verdict(
            "User: redeploy\nAssistant: done",
            r#"{"path":"/srv/aa-site/index.html"}"#,
        );
        assert_eq!(
            v,
            WingVerdict::Corroborated {
                wing: "atlas-atlantic".to_string(),
                source: CorroborationSource::ToolPath,
            }
        );
    }

    #[test]
    fn tool_arguments_are_searched_as_well_as_stored_content() {
        let c = corroborator("permagent", "Permagent", None);
        let v = c.verdict(
            "User: what changed?\nAssistant: a few things",
            r#"{"query":"permagent release notes"}"#,
        );
        assert_eq!(v.wing(), Some("permagent"));
    }

    #[test]
    fn the_personal_project_is_never_a_wing() {
        let c = corroborator("personal", "Personal", None);
        let v = c.verdict("User: a personal note about Personal\nAssistant: ok", "");
        assert_eq!(v, WingVerdict::Unverifiable);
        assert_eq!(v.wing(), None);
    }

    #[test]
    fn an_empty_root_path_is_not_a_wildcard() {
        // `contains("")` is always true — a blank root must not corroborate
        // every turn ever written.
        let c = corroborator("plekk", "Plekk", Some("   "));
        assert_eq!(
            c.verdict("User: unrelated\nAssistant: unrelated", ""),
            WingVerdict::Unverifiable
        );
    }

    #[test]
    fn matching_is_case_insensitive_in_both_directions() {
        let c = corroborator("permagent", "Permagent", None);
        assert_eq!(
            c.verdict("User: PERMAGENT is down\nAssistant: …", "")
                .wing(),
            Some("permagent")
        );
    }

    // ── slug derivation agrees with the ambient path ──

    #[test]
    fn slug_from_canonical_matches_the_ambient_derivation() {
        assert_eq!(
            ProjectHint::slug_from_canonical("project:permagent"),
            Some("permagent")
        );
        assert_eq!(ProjectHint::slug_from_canonical("permagent"), None);
        assert_eq!(ProjectHint::slug_from_canonical("project:"), None);
        assert_eq!(
            ProjectHint::slug_from_canonical("did:chitin:henry-malcolm"),
            None
        );
    }

    #[test]
    fn source_labels_are_stable() {
        assert_eq!(CorroborationSource::ContentName.as_str(), "content-name");
        assert_eq!(CorroborationSource::Alias.as_str(), "alias");
        assert_eq!(CorroborationSource::ToolPath.as_str(), "tool-path");
    }

    // ── the session hint, end to end against a real schema ──

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    async fn a_project(pool: &Pool<Sqlite>, slug: &str, name: &str, root: Option<&str>) {
        sqlx::query("INSERT INTO projects (id, slug, name, root_path) VALUES (?, ?, ?, ?)")
            .bind(format!("id-{slug}"))
            .bind(slug)
            .bind(name)
            .bind(root)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn a_session(pool: &Pool<Sqlite>, id: &str) {
        sqlx::query("INSERT INTO sessions (id, name, working_dir) VALUES (?, ?, ?)")
            .bind(id)
            .bind("New Chat")
            .bind("/tmp")
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_session_created_in_a_project_pins_that_project_as_its_hint() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", Some("/dev/permagent")).await;
        a_session(&pool, "s1").await;

        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        let hint = load_session_project_hint(&pool, "s1").await.unwrap();
        assert_eq!(hint.project_id, "project:permagent");
        assert_eq!(hint.slug, "permagent");
        assert_eq!(hint.name, "Permagent");
        assert_eq!(hint.root_path.as_deref(), Some("/dev/permagent"));
    }

    #[tokio::test]
    async fn a_session_with_no_project_has_no_hint_and_that_is_the_right_answer() {
        let pool = test_pool().await;
        a_session(&pool, "s-global").await;
        assert!(load_session_project_hint(&pool, "s-global").await.is_none());

        let (hint, verdict) =
            decide_turn_wing(&pool, "s-global", "User: hello\nAssistant: hi", "").await;
        assert!(hint.is_none());
        assert_eq!(verdict, WingVerdict::Unverifiable);
        assert_eq!(verdict.wing(), None, "a global chat is honestly `general`");
    }

    #[tokio::test]
    async fn a_non_canonical_project_id_is_refused_rather_than_coerced() {
        let pool = test_pool().await;
        a_session(&pool, "s1").await;
        assert!(set_session_project_hint(&pool, "s1", "permagent")
            .await
            .is_err());
        assert!(load_session_project_hint(&pool, "s1").await.is_none());
    }

    #[tokio::test]
    async fn every_turn_of_a_session_sees_the_same_hint() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        for turn in [
            "User: permagent build\nAssistant: ok",
            "User: and permagent tests\nAssistant: ok",
        ] {
            let (hint, verdict) = decide_turn_wing(&pool, "s1", turn, "").await;
            assert_eq!(hint.unwrap().slug, "permagent");
            assert_eq!(verdict.wing(), Some("permagent"));
        }
    }

    #[tokio::test]
    async fn an_uncorroborated_turn_stays_unwinged_and_keeps_its_hint() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        let (hint, verdict) = decide_turn_wing(
            &pool,
            "s1",
            "User: what's for dinner?\nAssistant: pasta",
            "",
        )
        .await;
        assert_eq!(verdict, WingVerdict::Unverifiable);
        assert_eq!(verdict.wing(), None);
        assert_eq!(
            hint.unwrap().slug,
            "permagent",
            "the hint survives a failed corroboration — it is evidence for later, \
             not a decision that was consumed"
        );
    }

    #[tokio::test]
    async fn a_turn_naming_another_project_is_a_conflict_and_writes_no_wing() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_project(&pool, "plekk", "Plekk", None).await;
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        let (_, verdict) =
            decide_turn_wing(&pool, "s1", "User: plekk onboarding\nAssistant: sure", "").await;
        assert_eq!(
            verdict,
            WingVerdict::Conflicting {
                named_wing: "plekk".to_string()
            }
        );
        assert_eq!(verdict.wing(), None);
    }

    #[tokio::test]
    async fn switching_project_does_not_rewing_the_session_already_open() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_project(&pool, "plekk", "Plekk", None).await;
        a_session(&pool, "s-first").await;
        a_session(&pool, "s-second").await;

        // The UI opens a chat in Permagent, then the user switches project and
        // the UI starts a NEW session there. The first session must be
        // untouched: re-stamping it would silently re-scope turns already
        // written under the old hypothesis.
        set_session_project_hint(&pool, "s-first", "project:permagent")
            .await
            .unwrap();
        set_session_project_hint(&pool, "s-second", "project:plekk")
            .await
            .unwrap();

        assert_eq!(
            load_session_project_hint(&pool, "s-first")
                .await
                .unwrap()
                .slug,
            "permagent"
        );
        assert_eq!(
            load_session_project_hint(&pool, "s-second")
                .await
                .unwrap()
                .slug,
            "plekk"
        );

        // And if a later `project_selected` DOES name the already-open session,
        // it must be refused rather than re-scoping turns already written.
        let recorded = set_session_project_hint(&pool, "s-first", "project:plekk")
            .await
            .unwrap();
        assert!(!recorded, "an existing hint must never be overwritten");
        assert_eq!(
            load_session_project_hint(&pool, "s-first")
                .await
                .unwrap()
                .slug,
            "permagent"
        );
    }

    #[tokio::test]
    async fn provenance_records_the_bucket_the_source_and_the_wing_written() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        let (hint, verdict) =
            decide_turn_wing(&pool, "s1", "User: permagent build\nAssistant: ok", "").await;
        record_turn_provenance(&pool, "chat-s1-1", "s1", hint.as_ref(), &verdict).await;

        let row: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT verdict, corroborated_by, wing_written, project_hint_wing
             FROM chat_turn_wing_provenance WHERE memory_key = ?",
        )
        .bind("chat-s1-1")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "corroborated");
        assert_eq!(row.1.as_deref(), Some("content-name"));
        assert_eq!(row.2.as_deref(), Some("permagent"));
        assert_eq!(row.3.as_deref(), Some("permagent"));
    }

    #[tokio::test]
    async fn an_unwinged_turn_still_records_its_provenance() {
        // The negative rows are half the measurement: without them the
        // corroborated yield is a numerator with no denominator.
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        let (hint, verdict) =
            decide_turn_wing(&pool, "s1", "User: dinner ideas\nAssistant: pasta", "").await;
        record_turn_provenance(&pool, "chat-s1-2", "s1", hint.as_ref(), &verdict).await;

        let row: (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT verdict, wing_written, project_hint_wing
             FROM chat_turn_wing_provenance WHERE memory_key = ?",
        )
        .bind("chat-s1-2")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "unverifiable");
        assert_eq!(row.1, None);
        assert_eq!(
            row.2.as_deref(),
            Some("permagent"),
            "the hint is preserved even though no wing was written"
        );
    }

    #[tokio::test]
    async fn the_hint_migration_is_idempotent() {
        let pool = test_pool().await;
        // init already applied it; running it again must be a no-op rather than
        // a duplicate-column error — this runs on every boot.
        for _ in 0..3 {
            crate::session::spectral_schema::apply_session_project_hint_schema(&pool)
                .await
                .unwrap();
        }
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();
        assert_eq!(
            load_session_project_hint(&pool, "s1").await.unwrap().slug,
            "permagent"
        );
    }

    // ── the hint expires on a long silence ──

    #[tokio::test]
    async fn a_hint_does_not_survive_a_ten_minute_silence() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        let t0 = chrono::DateTime::parse_from_rfc3339("2026-08-20T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // First turn establishes the clock and inherits the hint.
        let (hint, verdict) =
            decide_turn_wing_at(&pool, "s1", "User: permagent build\nAssistant: ok", "", t0).await;
        assert_eq!(hint.unwrap().slug, "permagent");
        assert_eq!(verdict.wing(), Some("permagent"));

        // Back within the window: still the same conversation.
        let (hint, verdict) = decide_turn_wing_at(
            &pool,
            "s1",
            "User: permagent again\nAssistant: ok",
            "",
            t0 + chrono::Duration::minutes(9),
        )
        .await;
        assert_eq!(hint.unwrap().slug, "permagent");
        assert_eq!(verdict.wing(), Some("permagent"));

        // Ten minutes later the window was reopened for something else. The
        // hint is dropped, and dropping it is a WRITE — a later turn must not
        // silently re-inherit it either.
        let (hint, verdict) = decide_turn_wing_at(
            &pool,
            "s1",
            "User: permagent still\nAssistant: ok",
            "",
            t0 + chrono::Duration::minutes(19),
        )
        .await;
        assert!(
            hint.is_none(),
            "a hint must not carry across a long silence"
        );
        assert_eq!(verdict, WingVerdict::Unverifiable);
        assert!(load_session_project_hint(&pool, "s1").await.is_none());

        // Even an immediately following turn stays unhinted: the hypothesis is
        // gone until a new project_selected re-establishes it.
        let (hint, _) = decide_turn_wing_at(
            &pool,
            "s1",
            "User: permagent once more\nAssistant: ok",
            "",
            t0 + chrono::Duration::minutes(20),
        )
        .await;
        assert!(hint.is_none());
    }

    #[tokio::test]
    async fn a_new_project_selection_re_establishes_a_dropped_hint() {
        let pool = test_pool().await;
        a_project(&pool, "permagent", "Permagent", None).await;
        a_project(&pool, "plekk", "Plekk", None).await;
        a_session(&pool, "s1").await;
        set_session_project_hint(&pool, "s1", "project:permagent")
            .await
            .unwrap();

        let t0 = chrono::DateTime::parse_from_rfc3339("2026-08-20T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        decide_turn_wing_at(&pool, "s1", "User: hi\nAssistant: hi", "", t0).await;
        decide_turn_wing_at(
            &pool,
            "s1",
            "User: hi\nAssistant: hi",
            "",
            t0 + chrono::Duration::hours(3),
        )
        .await;
        assert!(load_session_project_hint(&pool, "s1").await.is_none());

        // The once-only guard protects a LIVE hint, not a retired one: after
        // the gap the session is free to be told what it is about again.
        let recorded = set_session_project_hint(&pool, "s1", "project:plekk")
            .await
            .unwrap();
        assert!(recorded);
        let (hint, verdict) = decide_turn_wing_at(
            &pool,
            "s1",
            "User: plekk onboarding\nAssistant: ok",
            "",
            t0 + chrono::Duration::hours(3) + chrono::Duration::minutes(1),
        )
        .await;
        assert_eq!(hint.unwrap().slug, "plekk");
        assert_eq!(verdict.wing(), Some("plekk"));
    }

    #[tokio::test]
    async fn the_gap_rule_does_not_touch_the_episode_identity() {
        // R45 says the session IS the episode. What expires here is the scope
        // hypothesis, not the conversation's identity — nothing in this module
        // writes or clears an episode, and this test exists so a future change
        // that starts to has to delete an explicit statement that it must not.
        let pool = test_pool().await;
        a_session(&pool, "s1").await;
        let t0 = chrono::DateTime::parse_from_rfc3339("2026-08-20T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        decide_turn_wing_at(&pool, "s1", "User: a\nAssistant: b", "", t0).await;
        decide_turn_wing_at(
            &pool,
            "s1",
            "User: a\nAssistant: b",
            "",
            t0 + chrono::Duration::hours(5),
        )
        .await;
        // The session row is still one session; only the hint columns moved.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = 's1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
