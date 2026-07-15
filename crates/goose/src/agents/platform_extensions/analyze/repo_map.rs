//! Ranked-tags codebase map — a cheap, token-budgeted view of a repo's
//! *structure* (symbol signatures, not file bodies).
//!
//! This is a Rust port of [Aider's repo-map](https://aider.chat/docs/repomap.html)
//! built on Permagent's existing tree-sitter analyzer ([`super::parser`]). The
//! goal is the input-token cost lever: give a model the shape of a codebase —
//! the important symbols and their signatures — within a small token budget,
//! instead of dumping whole files.
//!
//! Pipeline (Aider's, proven):
//! 1. **Extract def/ref tags.** Reuses [`super::AnalyzeClient::analyze_file`] →
//!    [`FileAnalysis`]: `functions`/`classes` are *definitions* (their
//!    `detail` is a compact signature) and `calls` are *references*. No new
//!    tree-sitter queries are added — the analyzer already produces both.
//! 2. **Build a graph** ([`petgraph`]): one node per file, one weighted edge
//!    per reference `referencer → definer`, labelled with the referenced ident.
//! 3. **Rank** with a *personalized, weighted* PageRank biased toward a
//!    caller-supplied focus set (the "chat files" + mentioned identifiers), so
//!    the map centers on what the task is about.
//! 4. **Render** the top-ranked symbols as signatures, binary-searching the tag
//!    count to fill a token budget. Extracted tags are cached by file mtime so
//!    re-runs over an unchanged tree re-parse nothing.
//!
//! Note on [`petgraph::algo::page_rank`]: it exists but computes *unweighted*
//! PageRank with *uniform* teleport — it accepts neither per-edge weights nor a
//! personalization vector, both of which Aider's focus-biased ranking requires.
//! We use petgraph for the graph structure and implement the personalized,
//! weighted power iteration ([`personalized_pagerank`]) on top of it.
//!
//! This is a standalone, testable capability. It is intentionally **not** wired
//! into any coding loop or context-injection path yet.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use petgraph::graph::{DiGraph, NodeIndex};

use super::parser::FileAnalysis;
use super::AnalyzeClient;

// ── Tuning constants ───────────────────────────────────────────────────

/// PageRank damping factor (probability of following an edge vs. teleporting).
const DAMPING: f64 = 0.85;
/// Maximum power-iteration steps.
const MAX_ITER: usize = 100;
/// Convergence tolerance (L1 delta, scaled by node count).
const TOL: f64 = 1e-6;

/// Edge-weight multiplier for an identifier the caller explicitly mentioned.
const MENTIONED_MULTIPLIER: f64 = 10.0;
/// Edge-weight multiplier for a "private" identifier (leading underscore).
const PRIVATE_MULTIPLIER: f64 = 0.1;

// ── Public config ──────────────────────────────────────────────────────

/// Tunables for [`RepoMap`].
#[derive(Clone, Debug)]
pub struct RepoMapConfig {
    /// Target token budget for the rendered map when a focus set is given.
    pub max_map_tokens: usize,
    /// When no focus files/idents are supplied, the budget is multiplied by
    /// this factor (Aider uses 8×) — with nothing to center on, a broader map
    /// is more useful.
    pub no_focus_multiplier: usize,
    /// Directory recursion depth for [`RepoMap::map`] (0 = unlimited).
    pub max_depth: u32,
    /// Characters-per-token divisor for the (deterministic) token estimate.
    pub chars_per_token: usize,
}

impl Default for RepoMapConfig {
    fn default() -> Self {
        Self {
            max_map_tokens: 1024,
            no_focus_multiplier: 8,
            max_depth: 0,
            chars_per_token: 4,
        }
    }
}

// ── Tags ───────────────────────────────────────────────────────────────

/// What kind of definition a tag represents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefKind {
    Function,
    Class,
}

/// A single symbol definition extracted from a file.
#[derive(Clone, Debug)]
pub struct Def {
    pub name: String,
    pub line: usize,
    pub kind: DefKind,
    /// Compact signature (e.g. `(x: i32)->bool` for a fn, `(Base){a,b}` for a
    /// class), as produced by the analyzer. `None` when unavailable.
    pub signature: Option<String>,
    /// Enclosing type/class for a method, when applicable.
    pub parent: Option<String>,
}

/// The def/ref tags extracted from one file.
#[derive(Clone, Debug, Default)]
pub struct FileTags {
    pub defs: Vec<Def>,
    /// Referenced identifier → number of references within this file.
    pub refs: HashMap<String, usize>,
}

/// Extract def/ref tags from an already-parsed [`FileAnalysis`].
///
/// Definitions come from `functions` + `classes` (carrying their signature
/// `detail`); references come from `calls`, whose callee names are reduced to
/// their bare identifier (matching the analyzer's own `graph.rs` convention, so
/// `Type::method` resolves against a `method` definition).
pub fn tags_from_analysis(a: &FileAnalysis) -> FileTags {
    let mut defs = Vec::with_capacity(a.functions.len() + a.classes.len());
    for f in &a.functions {
        defs.push(Def {
            name: f.name.clone(),
            line: f.line,
            kind: DefKind::Function,
            signature: f.detail.clone(),
            parent: f.parent.clone(),
        });
    }
    for c in &a.classes {
        defs.push(Def {
            name: c.name.clone(),
            line: c.line,
            kind: DefKind::Class,
            signature: c.detail.clone(),
            parent: c.parent.clone(),
        });
    }

    let mut refs: HashMap<String, usize> = HashMap::new();
    for call in &a.calls {
        let bare = bare_ident(&call.callee);
        if bare.is_empty() {
            continue;
        }
        *refs.entry(bare.to_string()).or_default() += 1;
    }

    FileTags { defs, refs }
}

/// Reduce a (possibly scoped) callee to its bare identifier: `Type::new` → `new`.
fn bare_ident(s: &str) -> &str {
    s.rsplit("::").next().unwrap_or(s).trim()
}

// ── Ranking ────────────────────────────────────────────────────────────

/// A definition with its computed importance rank.
#[derive(Clone, Debug)]
struct RankedTag {
    file: PathBuf,
    def: Def,
    rank: f64,
}

/// A weighted ref→def edge between two files, labelled with the referenced ident.
#[derive(Clone, Debug)]
struct RefEdge {
    from: PathBuf,
    to: PathBuf,
    ident: String,
    weight: f64,
}

/// petgraph edge payload.
struct EdgeW {
    weight: f64,
    ident: String,
}

/// Per-ident edge-weight multiplier: boost mentioned idents, damp private ones.
fn ident_multiplier(ident: &str, mentioned: &HashSet<String>) -> f64 {
    let mut mul = 1.0;
    if mentioned.contains(ident) {
        mul *= MENTIONED_MULTIPLIER;
    }
    if ident.starts_with('_') {
        mul *= PRIVATE_MULTIPLIER;
    }
    mul
}

/// Indexed view of the corpus: which files define / reference each ident.
struct TagIndex<'a> {
    /// ident → files that define it.
    defines: HashMap<&'a str, HashSet<&'a Path>>,
    /// ident → (file → reference count).
    references: HashMap<&'a str, HashMap<&'a Path, usize>>,
}

/// Index defs and refs across all files.
fn index_tags(tags: &HashMap<PathBuf, FileTags>) -> TagIndex<'_> {
    let mut defines: HashMap<&str, HashSet<&Path>> = HashMap::new();
    let mut references: HashMap<&str, HashMap<&Path, usize>> = HashMap::new();

    for (path, ft) in tags {
        for d in &ft.defs {
            defines.entry(&d.name).or_default().insert(path.as_path());
        }
        for (ident, cnt) in &ft.refs {
            *references
                .entry(ident.as_str())
                .or_default()
                .entry(path.as_path())
                .or_default() += *cnt;
        }
    }

    // Aider fallback: with no references anywhere, treat each definition as a
    // self-reference so a corpus of isolated files can still be ranked.
    if references.is_empty() {
        for (ident, files) in &defines {
            let bucket = references.entry(*ident).or_default();
            for f in files {
                *bucket.entry(*f).or_default() += 1;
            }
        }
    }

    TagIndex {
        defines,
        references,
    }
}

/// Build the weighted ref→def edge list. One edge per
/// `(referencer file, definer file, ident)`; weight = `multiplier · √refs`
/// (the √ dampens high-frequency idents, matching Aider).
fn build_ref_edges(index: &TagIndex<'_>, mentioned: &HashSet<String>) -> Vec<RefEdge> {
    let mut edges = Vec::new();
    for (&ident, definers) in &index.defines {
        let Some(refs) = index.references.get(ident) else {
            continue; // defined but never referenced → contributes no edge
        };
        let mul = ident_multiplier(ident, mentioned);
        for (referencer, num_refs) in refs {
            let weight = mul * (*num_refs as f64).sqrt();
            for definer in definers {
                edges.push(RefEdge {
                    from: referencer.to_path_buf(),
                    to: definer.to_path_buf(),
                    ident: ident.to_string(),
                    weight,
                });
            }
        }
    }
    edges
}

/// Personalized, weighted PageRank via power iteration (networkx semantics,
/// which Aider relies on): teleport mass and dangling-node mass are both
/// redistributed according to `personalization`. `personalization` must sum to
/// 1 and have one entry per node.
fn personalized_pagerank(
    g: &DiGraph<(), EdgeW>,
    personalization: &[f64],
    damping: f64,
    max_iter: usize,
) -> Vec<f64> {
    let n = g.node_count();
    if n == 0 {
        return vec![];
    }

    // Total outgoing weight per node (0 ⇒ dangling).
    let mut out_sum = vec![0.0f64; n];
    for e in g.edge_indices() {
        let (s, _t) = g.edge_endpoints(e).expect("edge has endpoints");
        out_sum[s.index()] += g[e].weight;
    }

    let mut rank = vec![1.0 / n as f64; n];
    for _ in 0..max_iter {
        let mut next = vec![0.0f64; n];

        // Dangling mass (rank sitting on nodes with no out-edges; out_sum ≥ 0,
        // so `<= 0.0` means exactly zero).
        let dangling: f64 = out_sum
            .iter()
            .zip(&rank)
            .filter(|(o, _)| **o <= 0.0)
            .map(|(_, r)| *r)
            .sum();

        // Base term: teleport + dangling, both steered by personalization.
        for (slot, &p) in next.iter_mut().zip(personalization.iter()) {
            *slot = (1.0 - damping) * p + damping * dangling * p;
        }

        // Edge term: each node pushes its rank along out-edges, split by weight.
        for e in g.edge_indices() {
            let (s, t) = g.edge_endpoints(e).expect("edge has endpoints");
            let si = s.index();
            if out_sum[si] > 0.0 {
                next[t.index()] += damping * rank[si] * (g[e].weight / out_sum[si]);
            }
        }

        let err: f64 = next.iter().zip(&rank).map(|(a, b)| (a - b).abs()).sum();
        rank = next;
        if err < n as f64 * TOL {
            break;
        }
    }

    rank
}

/// Rank every `(file, symbol)` definition. Files with a focus/personalization
/// bias and the identifiers they reference float to the top.
///
/// Returns definitions ordered by importance (desc). Graph-ranked definitions
/// (those reachable through references) come first; any remaining unreferenced
/// definitions follow, ordered by their file's rank then line, so the budget
/// can still surface them when there is room.
fn rank_tags(
    tags: &HashMap<PathBuf, FileTags>,
    focus_files: &HashSet<PathBuf>,
    mentioned: &HashSet<String>,
) -> Vec<RankedTag> {
    if tags.is_empty() {
        return vec![];
    }

    let index = index_tags(tags);
    let edges = build_ref_edges(&index, mentioned);

    // Build the petgraph: one node per file (including isolated ones, so they
    // can carry personalization and appear in the remainder).
    let mut g: DiGraph<(), EdgeW> = DiGraph::new();
    let mut node_of: HashMap<&Path, NodeIndex> = HashMap::new();
    for path in tags.keys() {
        node_of.insert(path.as_path(), g.add_node(()));
    }
    for e in &edges {
        let (Some(&s), Some(&t)) = (node_of.get(e.from.as_path()), node_of.get(e.to.as_path()))
        else {
            continue;
        };
        g.add_edge(
            s,
            t,
            EdgeW {
                weight: e.weight,
                ident: e.ident.clone(),
            },
        );
    }

    let n = g.node_count();
    let mut path_of: Vec<&Path> = vec![Path::new(""); n];
    for (p, idx) in &node_of {
        path_of[idx.index()] = *p;
    }

    // Personalization vector: mass on focus files, else uniform.
    let focus_nodes: Vec<usize> = focus_files
        .iter()
        .filter_map(|f| node_of.get(f.as_path()).map(|idx| idx.index()))
        .collect();
    let mut personalization = vec![0.0f64; n];
    if focus_nodes.is_empty() {
        for p in personalization.iter_mut() {
            *p = 1.0 / n as f64;
        }
    } else {
        let share = 1.0 / focus_nodes.len() as f64;
        for &idx in &focus_nodes {
            personalization[idx] = share;
        }
    }

    let rank = personalized_pagerank(&g, &personalization, DAMPING, MAX_ITER);

    // Back-distribute each file's rank across its out-edges, crediting the
    // (definer file, ident) it points at. Zero-rank sources (e.g. a component
    // unreachable from the focus set) credit nothing, so their definitions fall
    // through to the file-rank-ordered remainder rather than jumping the queue.
    let mut out_sum = vec![0.0f64; n];
    for e in g.edge_indices() {
        let (s, _t) = g.edge_endpoints(e).expect("edge has endpoints");
        out_sum[s.index()] += g[e].weight;
    }
    let mut ranked_def: HashMap<(PathBuf, String), f64> = HashMap::new();
    for e in g.edge_indices() {
        let (s, t) = g.edge_endpoints(e).expect("edge has endpoints");
        let si = s.index();
        if out_sum[si] <= 0.0 {
            continue;
        }
        let contrib = rank[si] * (g[e].weight / out_sum[si]);
        if contrib <= 0.0 {
            continue;
        }
        let key = (path_of[t.index()].to_path_buf(), g[e].ident.clone());
        *ranked_def.entry(key).or_default() += contrib;
    }

    let mut file_rank: HashMap<&Path, f64> = HashMap::new();
    for (p, idx) in &node_of {
        file_rank.insert(*p, rank[idx.index()]);
    }

    // Phase 1: graph-ranked definitions, highest first (deterministic ties).
    let mut items: Vec<((PathBuf, String), f64)> = ranked_def.into_iter().collect();
    items.sort_by(|a, b| {
        let ((pa, ia), ra) = a;
        let ((pb, ib), rb) = b;
        rb.partial_cmp(ra)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| pa.cmp(pb))
            .then_with(|| ia.cmp(ib))
    });

    let mut ranked: Vec<RankedTag> = Vec::new();
    let mut seen: HashSet<(PathBuf, String, usize)> = HashSet::new();
    for ((path, ident), r) in items {
        if let Some(ft) = tags.get(&path) {
            for d in ft.defs.iter().filter(|d| d.name == ident) {
                if seen.insert((path.clone(), d.name.clone(), d.line)) {
                    ranked.push(RankedTag {
                        file: path.clone(),
                        def: d.clone(),
                        rank: r,
                    });
                }
            }
        }
    }

    // Phase 2: remaining (unreferenced) definitions, ordered by file rank.
    let mut rest: Vec<RankedTag> = Vec::new();
    for (path, ft) in tags {
        let fr = file_rank.get(path.as_path()).copied().unwrap_or(0.0);
        for d in &ft.defs {
            if !seen.contains(&(path.clone(), d.name.clone(), d.line)) {
                rest.push(RankedTag {
                    file: path.clone(),
                    def: d.clone(),
                    rank: fr,
                });
            }
        }
    }
    rest.sort_by(|a, b| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.def.line.cmp(&b.def.line))
    });
    ranked.extend(rest);
    ranked
}

// ── Rendering + budget ─────────────────────────────────────────────────

/// One-line signature label for a definition, matching the `analyze` tool's
/// house style (`Parent.name(sig)->Ret:line` for fns, `Name:line(detail)` for
/// classes).
fn def_label(d: &Def) -> String {
    match d.kind {
        DefKind::Function => {
            let mut s = String::new();
            if let Some(p) = &d.parent {
                s.push_str(p);
                s.push('.');
            }
            s.push_str(&d.name);
            if let Some(sig) = &d.signature {
                s.push_str(sig);
            }
            s.push(':');
            s.push_str(&d.line.to_string());
            s
        }
        DefKind::Class => {
            let mut s = format!("{}:{}", d.name, d.line);
            if let Some(sig) = &d.signature {
                s.push_str(sig);
            }
            s
        }
    }
}

/// Render the first `count` ranked tags as a per-file signature map. Files
/// appear in rank order (first appearance); symbols within a file are ordered
/// by line. Output length is non-decreasing in `count`, so a binary search over
/// `count` is well-defined.
fn render_tags(ranked: &[RankedTag], count: usize, root: &Path) -> String {
    let n = count.min(ranked.len());
    if n == 0 {
        return String::new();
    }

    let mut file_order: Vec<&Path> = Vec::new();
    let mut by_file: HashMap<&Path, Vec<&RankedTag>> = HashMap::new();
    for t in &ranked[..n] {
        let p = t.file.as_path();
        if !by_file.contains_key(p) {
            file_order.push(p);
        }
        by_file.entry(p).or_default().push(t);
    }

    let mut out = String::new();
    for (i, &p) in file_order.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let rel = p.strip_prefix(root).unwrap_or(p);
        out.push_str(&rel.display().to_string());
        out.push_str(":\n");
        let mut defs = by_file[p].clone();
        defs.sort_by_key(|t| t.def.line);
        for t in defs {
            out.push_str("  ");
            out.push_str(&def_label(&t.def));
            out.push('\n');
        }
    }
    out
}

/// Deterministic token estimate: `ceil(chars / chars_per_token)`.
fn estimate_tokens(s: &str, chars_per_token: usize) -> usize {
    s.len().div_ceil(chars_per_token.max(1))
}

/// Render the largest prefix of `ranked` whose token estimate fits `budget`,
/// found by binary search over the tag count. Never overflows the budget.
fn fill_budget(ranked: &[RankedTag], budget_tokens: usize, root: &Path, cpt: usize) -> String {
    if ranked.is_empty() {
        return String::new();
    }
    // Fast path: everything fits.
    let full = render_tags(ranked, ranked.len(), root);
    if estimate_tokens(&full, cpt) <= budget_tokens {
        return full;
    }

    // Binary search the largest count that fits.
    let mut lo = 0usize;
    let mut hi = ranked.len();
    let mut best = String::new();
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let candidate = render_tags(ranked, mid, root);
        if estimate_tokens(&candidate, cpt) <= budget_tokens {
            best = candidate;
            lo = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    best
}

// ── RepoMap: the cached, top-level capability ──────────────────────────

/// mtime-keyed cache of a file's extracted tags.
struct CacheEntry {
    mtime: Option<SystemTime>,
    tags: FileTags,
}

/// Builds ranked-tags maps, caching extracted tags by file mtime so re-runs
/// over an unchanged tree re-parse nothing.
pub struct RepoMap {
    config: RepoMapConfig,
    cache: HashMap<PathBuf, CacheEntry>,
    /// Cache misses (files actually parsed) during the most recent map build.
    last_parsed: usize,
}

impl Default for RepoMap {
    fn default() -> Self {
        Self::new()
    }
}

impl RepoMap {
    pub fn new() -> Self {
        Self::with_config(RepoMapConfig::default())
    }

    pub fn with_config(config: RepoMapConfig) -> Self {
        Self {
            config,
            cache: HashMap::new(),
            last_parsed: 0,
        }
    }

    /// Number of files parsed (cache misses) during the most recent
    /// [`Self::map`] / [`Self::map_files`] call.
    pub fn files_parsed_last(&self) -> usize {
        self.last_parsed
    }

    /// Build a ranked-tags map for the tree rooted at `root`, biased toward the
    /// given focus files and mentioned identifiers.
    pub fn map(&mut self, root: &Path, focus_files: &[PathBuf], mentioned: &[String]) -> String {
        let files = AnalyzeClient::collect_files(root, self.config.max_depth);
        self.map_files(root, &files, focus_files, mentioned)
    }

    /// Like [`Self::map`] but over an explicit file list (paths displayed
    /// relative to `root`).
    pub fn map_files(
        &mut self,
        root: &Path,
        files: &[PathBuf],
        focus_files: &[PathBuf],
        mentioned: &[String],
    ) -> String {
        let tags = self.collect_tags(files);
        let focus_set: HashSet<PathBuf> = focus_files.iter().cloned().collect();
        let mentioned_set: HashSet<String> = mentioned.iter().cloned().collect();
        let ranked = rank_tags(&tags, &focus_set, &mentioned_set);

        let budget = if focus_files.is_empty() && mentioned.is_empty() {
            self.config
                .max_map_tokens
                .saturating_mul(self.config.no_focus_multiplier)
        } else {
            self.config.max_map_tokens
        };
        fill_budget(&ranked, budget, root, self.config.chars_per_token)
    }

    /// Extract tags for every file, honoring the mtime cache. Records the number
    /// of cache misses in `last_parsed`.
    fn collect_tags(&mut self, files: &[PathBuf]) -> HashMap<PathBuf, FileTags> {
        self.last_parsed = 0;
        let mut out = HashMap::new();
        for f in files {
            let mtime = file_mtime(f);
            // Cache hit: same file, same mtime → reuse the parsed tags.
            if let Some(entry) = self.cache.get(f).filter(|e| e.mtime == mtime) {
                out.insert(f.clone(), entry.tags.clone());
                continue;
            }
            // Cache miss: parse via the shared analyzer.
            let Some(analysis) = AnalyzeClient::analyze_file(f) else {
                continue; // unsupported language / unreadable — skip
            };
            let tags = tags_from_analysis(&analysis);
            self.last_parsed += 1;
            self.cache.insert(
                f.clone(),
                CacheEntry {
                    mtime,
                    tags: tags.clone(),
                },
            );
            out.insert(f.clone(), tags);
        }
        out
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

// ── Coding-harness context injection ────────────────────────────────────

/// Build the coding harness's repo-map context block for the tree rooted at
/// `root`, token-budgeted to `max_map_tokens`. Returns `None` when the tree has
/// no analyzable source, so the caller injects nothing rather than an empty
/// block.
///
/// The map is built with no focus set (a whole-repo orientation view at session
/// start) and wrapped in a labelled `<repo_map>` element. It belongs in the
/// cache-stable prefix at `PrefixSegment::RepoMap` (tools → system → repo-map →
/// read-only files) — see `crate::cost_router::cache`. `.gitignore` / hidden
/// files are excluded by `AnalyzeClient::collect_files`, and the map shows
/// signatures, not bodies, so a whole repo fits a small token budget.
pub fn coding_context_block(root: &Path, max_map_tokens: usize) -> Option<String> {
    let config = RepoMapConfig {
        max_map_tokens,
        ..RepoMapConfig::default()
    };
    let map = RepoMap::with_config(config).map(root, &[], &[]);
    if map.trim().is_empty() {
        return None;
    }
    Some(format!(
        "<repo_map>\n\
         A ranked-tags map of this repository's structure — its most important \
         symbols and their signatures (not file bodies), ranked by how connected \
         they are. Use it to orient before reading files. It is a hint, not \
         authoritative; read a file before editing it.\n\n\
         {map}</repo_map>"
    ))
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn func(name: &str, line: usize, sig: &str) -> Def {
        Def {
            name: name.to_string(),
            line,
            kind: DefKind::Function,
            signature: Some(sig.to_string()),
            parent: None,
        }
    }

    fn tags_of(defs: Vec<Def>, refs: &[(&str, usize)]) -> FileTags {
        FileTags {
            defs,
            refs: refs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    fn rank_pos(ranked: &[RankedTag], name: &str) -> Option<usize> {
        ranked.iter().position(|t| t.def.name == name)
    }

    // ---- Tag extraction (real tree-sitter, two languages) ----

    #[test]
    fn extracts_rust_defs_and_refs() {
        let src = r#"
fn validate(x: i32) -> bool { x > 0 }
fn process() {
    validate(1);
    validate(2);
}
struct Config;
"#;
        let dir = tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        fs::write(&path, src).unwrap();

        let analysis = AnalyzeClient::analyze_file(&path).unwrap();
        let ft = tags_from_analysis(&analysis);

        let names: HashSet<&str> = ft.defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains("validate"));
        assert!(names.contains("process"));
        assert!(names.contains("Config"));

        // `validate` is referenced (twice) from `process`.
        assert!(ft.refs.get("validate").copied().unwrap_or(0) >= 2);

        // The function definition carries a signature with its return type.
        let v = ft.defs.iter().find(|d| d.name == "validate").unwrap();
        assert!(v.signature.as_deref().unwrap_or("").contains("bool"));
    }

    #[test]
    fn extracts_python_defs_and_refs() {
        let src = "def main():\n    helper()\n    helper()\n\ndef helper():\n    pass\n";
        let dir = tempdir().unwrap();
        let path = dir.path().join("app.py");
        fs::write(&path, src).unwrap();

        let analysis = AnalyzeClient::analyze_file(&path).unwrap();
        let ft = tags_from_analysis(&analysis);

        let names: HashSet<&str> = ft.defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains("main"));
        assert!(names.contains("helper"));
        assert!(ft.refs.get("helper").copied().unwrap_or(0) >= 2);
    }

    // ---- Graph construction: ref→def edges ----

    #[test]
    fn builds_correct_ref_to_def_edges() {
        // a.rs references `target` (defined in b.rs) and `gamma` (defined in c.rs).
        let mut tags = HashMap::new();
        tags.insert(
            pb("a.rs"),
            tags_of(vec![func("alpha", 1, "()")], &[("target", 1), ("gamma", 1)]),
        );
        tags.insert(pb("b.rs"), tags_of(vec![func("target", 1, "()")], &[]));
        tags.insert(pb("c.rs"), tags_of(vec![func("gamma", 1, "()")], &[]));

        let index = index_tags(&tags);
        let edges = build_ref_edges(&index, &HashSet::new());

        // Exactly two edges, both originating at a.rs, pointing at the definers.
        assert_eq!(edges.len(), 2);
        assert!(edges
            .iter()
            .any(|e| e.from == pb("a.rs") && e.to == pb("b.rs") && e.ident == "target"));
        assert!(edges
            .iter()
            .any(|e| e.from == pb("a.rs") && e.to == pb("c.rs") && e.ident == "gamma"));
        // Definers reference nothing, so no edge originates from them.
        assert!(!edges
            .iter()
            .any(|e| e.from == pb("b.rs") || e.from == pb("c.rs")));
        // √1 · 1.0 = 1.0 weight with no mention.
        assert!(edges.iter().all(|e| (e.weight - 1.0).abs() < 1e-9));
    }

    #[test]
    fn edge_weight_scales_with_reference_count_and_mention() {
        let mut tags = HashMap::new();
        tags.insert(pb("a.rs"), tags_of(vec![], &[("target", 4)]));
        tags.insert(pb("b.rs"), tags_of(vec![func("target", 1, "()")], &[]));

        let index = index_tags(&tags);

        // √4 = 2.0 without mention.
        let plain = build_ref_edges(&index, &HashSet::new());
        assert_eq!(plain.len(), 1);
        assert!((plain[0].weight - 2.0).abs() < 1e-9);

        // Mentioned ⇒ ×10 ⇒ 20.0.
        let mentioned: HashSet<String> = ["target".to_string()].into_iter().collect();
        let boosted = build_ref_edges(&index, &mentioned);
        assert!((boosted[0].weight - 20.0).abs() < 1e-9);
    }

    // ---- PageRank ordering ----

    #[test]
    fn pagerank_ranks_popular_definitions_first() {
        // Both a.rs and c.rs call `target` (in b.rs); nobody calls `lonely`.
        let mut tags = HashMap::new();
        tags.insert(
            pb("a.rs"),
            tags_of(vec![func("acaller", 1, "()")], &[("target", 1)]),
        );
        tags.insert(
            pb("c.rs"),
            tags_of(vec![func("ccaller", 1, "()")], &[("target", 1)]),
        );
        tags.insert(
            pb("b.rs"),
            tags_of(vec![func("target", 1, "()"), func("lonely", 2, "()")], &[]),
        );

        let ranked = rank_tags(&tags, &HashSet::new(), &HashSet::new());
        let target = rank_pos(&ranked, "target").unwrap();
        let lonely = rank_pos(&ranked, "lonely").unwrap();
        assert!(
            target < lonely,
            "referenced `target` should outrank unreferenced `lonely`: {ranked:#?}"
        );
    }

    #[test]
    fn pagerank_is_focus_biased() {
        // a.rs (focus) references `target` in b.rs. c.rs/d.rs are an unrelated
        // island: c references `delta` in d.
        let mut tags = HashMap::new();
        tags.insert(
            pb("a.rs"),
            tags_of(vec![func("alpha", 1, "()")], &[("target", 1)]),
        );
        tags.insert(pb("b.rs"), tags_of(vec![func("target", 1, "()")], &[]));
        tags.insert(
            pb("c.rs"),
            tags_of(vec![func("gamma", 1, "()")], &[("delta", 1)]),
        );
        tags.insert(pb("d.rs"), tags_of(vec![func("delta", 1, "()")], &[]));

        let focus: HashSet<PathBuf> = [pb("a.rs")].into_iter().collect();
        let ranked = rank_tags(&tags, &focus, &HashSet::new());

        // What the focus file points at (`target`) and the focus file's own
        // symbol (`alpha`) must outrank the unrelated island (`delta`,`gamma`).
        let target = rank_pos(&ranked, "target").unwrap();
        let alpha = rank_pos(&ranked, "alpha").unwrap();
        let delta = rank_pos(&ranked, "delta").unwrap();
        let gamma = rank_pos(&ranked, "gamma").unwrap();
        assert!(
            target < delta,
            "focus-referenced `target` < `delta`: {ranked:#?}"
        );
        assert!(
            target < gamma,
            "focus-referenced `target` < `gamma`: {ranked:#?}"
        );
        assert!(alpha < delta, "focus file's `alpha` < `delta`: {ranked:#?}");
    }

    #[test]
    fn mentioning_an_ident_lifts_its_definition() {
        // a.rs references two symmetric idents; mentioning one must lift it.
        let mut tags = HashMap::new();
        tags.insert(
            pb("a.rs"),
            tags_of(
                vec![func("caller", 1, "()")],
                &[("target", 1), ("other", 1)],
            ),
        );
        tags.insert(pb("b.rs"), tags_of(vec![func("target", 1, "()")], &[]));
        tags.insert(pb("d.rs"), tags_of(vec![func("other", 1, "()")], &[]));

        // Baseline: symmetric ⇒ equal ranks.
        let base = rank_tags(&tags, &HashSet::new(), &HashSet::new());
        let bt = base.iter().find(|t| t.def.name == "target").unwrap().rank;
        let bo = base.iter().find(|t| t.def.name == "other").unwrap().rank;
        assert!((bt - bo).abs() < 1e-9, "expected symmetric baseline ranks");

        // Mention `target` ⇒ its definition outranks `other`.
        let mentioned: HashSet<String> = ["target".to_string()].into_iter().collect();
        let ranked = rank_tags(&tags, &HashSet::new(), &mentioned);
        assert!(
            rank_pos(&ranked, "target").unwrap() < rank_pos(&ranked, "other").unwrap(),
            "mentioned `target` should outrank `other`: {ranked:#?}"
        );
    }

    // ---- Token budget ----

    fn many_ranked(n: usize) -> Vec<RankedTag> {
        (0..n)
            .map(|i| RankedTag {
                file: pb("src/lib.rs"),
                def: func(&format!("symbol_{i:03}"), i + 1, "(a: i32, b: i32)->Result"),
                rank: (n - i) as f64,
            })
            .collect()
    }

    #[test]
    fn budget_is_respected_and_never_overflows() {
        let ranked = many_ranked(200);
        let root = Path::new("");
        let cpt = 4;
        for budget in [5usize, 20, 50, 100] {
            let out = fill_budget(&ranked, budget, root, cpt);
            assert!(
                estimate_tokens(&out, cpt) <= budget,
                "budget {budget} overflowed: {} tokens",
                estimate_tokens(&out, cpt)
            );
        }
    }

    #[test]
    fn budget_fills_and_prioritizes_high_rank() {
        let ranked = many_ranked(200);
        let root = Path::new("");
        let cpt = 4;
        let budget = 60;

        let out = fill_budget(&ranked, budget, root, cpt);
        // The highest-ranked symbol is present; a much lower one is dropped.
        assert!(out.contains("symbol_000"), "top symbol must be included");
        assert!(!out.contains("symbol_199"), "lowest symbol must be dropped");

        // Binary search is tight: adding one more tag would exceed the budget.
        let included = out.matches("symbol_").count();
        let one_more = render_tags(&ranked, included + 1, root);
        assert!(
            estimate_tokens(&one_more, cpt) > budget,
            "should have packed the budget: {included} tags fit but +1 stayed under"
        );
    }

    #[test]
    fn budget_binary_search_matches_linear_scan() {
        let ranked = many_ranked(120);
        let root = Path::new("");
        let cpt = 4;
        let budget = 40;

        // Independent linear reference for the largest fitting count.
        let mut expected = 0usize;
        for k in 0..=ranked.len() {
            if estimate_tokens(&render_tags(&ranked, k, root), cpt) <= budget {
                expected = k;
            }
        }
        let out = fill_budget(&ranked, budget, root, cpt);
        assert_eq!(out, render_tags(&ranked, expected, root));
    }

    // ---- mtime cache ----

    #[test]
    fn mtime_cache_hits_when_unchanged_and_recomputes_when_stale() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.rs");
        let b = dir.path().join("b.rs");
        fs::write(&a, "fn one() { two(); }\n").unwrap();
        fs::write(&b, "fn two() {}\n").unwrap();
        let files = vec![a.clone(), b.clone()];

        let mut rm = RepoMap::new();

        // First build parses both files.
        let map1 = rm.map_files(dir.path(), &files, &[], &[]);
        assert_eq!(rm.files_parsed_last(), 2);
        assert!(!map1.is_empty());

        // Second build over the unchanged tree parses nothing (all cache hits)
        // and yields an identical map.
        let map2 = rm.map_files(dir.path(), &files, &[], &[]);
        assert_eq!(rm.files_parsed_last(), 0);
        assert_eq!(map1, map2);

        // Invalidate a.rs by staling its cached mtime; only it re-parses.
        rm.cache.get_mut(&a).unwrap().mtime = Some(SystemTime::UNIX_EPOCH);
        rm.map_files(dir.path(), &files, &[], &[]);
        assert_eq!(rm.files_parsed_last(), 1);
    }

    // ---- End-to-end ----

    #[test]
    fn end_to_end_map_renders_signatures_not_bodies() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("core.rs");
        let b = dir.path().join("util.rs");
        fs::write(
            &a,
            "fn run() { validate(1); }\nfn validate(x: i32) -> bool { x > 0 }\n",
        )
        .unwrap();
        fs::write(&b, "fn helper() {}\n").unwrap();

        let mut rm = RepoMap::new();
        let map = rm.map_files(dir.path(), &[a, b], &[], &[]);

        // Renders relative paths and signatures (not bodies).
        assert!(map.contains("core.rs:"));
        assert!(map.contains("validate(x: i32)->bool"));
        assert!(
            !map.contains("x > 0"),
            "map must show signatures, not bodies"
        );
    }

    // ---- Coding-harness context block ----

    #[test]
    fn coding_context_block_wraps_the_ranked_tags_map() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("core.rs"),
            "fn run() { validate(1); }\nfn validate(x: i32) -> bool { x > 0 }\n",
        )
        .unwrap();

        let block = coding_context_block(dir.path(), 1024).expect("non-empty repo yields a block");
        // Labelled, cache-stable element…
        assert!(block.starts_with("<repo_map>"));
        assert!(block.trim_end().ends_with("</repo_map>"));
        // …carrying the token-budgeted signature map (not bodies).
        assert!(block.contains("core.rs:"));
        assert!(block.contains("validate(x: i32)->bool"));
        assert!(!block.contains("x > 0"), "must show signatures, not bodies");
    }

    #[test]
    fn coding_context_block_is_none_for_an_empty_tree() {
        let dir = tempdir().unwrap();
        // No analyzable source → inject nothing, not an empty block.
        assert!(coding_context_block(dir.path(), 1024).is_none());
    }

    #[test]
    fn coding_context_block_respects_the_token_budget() {
        let dir = tempdir().unwrap();
        // Many symbols, so a tiny budget must drop most of them.
        let mut src = String::new();
        for i in 0..200 {
            src.push_str(&format!(
                "fn symbol_{i:03}(a: i32, b: i32) -> Result<(), ()> {{ Ok(()) }}\n"
            ));
        }
        fs::write(dir.path().join("big.rs"), src).unwrap();

        let tight = coding_context_block(dir.path(), 40).expect("has source");
        let generous = coding_context_block(dir.path(), 100_000).expect("has source");

        // A tighter budget surfaces strictly fewer symbols than a generous one —
        // proving the budget constrains the injected map.
        let tight_syms = tight.matches("symbol_").count();
        let generous_syms = generous.matches("symbol_").count();
        assert!(
            tight_syms >= 1,
            "even a tight budget surfaces at least one symbol"
        );
        assert!(
            tight_syms < generous_syms,
            "tight budget ({tight_syms} symbols) should surface fewer than generous ({generous_syms})"
        );
    }
}
