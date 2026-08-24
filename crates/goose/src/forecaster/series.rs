//! The closed set of market sources, and the shape of a bound series.
//!
//! `SourceKind` is a closed enum for the same reason `growth::metrics::TargetMetric`
//! is (`growth/metrics.rs`): an arbitrary string here would let a model invent a
//! series, and — worse — would let a model invent a URL. Every collector builds
//! its request from a validated variant plus a normalized, percent-encoded
//! subject, so "an unknown source" is refused at parse time and never reaches
//! the network.
//!
//! `subject` stays free text because every project has different competitors,
//! but it is normalized *per source*: "LangChain" is `langchain` on npm, an
//! already-normalized `langchain` on PyPI (PEP 503), `LangChain` as a Wikipedia
//! article and a free-text query on HN. One competitor, four strings — so a
//! small alias table maps a canonical group name onto each source's spelling,
//! and `subject_group` reads it back so the UI can show the four as one.

use serde::{Deserialize, Serialize};

/// A source of market-direction numbers.
///
/// Membership is decided by one property: **does it backfill?** A collector
/// that only starts accumulating today is useless for six months, so the
/// backfilling sources are the ones that carry the feature. `GithubRepo` is
/// here anyway and deliberately marked `snapshot_only` — `/stargazers` with
/// `application/vnd.github.star+json` was restricted to repo admins on
/// 2026-06-30, so the classic star-history backfill no longer exists for repos
/// we do not own. Reporting that honestly is more useful than omitting it and
/// letting someone re-derive the same dead end.
///
/// Deliberately absent: Google Trends (application-gated alpha; `pytrends`
/// scrapes an undocumented endpoint against Google's ToS) and Reddit
/// (self-serve registration closed under the Responsible Builder Policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Npm,
    Crates,
    PyPi,
    WikiPageviews,
    HnMentions,
    ArxivCount,
    StackExchangeTag,
    GithubRepo,
    EquityClose,
}

impl SourceKind {
    /// Every variant, in registry order. Used by the tools' error messages and
    /// by the tests that assert the closed set has not quietly grown.
    pub const ALL: &'static [SourceKind] = &[
        SourceKind::Npm,
        SourceKind::Crates,
        SourceKind::PyPi,
        SourceKind::WikiPageviews,
        SourceKind::HnMentions,
        SourceKind::ArxivCount,
        SourceKind::StackExchangeTag,
        SourceKind::GithubRepo,
        SourceKind::EquityClose,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Crates => "crates",
            Self::PyPi => "pypi",
            Self::WikiPageviews => "wiki_pageviews",
            Self::HnMentions => "hn_mentions",
            Self::ArxivCount => "arxiv_count",
            Self::StackExchangeTag => "stackexchange_tag",
            Self::GithubRepo => "github_repo",
            Self::EquityClose => "equity_close",
        }
    }

    /// Parse a source name. The error names the whole closed set, because the
    /// caller is usually a model and "that is not a source" without the list
    /// is an invitation to guess again.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let key = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match key.as_str() {
            "npm" | "npmjs" => Ok(Self::Npm),
            "crates" | "crates_io" | "cratesio" | "cargo" => Ok(Self::Crates),
            "pypi" | "pypistats" | "python" => Ok(Self::PyPi),
            "wiki_pageviews" | "wikipedia" | "wiki" | "pageviews" => Ok(Self::WikiPageviews),
            "hn_mentions" | "hn" | "hackernews" | "hacker_news" => Ok(Self::HnMentions),
            "arxiv_count" | "arxiv" => Ok(Self::ArxivCount),
            "stackexchange_tag" | "stackexchange" | "stackoverflow" | "so_tag" => {
                Ok(Self::StackExchangeTag)
            }
            "github_repo" | "github" | "stars" => Ok(Self::GithubRepo),
            "equity_close" | "equity" | "ticker" | "stock" => Ok(Self::EquityClose),
            other => Err(format!(
                "\"{other}\" is not a market source. Choose one of: {}.",
                Self::ALL
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Can this source hand us the past, or only the present?
    ///
    /// The one property that decides whether a series is worth binding. A
    /// snapshot-only source has to self-accumulate a point per sweep and will
    /// not clear the minimum-length gate for months; the registry says so
    /// rather than presenting one point as a trend.
    pub fn backfills(self) -> bool {
        !matches!(self, Self::GithubRepo)
    }

    /// The cadence the source's own data actually has. npm and Wikimedia are
    /// daily; a snapshot poll is only as dense as our sweep.
    pub fn native_cadence(self) -> Cadence {
        match self {
            Self::Npm
            | Self::Crates
            | Self::PyPi
            | Self::WikiPageviews
            | Self::EquityClose
            | Self::GithubRepo => Cadence::Daily,
            // Counts of *events* (posts, papers, questions) are far too sparse
            // per day to forecast; weekly is the resolution the number has.
            Self::HnMentions | Self::ArxivCount | Self::StackExchangeTag => Cadence::Weekly,
        }
    }

    /// Whether this source is sanctioned by its publisher, stated out loud.
    ///
    /// `market_data.rs` already documents the Yahoo endpoint as "not a
    /// supported API"; persisting its closes would deepen that dependency, so
    /// by default we do not (see `Knobs::persist_equity_closes`).
    pub fn is_official(self) -> bool {
        !matches!(self, Self::EquityClose)
    }

    /// Human label for the Market card and the brief.
    pub fn label(self) -> &'static str {
        match self {
            Self::Npm => "npm downloads",
            Self::Crates => "crates.io downloads",
            Self::PyPi => "PyPI downloads",
            Self::WikiPageviews => "Wikipedia pageviews",
            Self::HnMentions => "Hacker News mentions",
            Self::ArxivCount => "arXiv papers",
            Self::StackExchangeTag => "Stack Exchange questions",
            Self::GithubRepo => "GitHub stars",
            Self::EquityClose => "equity close",
        }
    }
}

/// How often a series has a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    Daily,
    Weekly,
}

impl Cadence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "daily" | "day" | "d" => Ok(Self::Daily),
            "weekly" | "week" | "w" => Ok(Self::Weekly),
            other => Err(format!(
                "\"{other}\" is not a cadence. Use daily or weekly."
            )),
        }
    }

    /// Points below which no method is allowed to speak.
    ///
    /// 180 daily gives at least 25 non-overlapping rolling-origin folds at
    /// H=7 — the smallest number at which the fold-win test in
    /// [`crate::forecaster::backtest`] has any power. 104 weekly is that same
    /// argument at weekly resolution (two years).
    pub fn min_points(self) -> usize {
        match self {
            Self::Daily => 180,
            Self::Weekly => 104,
        }
    }

    /// Seasonal period for the seasonal-naive denominator: a week in daily
    /// points, a year in weekly ones.
    pub fn seasonal_period(self) -> usize {
        match self {
            Self::Daily => 7,
            Self::Weekly => 52,
        }
    }

    /// The default forecast horizon: one week ahead either way.
    pub fn default_horizon(self) -> usize {
        match self {
            Self::Daily => 7,
            Self::Weekly => 4,
        }
    }
}

/// Where a series sits in the review gate.
///
/// Binding *proposes*; a human approves. The Forecaster never promotes its own
/// series, for the same reason `propose_project_intel` never applies its own
/// findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesStatus {
    Proposed,
    Active,
    Dismissed,
}

impl SeriesStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "proposed" => Ok(Self::Proposed),
            "active" => Ok(Self::Active),
            "dismissed" => Ok(Self::Dismissed),
            other => Err(format!("\"{other}\" is not a series status.")),
        }
    }
}

/// One bound series: a `project_intel` row (or a project) plus a number to
/// watch. `intel_id` is nullable and un-foreign-keyed on purpose — dismissing
/// an intelligence item must not silently delete months of collected history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub id: String,
    pub project_id: String,
    pub intel_id: Option<String>,
    pub source_kind: SourceKind,
    pub subject: String,
    pub cadence: Cadence,
    pub label: String,
    pub status: SeriesStatus,
    pub last_collected_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
}

/// One competitor, spelled the way each source spells it.
///
/// This is the answer to "does `subject` need normalization?": yes, but a
/// four-row table beats a second identity system. A canonical `group` maps to
/// the per-source subject; `resolve` applies it on bind, `subject_group` reads
/// it back so the Market card can show four rows as one competitor.
pub struct SubjectAlias {
    pub group: &'static str,
    pub kind: SourceKind,
    pub subject: &'static str,
}

/// The mapping table. Small and hand-maintained on purpose — it exists to stop
/// the same competitor appearing four times under four spellings, not to model
/// the software ecosystem.
pub const SUBJECT_ALIASES: &[SubjectAlias] = &[
    SubjectAlias {
        group: "langchain",
        kind: SourceKind::Npm,
        subject: "langchain",
    },
    SubjectAlias {
        group: "langchain",
        kind: SourceKind::PyPi,
        subject: "langchain",
    },
    SubjectAlias {
        group: "langchain",
        kind: SourceKind::WikiPageviews,
        subject: "LangChain",
    },
    SubjectAlias {
        group: "langchain",
        kind: SourceKind::HnMentions,
        subject: "langchain",
    },
    SubjectAlias {
        group: "llamaindex",
        kind: SourceKind::PyPi,
        subject: "llama-index",
    },
    SubjectAlias {
        group: "llamaindex",
        kind: SourceKind::Npm,
        subject: "llamaindex",
    },
    SubjectAlias {
        group: "llamaindex",
        kind: SourceKind::HnMentions,
        subject: "llamaindex",
    },
    SubjectAlias {
        group: "ollama",
        kind: SourceKind::Npm,
        subject: "ollama",
    },
    SubjectAlias {
        group: "ollama",
        kind: SourceKind::PyPi,
        subject: "ollama",
    },
    SubjectAlias {
        group: "ollama",
        kind: SourceKind::HnMentions,
        subject: "ollama",
    },
    SubjectAlias {
        group: "ollama",
        kind: SourceKind::GithubRepo,
        subject: "ollama/ollama",
    },
    SubjectAlias {
        group: "coworking",
        kind: SourceKind::WikiPageviews,
        subject: "Coworking",
    },
    SubjectAlias {
        group: "coworking",
        kind: SourceKind::HnMentions,
        subject: "coworking",
    },
];

/// Apply the alias table: a canonical group name becomes this source's
/// spelling of it. Anything not in the table is passed through untouched — the
/// table is a convenience, never a whitelist.
pub fn resolve_alias(kind: SourceKind, raw: &str) -> Option<&'static str> {
    let key = raw.trim().to_ascii_lowercase();
    SUBJECT_ALIASES
        .iter()
        .find(|a| a.kind == kind && a.group == key)
        .map(|a| a.subject)
}

/// The reverse read: which competitor is this per-source string about?
pub fn subject_group(kind: SourceKind, subject: &str) -> Option<&'static str> {
    let key = subject.trim().to_ascii_lowercase();
    SUBJECT_ALIASES
        .iter()
        .find(|a| a.kind == kind && a.subject.to_ascii_lowercase() == key)
        .map(|a| a.group)
}

/// Longest subject we will accept. Generous for an article title, far short of
/// anything that could be a smuggled payload.
const MAX_SUBJECT: usize = 200;

/// Normalize a subject into the spelling its source uses, refusing anything
/// that could not be a package / article / tag / ticker.
///
/// This runs *before* a URL is ever built. The character allowlists are the
/// point: a subject that reaches the network has already been proven to be a
/// name, so no collector has to reason about escaping a hostile one.
pub fn normalize_subject(kind: SourceKind, raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("subject is empty".into());
    }
    if raw.len() > MAX_SUBJECT {
        return Err(format!("subject is longer than {MAX_SUBJECT} characters"));
    }
    if let Some(alias) = resolve_alias(kind, raw) {
        return Ok(alias.to_string());
    }
    // Nothing addressable contains a control character, and a newline in a
    // subject is the shape of an attempt to smuggle a second request.
    if raw.chars().any(|c| c.is_control()) {
        return Err("subject contains a control character".into());
    }
    let out = match kind {
        SourceKind::Npm => {
            let s = raw.to_ascii_lowercase();
            let body = s.strip_prefix('@').unwrap_or(&s);
            // `..` and a leading dot are excluded explicitly: the npm subject
            // is the only one that keeps its slash on the way into a URL path
            // (scoped packages need it), so it is the only one where a dot
            // segment could walk out of its endpoint.
            let ok = !body.is_empty()
                && body.matches('/').count() <= 1
                && !body.contains("..")
                && !body.starts_with('.')
                && !body
                    .split('/')
                    .any(|seg| seg.is_empty() || seg.starts_with('.'))
                && body
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
            if !ok {
                return Err(format!("\"{raw}\" is not an npm package name"));
            }
            s
        }
        SourceKind::Crates => {
            let s = raw.to_ascii_lowercase();
            if !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            {
                return Err(format!("\"{raw}\" is not a crates.io crate name"));
            }
            s
        }
        SourceKind::PyPi => {
            // PEP 503: lowercase, runs of -_. collapse to a single -.
            let lowered = raw.to_ascii_lowercase();
            if !lowered
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return Err(format!("\"{raw}\" is not a PyPI project name"));
            }
            let mut s = String::with_capacity(lowered.len());
            let mut in_sep = false;
            for c in lowered.chars() {
                if matches!(c, '-' | '_' | '.') {
                    if !in_sep {
                        s.push('-');
                        in_sep = true;
                    }
                } else {
                    s.push(c);
                    in_sep = false;
                }
            }
            let s = s.trim_matches('-').to_string();
            if s.is_empty() {
                return Err(format!("\"{raw}\" is not a PyPI project name"));
            }
            s
        }
        SourceKind::WikiPageviews => {
            // Article titles keep their case; spaces are underscores.
            if raw.contains(['?', '#', '&']) {
                return Err(format!("\"{raw}\" is not a Wikipedia article title"));
            }
            raw.replace(' ', "_")
        }
        SourceKind::HnMentions | SourceKind::ArxivCount => {
            // Free-text queries. Collapse whitespace so the same query does not
            // become two series, and refuse the characters that would let a
            // query become extra query parameters.
            if raw.contains(['&', '?', '=', '#']) {
                return Err(format!("\"{raw}\" contains a query separator"));
            }
            raw.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        SourceKind::StackExchangeTag => {
            let s = raw.to_ascii_lowercase();
            if !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+' | '#'))
            {
                return Err(format!("\"{raw}\" is not a Stack Exchange tag"));
            }
            s
        }
        SourceKind::GithubRepo => {
            let s = raw.trim_start_matches("https://github.com/").to_string();
            let s = s.trim_end_matches('/').to_string();
            let mut parts = s.split('/');
            let (owner, repo) = match (parts.next(), parts.next(), parts.next()) {
                (Some(o), Some(r), None) if !o.is_empty() && !r.is_empty() => (o, r),
                _ => return Err(format!("\"{raw}\" is not an owner/repo pair")),
            };
            let name_ok = |p: &str| {
                !p.starts_with('.')
                    && !p.contains("..")
                    && p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            };
            if !name_ok(owner) || !name_ok(repo) {
                return Err(format!("\"{raw}\" is not an owner/repo pair"));
            }
            format!("{owner}/{repo}")
        }
        SourceKind::EquityClose => {
            let s = raw.to_ascii_uppercase();
            if s.len() > 24
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '^'))
            {
                return Err(format!("\"{raw}\" is not a ticker"));
            }
            s
        }
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_source_kind_is_rejected_before_it_becomes_a_url() {
        let err = SourceKind::parse("google_trends").unwrap_err();
        assert!(err.contains("is not a market source"), "{err}");
        // The refusal has to name the alternatives, or the next guess is
        // another invented source.
        assert!(
            err.contains("npm") && err.contains("wiki_pageviews"),
            "{err}"
        );
        assert!(SourceKind::parse("reddit").is_err());
        assert!(SourceKind::parse("").is_err());
        // And the closed set stays closed.
        assert_eq!(SourceKind::ALL.len(), 9);
    }

    #[test]
    fn a_snapshot_only_source_reports_snapshot_not_a_series() {
        assert!(!SourceKind::GithubRepo.backfills());
        for kind in SourceKind::ALL {
            if *kind != SourceKind::GithubRepo {
                assert!(kind.backfills(), "{} must backfill", kind.as_str());
            }
        }
    }

    #[test]
    fn subjects_normalize_per_source() {
        assert_eq!(
            normalize_subject(SourceKind::PyPi, "LangChain_Core.Ext").unwrap(),
            "langchain-core-ext"
        );
        assert_eq!(
            normalize_subject(SourceKind::Npm, "@LangChain/Core").unwrap(),
            "@langchain/core"
        );
        assert_eq!(
            normalize_subject(SourceKind::WikiPageviews, "Large language model").unwrap(),
            "Large_language_model"
        );
        assert_eq!(
            normalize_subject(SourceKind::EquityClose, "  wework ").unwrap(),
            "WEWORK"
        );
        assert_eq!(
            normalize_subject(SourceKind::GithubRepo, "https://github.com/ollama/ollama/").unwrap(),
            "ollama/ollama"
        );
        assert_eq!(
            normalize_subject(SourceKind::HnMentions, "agent   memory").unwrap(),
            "agent memory"
        );
    }

    #[test]
    fn a_subject_that_could_become_a_url_is_refused() {
        assert!(normalize_subject(SourceKind::WikiPageviews, "Foo?action=delete").is_err());
        assert!(normalize_subject(SourceKind::HnMentions, "a&b=c").is_err());
        assert!(normalize_subject(SourceKind::Npm, "../../etc/passwd").is_err());
        // One slash and only allowed characters — and still a path escape, so
        // dot segments are refused by name rather than by accident.
        assert!(normalize_subject(SourceKind::Npm, "../vitest").is_err());
        assert!(normalize_subject(SourceKind::Npm, "@scope/..").is_err());
        assert!(normalize_subject(SourceKind::GithubRepo, "../x").is_err());
        assert!(normalize_subject(SourceKind::Crates, "serde json").is_err());
        assert!(normalize_subject(SourceKind::WikiPageviews, "Foo\nBar").is_err());
        // A bare name with no alias entry cannot become an owner/repo path.
        assert!(normalize_subject(SourceKind::GithubRepo, "vitest").is_err());
        assert!(normalize_subject(SourceKind::GithubRepo, "a/b/c").is_err());
        // "ollama" IS in the alias table, and resolving it is the table doing
        // its job — one competitor, each source's spelling.
        assert_eq!(
            normalize_subject(SourceKind::GithubRepo, "ollama").unwrap(),
            "ollama/ollama"
        );
        assert!(normalize_subject(SourceKind::Npm, &"x".repeat(300)).is_err());
    }

    #[test]
    fn one_competitor_maps_to_each_sources_spelling() {
        assert_eq!(
            normalize_subject(SourceKind::PyPi, "LlamaIndex").unwrap(),
            "llama-index"
        );
        assert_eq!(
            subject_group(SourceKind::PyPi, "llama-index"),
            Some("llamaindex")
        );
        assert_eq!(
            subject_group(SourceKind::Npm, "llamaindex"),
            Some("llamaindex")
        );
        // Not in the table: passed straight through, never rejected.
        assert_eq!(
            normalize_subject(SourceKind::Npm, "vitest").unwrap(),
            "vitest"
        );
        assert_eq!(subject_group(SourceKind::Npm, "vitest"), None);
    }

    #[test]
    fn cadence_carries_its_own_minimum_and_season() {
        assert_eq!(Cadence::Daily.min_points(), 180);
        assert_eq!(Cadence::Weekly.min_points(), 104);
        assert_eq!(Cadence::Daily.seasonal_period(), 7);
        assert_eq!(Cadence::Weekly.seasonal_period(), 52);
        assert!(Cadence::parse("fortnightly").is_err());
    }
}
