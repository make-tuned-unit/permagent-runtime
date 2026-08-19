//! **Dependency-claim guard.** A comment that says a dependency's capability
//! is unavailable is a *load-bearing claim*, because it is the reason no wire
//! exists. This scans shipped source for claims of that shape and fails when
//! the named capability is in fact reachable at the current pin.
//!
//! ## Why it exists
//!
//! `recognition_sink.rs` carried "Spectral's `Brain::recognize()` … [is] not
//! in the pinned Spectral rev yet" until 2026-08-19, and it had been false
//! since before that pin was chosen: `recognize()` landed on Spectral's facade
//! on 2026-07-12 (`f1692f0`) and the stream tracker on 2026-07-03 (`095f234`),
//! while this repo's pin `c2c8381` is dated 2026-07-31. A recognition
//! subsystem measuring 0.9946 AUC therefore never answered a question in
//! production, and no test could catch it: **a seam nobody exercises cannot
//! fail.** That was the fourth "the wire does not exist" defect found in one
//! week.
//!
//! So this guard does not test behaviour. It tests the *justification* for
//! there being no behaviour, against the artefact that justification is about
//! — the vendored source of the pinned dependency. Same shape as the
//! phantom-tool guard in `agents::self_knowledge` and the identity-name guard
//! in `config::identity_name_guard`: scan the real artefact, fail loudly, make
//! every exemption a written decision.
//!
//! ## What it scans, and how
//!
//! Every `.rs` file under `crates/*/src`, comments only (`//`, `///`, `//!`,
//! `/* */`). Comment text is normalized first — markers and *newlines* collapse
//! to single spaces — because the defect that motivated this guard was written
//! across two lines:
//!
//! ```text
//! //! and the session-level stream tracker (edge-triggered routine locks) are not
//! //! in the pinned Spectral rev yet, so nothing here computes a verdict
//! ```
//!
//! A line-oriented scan for "not in the pinned" would have missed the exact
//! comment it exists to catch. Non-comment bytes become a barrier character
//! that no pattern can cross, so a match never spans code.
//!
//! ## The two rules
//!
//! A comment only counts as a claim if it also names a dependency (see
//! [`DEPENDENCY_MARKERS`]) — "flags when every change lands in a test file" is
//! about our own behaviour and must stay quiet, or the allowlist becomes a
//! dumping ground and stops meaning anything. Then:
//!
//! 1. Every matched claim must appear in [`CLAIMS`] with a written reason.
//!    A new one fails: state what is unavailable and why, or delete the claim.
//! 2. Every [`CLAIMS`] entry whose probe is [`Probe::SpectralSymbolAbsent`] is
//!    checked against the vendored source of the pin recorded in `Cargo.lock`.
//!    If the symbol is present, the claim is stale and the guard fails naming
//!    it. Claims that are not about a dependency's API surface must say so
//!    explicitly via [`Probe::NotADependencySymbol`] — an exemption, written
//!    down, with a reason.
//!
//! Dead entries fail too: an allowlist that outlives the comment it excuses is
//! the same rot in a different place.
//!
//! ## Cost
//!
//! No network, no build of the dependency, no runtime. It reads ~1.5k source
//! files and the vendored dependency's own `.rs` files once, from disk.

// string_slice: every byte index below is either a `find()` result (already a
// char boundary) or a window edge passed through `floor_boundary` /
// `ceil_boundary` first, so no slice can split a UTF-8 sequence. Same argument
// as `identity_name_guard`.
#![allow(clippy::string_slice)]

use std::path::{Path, PathBuf};

/// A claim shape. `needles` must all appear, in order, within
/// [`MAX_SPAN`] normalized bytes of each other. Matching is case-insensitive.
struct Pattern {
    name: &'static str,
    needles: &'static [&'static str],
}

/// How far apart an ordered multi-needle pattern's parts may sit. Roughly one
/// sentence: wide enough for "once the pack registry exists", narrow enough
/// that an unrelated "when" and "lands" three paragraphs apart do not pair up.
const MAX_SPAN: usize = 90;

/// The claim shapes. Each is a way of saying "we cannot do this because the
/// dependency will not let us" — the sentence that stops work.
const PATTERNS: &[Pattern] = &[
    // The exact shape of the recognition defect.
    Pattern {
        name: "not in the pinned <dep>",
        needles: &["not in the pinned"],
    },
    Pattern {
        name: "not yet in the pinned <dep>",
        needles: &["not yet in the pinned"],
    },
    // "when the dep lands", "when that pin lands", "when Spectral's X lands",
    // "until it lands", "lands upstream".
    Pattern {
        name: "when/once <X> lands",
        needles: &["when", "lands"],
    },
    Pattern {
        name: "once <X> lands",
        needles: &["once", "lands"],
    },
    Pattern {
        name: "until <X> lands",
        needles: &["until", "lands"],
    },
    Pattern {
        name: "<X> lands upstream",
        needles: &["lands upstream"],
    },
    // "once the pack registry exists", "once the API exists".
    Pattern {
        name: "once <X> exists",
        needles: &["once", "exists"],
    },
    // "graph triples pending Spectral API", "verdict pending Spectral recognize()".
    Pattern {
        name: "pending <dep>",
        needles: &["pending spectral"],
    },
    // "when spectral grows the API".
    Pattern {
        name: "<dep> grows the API",
        needles: &["grows the api"],
    },
    // "the dep is not upgraded until the branch merges".
    Pattern {
        name: "<dep> not upgraded until",
        needles: &["not upgraded until"],
    },
];

/// A claim only counts if the comment is talking about a DEPENDENCY. Without
/// this, "flags when every change lands in a test file" and "poll until it
/// lands" read as unavailability claims; with it, seven such comments in this
/// tree stay quiet and every real one is still caught, because a claim about
/// an external API cannot avoid naming the thing it is about.
const DEPENDENCY_MARKERS: &[&str] = &["spectral", "the dep", "pin", "upstream", "the crate"];

/// How far from the matched phrase a [`DEPENDENCY_MARKERS`] mention may sit —
/// wide enough to reach the subject of the sentence before, narrow enough not
/// to borrow one from an unrelated paragraph.
const MARKER_SPAN: usize = 120;

/// How a claim's truth is checked against the pinned dependency.
enum Probe {
    /// The claim says a Spectral symbol is unavailable. Every needle is
    /// searched for in the vendored source of the pin in `Cargo.lock`; ANY hit
    /// means the claim is stale and the guard fails.
    SpectralSymbolAbsent(&'static [&'static str]),
    /// The claim is not about an external dependency's API surface, so there
    /// is no pinned artefact to check it against. An exemption: the string is
    /// the written reason.
    NotADependencySymbol(&'static str),
}

/// One written-down claim. `quote` is a distinctive fragment of the claim as
/// it reads in the *normalized* comment text (lowercase, single-spaced), used
/// to tie a scan hit to its entry.
struct Claim {
    file: &'static str,
    quote: &'static str,
    reason: &'static str,
    probe: Probe,
}

/// **The allowlist.** Every claim of unavailability that shipped source is
/// still allowed to make. Adding an entry is a decision: say what is missing,
/// and give the guard a symbol it can check, or say why there is none.
const CLAIMS: &[Claim] = &[
    Claim {
        file: "crates/goose/src/brain_handle.rs",
        quote: "graph triples pending spectral api",
        reason: "Spectral exposes no triple/entity delete API, so a scope sweep cannot remove \
                 graph facts and says so rather than overclaiming erasure. The surrounding \
                 text still names the older pin fb1038db; the claim itself is what this probe \
                 re-checks at whatever rev Cargo.lock currently holds.",
        probe: Probe::SpectralSymbolAbsent(&["fn delete_triple", "fn delete_entity"]),
    },
    Claim {
        file: "crates/goose/src/project_graph.rs",
        quote: "pin bump once it lands in spectral main",
        reason: "Same missing delete surface, stated as the upstream option: adding \
                 delete_triple to Spectral is cross-repo and would need a deliberate pin bump.",
        probe: Probe::SpectralSymbolAbsent(&["fn delete_triple"]),
    },
    Claim {
        file: "crates/goose/src/project_graph.rs",
        quote: "when spectral grows the api",
        reason: "Same missing delete surface, seen from the caller: delete_works_on_triples is \
                 a direct-SQL stopgap against graph.sqlite until store.delete_triple exists.",
        probe: Probe::SpectralSymbolAbsent(&["fn delete_triple"]),
    },
    Claim {
        file: "crates/goose/src/brain_handle.rs",
        quote: "when that pin lands",
        reason: "assert_typed_from(memory_id, …) — triple provenance threaded through the \
                 write — is genuinely absent at the pin; assert_typed carries no source \
                 memory, and an unsourced triple is the accepted interim state.",
        probe: Probe::SpectralSymbolAbsent(&["assert_typed_from"]),
    },
    Claim {
        file: "crates/goose/src/brain_handle.rs",
        quote: "deferred delivery write lands upstream",
        reason: "Brain::turn commits its delivery record synchronously. That is a performance \
                 property of code that IS present, not a missing symbol, so there is nothing \
                 to grep for; it is held instead by Spectral's own preregistered latency gate \
                 (recall-only p95 +87-100% against a +5% kill line), which is why turn is \
                 sampled rather than default.",
        probe: Probe::NotADependencySymbol(
            "a latency property of an API that is present, not an absent API",
        ),
    },
    Claim {
        file: "crates/goose/src/turn_sampling.rs",
        quote: "delivery write lands upstream",
        reason: "The sampling-rate half of the same statement; same reasoning, same gate.",
        probe: Probe::NotADependencySymbol(
            "a latency property of an API that is present, not an absent API",
        ),
    },
];

const SKIP_DIRS: &[&str] = &["node_modules", "dist", "target", "vendor", ".git"];

/// Barrier byte: stands in for every non-comment region so a pattern can never
/// match across code. No pattern contains it.
const BARRIER: char = '\u{0}';

// ── Normalization ────────────────────────────────────────────────────────

/// Lowercased comment text with markers and whitespace collapsed to single
/// spaces, non-comment bytes replaced by a single [`BARRIER`], plus a parallel
/// map from normalized index to source byte offset.
struct Normalized {
    text: String,
    offsets: Vec<usize>,
}

/// Byte ranges of `src` that ARE comments — string-aware, so a `//` inside a
/// string literal (a URL) does not open a phantom comment and a char literal
/// `'"'` does not open a phantom string. Deliberately the same lexer shape as
/// `identity_name_guard::code_regions`, inverted.
fn comment_regions(src: &str) -> Vec<(usize, usize)> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if b[i..].starts_with(b"//") {
            let end = src[i..].find('\n').map(|j| i + j).unwrap_or(n);
            out.push((i, end));
            i = end;
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let end = src[i + 2..].find("*/").map(|j| i + 2 + j + 2).unwrap_or(n);
            out.push((i, end));
            i = end;
            continue;
        }
        if b[i] == b'r' && i + 1 < n && (b[i + 1] == b'"' || b[i + 1] == b'#') {
            let mut k = i + 1;
            let mut hashes = 0;
            while k < n && b[k] == b'#' {
                hashes += 1;
                k += 1;
            }
            if k < n && b[k] == b'"' {
                let close = format!("\"{}", "#".repeat(hashes));
                match src[k + 1..].find(&close) {
                    Some(j) => {
                        i = k + 1 + j + close.len();
                        continue;
                    }
                    None => break,
                }
            }
        }
        if b[i] == b'\'' {
            if i + 1 < n && b[i + 1] == b'\\' {
                i = src[i + 2..].find('\'').map(|j| i + 2 + j + 1).unwrap_or(n);
                continue;
            }
            if i + 2 < n && b[i + 2] == b'\'' {
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }
        if b[i] == b'"' {
            let mut j = i + 1;
            while j < n {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'"' {
                    break;
                }
                j += 1;
            }
            i = (j + 1).min(n);
            continue;
        }
        i += 1;
    }
    out
}

/// True for the comment-marker characters that carry no meaning: they become
/// spaces so `//!` continuation lines join into one sentence.
fn is_marker(c: char) -> bool {
    matches!(c, '/' | '!' | '*')
}

fn normalize(src: &str) -> Normalized {
    let mut text = String::with_capacity(src.len() / 2);
    let mut offsets = Vec::with_capacity(src.len() / 2);
    let mut cursor = 0usize;
    let push = |text: &mut String, offsets: &mut Vec<usize>, c: char, at: usize| {
        // Collapse runs of spaces and runs of barriers alike.
        if (c == ' ' || c == BARRIER) && text.ends_with(c) {
            return;
        }
        text.push(c);
        offsets.push(at);
    };
    for (start, end) in comment_regions(src) {
        if start > cursor {
            // Whitespace between two comment regions is the line break
            // BETWEEN `//!` lines — the thing that hid the defect this guard
            // exists for. It joins; anything else is code and blocks.
            let gap = if src[cursor..start].trim().is_empty() {
                ' '
            } else {
                BARRIER
            };
            push(&mut text, &mut offsets, gap, cursor);
        }
        // Leading markers of the comment opener become a space.
        let mut at = start;
        for c in src[start..end].chars() {
            let mapped = if c.is_whitespace() || is_marker(c) {
                ' '
            } else {
                c.to_ascii_lowercase()
            };
            push(&mut text, &mut offsets, mapped, at);
            at += c.len_utf8();
        }
        push(&mut text, &mut offsets, ' ', end);
        cursor = end;
    }
    if cursor < src.len() {
        push(&mut text, &mut offsets, BARRIER, cursor);
    }
    Normalized { text, offsets }
}

/// Nearest char boundary at or below `at` — window edges are arbitrary byte
/// offsets and comments contain em dashes.
fn floor_boundary(s: &str, mut at: usize) -> usize {
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Nearest char boundary at or above `at`.
fn ceil_boundary(s: &str, mut at: usize) -> usize {
    while at < s.len() && !s.is_char_boundary(at) {
        at += 1;
    }
    at
}

fn line_of(src: &str, offset: usize) -> usize {
    src[..offset.min(src.len())].matches('\n').count() + 1
}

// ── Scanning ─────────────────────────────────────────────────────────────

/// One claim found in source: which pattern, which line, and the surrounding
/// normalized text (for tying it to a [`Claim`] and for the failure message).
#[derive(Debug, PartialEq, Eq)]
struct Hit {
    pattern: &'static str,
    line: usize,
    window: String,
}

/// Find `needles` in order within [`MAX_SPAN`] of each other, never crossing a
/// [`BARRIER`]. Returns the normalized index of the first needle.
fn ordered_match(hay: &str, needles: &[&str], from: usize) -> Option<(usize, usize)> {
    let mut cursor = ceil_boundary(hay, from);
    loop {
        let first = hay[cursor..].find(needles[0])? + cursor;
        cursor = ceil_boundary(hay, first + 1);
        let mut at = first + needles[0].len();
        let mut matched = true;
        for needle in &needles[1..] {
            // MAX_SPAN is a byte count, and comments are full of em dashes.
            let limit = ceil_boundary(hay, (first + MAX_SPAN).min(hay.len()));
            // A later occurrence of needles[0] may still pair up, so a miss
            // here retries rather than giving up on the whole file — the bug
            // that first hid `assert_typed_from`'s claim from this guard.
            let found = if at <= limit {
                hay[at..limit].find(needle).map(|j| j + at)
            } else {
                None
            };
            match found {
                Some(found) => at = found + needle.len(),
                None => {
                    matched = false;
                    break;
                }
            }
        }
        if matched && !hay[first..at].contains(BARRIER) {
            return Some((first, at));
        }
    }
}

fn scan_source(src: &str) -> Vec<Hit> {
    let norm = normalize(src);
    let mut hits: Vec<Hit> = Vec::new();
    for pattern in PATTERNS {
        let mut from = 0;
        while let Some((start, end)) = ordered_match(&norm.text, pattern.needles, from) {
            from = start + 1;
            let m_lo = start.saturating_sub(MARKER_SPAN);
            let m_hi = (end + MARKER_SPAN).min(norm.text.len());
            let context =
                &norm.text[floor_boundary(&norm.text, m_lo)..ceil_boundary(&norm.text, m_hi)];
            if !DEPENDENCY_MARKERS.iter().any(|m| context.contains(m)) {
                continue;
            }
            let src_offset = norm.offsets.get(start).copied().unwrap_or(0);
            let w_lo = floor_boundary(&norm.text, start.saturating_sub(140));
            let w_hi = ceil_boundary(&norm.text, (end + 140).min(norm.text.len()));
            hits.push(Hit {
                pattern: pattern.name,
                line: line_of(src, src_offset),
                window: norm.text[w_lo..w_hi].replace(BARRIER, " | "),
            });
        }
    }
    hits.sort_by(|a, b| (a.line, a.pattern).cmp(&(b.line, b.pattern)));
    hits.dedup_by(|a, b| a.line == b.line);
    hits
}

// ── The pinned dependency's real source ──────────────────────────────────

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The Spectral rev this workspace is locked to, from `Cargo.lock`.
fn locked_spectral_rev(root: &Path) -> String {
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock is readable");
    for block in lock.split("[[package]]") {
        if !block.contains("\nname = \"spectral\"\n") {
            continue;
        }
        let line = block
            .lines()
            .find(|l| l.starts_with("source = ") && l.contains("spectral"))
            .expect("spectral package has a source line");
        let sha = line
            .rsplit_once('#')
            .expect("git source carries a #<sha>")
            .1
            .trim_end_matches('"');
        return sha.to_string();
    }
    panic!("Cargo.lock has no `spectral` package — has the dependency been renamed?");
}

/// Where cargo actually unpacked that rev. Checked, not assumed: this guard is
/// worthless if it silently skips.
fn pinned_spectral_dir(root: &Path, sha: &str) -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("PERMAGENT_SPECTRAL_SRC") {
        let path = PathBuf::from(explicit);
        return if path.join("crates/spectral/src/lib.rs").is_file() {
            Ok(path)
        } else {
            Err(format!(
                "PERMAGENT_SPECTRAL_SRC={} has no crates/spectral/src/lib.rs",
                path.display()
            ))
        };
    }
    let vendored = root.join("vendor/spectral");
    if vendored.join("crates/spectral/src/lib.rs").is_file() {
        return Ok(vendored);
    }
    let cargo_home = std::env::var("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cargo")
        });
    let checkouts = cargo_home.join("git/checkouts");
    let Ok(entries) = std::fs::read_dir(&checkouts) else {
        return Err(format!("no git checkouts under {}", checkouts.display()));
    };
    for entry in entries.flatten() {
        let Ok(revs) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        for rev in revs.flatten() {
            let name = rev.file_name().to_string_lossy().to_string();
            if name.len() >= 7 && sha.starts_with(&name) {
                let path = rev.path();
                if path.join("crates/spectral/src/lib.rs").is_file() {
                    return Ok(path);
                }
            }
        }
    }
    Err(format!(
        "no checkout of spectral rev {sha} under {}",
        checkouts.display()
    ))
}

/// Every `.rs` byte of the pinned dependency, concatenated with a `path:line`
/// index so a hit can be reported precisely.
fn pinned_spectral_sources(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    walk_rs(&dir.join("crates"), &mut files);
    files
        .into_iter()
        .filter_map(|path| {
            let rel = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            std::fs::read_to_string(&path).ok().map(|src| (rel, src))
        })
        .collect()
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if SKIP_DIRS.iter().any(|s| entry.file_name() == **s) {
                continue;
            }
            walk_rs(&path, out);
        } else if meta.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Non-test source of the dependency only: a symbol that exists solely in the
/// dependency's own tests is not reachable by us, and a doc comment naming a
/// symbol is not the symbol. Test modules are cut at the same `#[cfg(test)]
/// mod` boundary the identity-name guard uses.
fn shippable(src: &str) -> &str {
    let mut offset = 0;
    let mut lines = src.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let opens_test_mod = lines
            .peek()
            .is_some_and(|next| next.trim_start().starts_with("mod "));
        if line.trim() == "#[cfg(test)]" && opens_test_mod {
            return &src[..offset];
        }
        offset += line.len();
    }
    src
}

/// Where `needle` appears in the pinned dependency's shipped source, if it
/// does. Hits inside the dependency's OWN comments do not count: a Spectral
/// TODO naming a symbol is not the symbol, and treating it as one would make
/// every claim look stale.
fn find_in_pinned(sources: &[(String, String)], needle: &str) -> Option<String> {
    for (rel, src) in sources {
        let body = shippable(src);
        let comments = comment_regions(body);
        for (idx, _) in body.match_indices(needle) {
            if comments.iter().any(|(a, z)| idx >= *a && idx < *z) {
                continue;
            }
            return Some(format!("{rel}:{}", line_of(body, idx)));
        }
    }
    None
}

// ── The guard ────────────────────────────────────────────────────────────

fn shipped_rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("crates/ exists") {
        let src = entry.expect("readable crate dir").path().join("src");
        if src.is_dir() {
            walk_rs(&src, &mut files);
        }
    }
    files
}

/// **The guard.** No shipped comment says a dependency capability is
/// unavailable unless it is written down here AND still true at the pin.
#[test]
fn no_shipped_comment_claims_a_reachable_dependency_is_missing() {
    let root = repo_root();
    let files = shipped_rust_files(&root);
    assert!(
        files.len() > 100,
        "scan found only {} rust files under {} — the walk is broken, not the code",
        files.len(),
        root.display()
    );

    let sha = locked_spectral_rev(&root);
    let dir = pinned_spectral_dir(&root, &sha).unwrap_or_else(|why| {
        panic!(
            "cannot locate the pinned Spectral source, so this guard cannot verify anything \
             ({why}).\nA silent skip here is exactly the failure this guard exists to \
             prevent, so it fails instead. Fetch the dependency (any `cargo build` does), \
             or point PERMAGENT_SPECTRAL_SRC at a checkout of rev {sha}."
        )
    });
    let pinned = pinned_spectral_sources(&dir);
    assert!(
        pinned.len() > 20,
        "only {} rust files under {} — that is not the Spectral tree",
        pinned.len(),
        dir.display()
    );

    let mut undeclared = Vec::new();
    let mut matched = vec![false; CLAIMS.len()];

    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("under repo root")
            .to_string_lossy()
            .replace('\\', "/");
        // This file quotes every claim shape it hunts for.
        if rel == "crates/goose/src/dependency_claim_guard.rs" {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        for hit in scan_source(&src) {
            match CLAIMS
                .iter()
                .position(|c| c.file == rel && hit.window.contains(c.quote))
            {
                Some(idx) => matched[idx] = true,
                None => undeclared.push(format!(
                    "{rel}:{} [{}]\n      …{}…",
                    hit.line, hit.pattern, hit.window
                )),
            }
        }
    }

    assert!(
        undeclared.is_empty(),
        "shipped source claims a dependency capability is unavailable, and the claim is not \
         written down.\n\n\
         A comment of this shape is why a wire does not exist, so it has to be checkable. \
         Either:\n  \
         (a) the capability IS reachable at the pinned rev — delete the comment and wire the \
         call (this is what happened to Spectral's recognize(): the claim outlived the truth \
         by nineteen days at the pin, and a 0.9946-AUC subsystem never ran); or\n  \
         (b) it is genuinely absent — add a `Claim` to CLAIMS in \
         crates/goose/src/dependency_claim_guard.rs naming the symbol, so the day it lands \
         this test tells you.\n\n{}",
        undeclared.join("\n")
    );

    let mut stale = Vec::new();
    for (idx, claim) in CLAIMS.iter().enumerate() {
        let exemption = match claim.probe {
            Probe::NotADependencySymbol(why) => format!(" [unverifiable: {why}]"),
            Probe::SpectralSymbolAbsent(_) => String::new(),
        };
        assert!(
            matched[idx],
            "CLAIMS entry {}:\"{}\"{exemption} matches nothing in shipped source. The comment \
             it excuses is gone — delete the entry; a stale allowlist is the same rot in a \
             different place.",
            claim.file, claim.quote
        );
        let Probe::SpectralSymbolAbsent(needles) = claim.probe else {
            continue;
        };
        for needle in needles {
            if let Some(found) = find_in_pinned(&pinned, needle) {
                stale.push(format!(
                    "  {}: \"{}\"\n      claims `{}` is unavailable — it is PRESENT at pinned \
                     rev {} ({})\n      written reason: {}",
                    claim.file,
                    claim.quote,
                    needle,
                    &sha[..7],
                    found,
                    claim.reason
                ));
            }
        }
    }

    assert!(
        stale.is_empty(),
        "a written unavailability claim is no longer true at the pinned dependency rev.\n\n\
         The capability is reachable NOW, with no pin bump: wire the call, delete the \
         comment, and remove the CLAIMS entry. Do not leave the comment standing — it is \
         what the next reader will believe.\n\n{}",
        stale.join("\n\n")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-fix `recognition_sink.rs` docstring, verbatim, including the
    /// line break that a line-oriented scanner would have hidden behind. This
    /// is the regression: the guard MUST flag this text.
    const PRE_FIX_RECOGNITION_DOCSTRING: &str = "\
//! RecognitionSink — Permagent's consumer boundary for Spectral's incoming
//! recognition subsystem.
//!
//! This module is the SEAM ONLY. Spectral's `Brain::recognize()` (query mode)
//! and the session-level stream tracker (edge-triggered routine locks) are not
//! in the pinned Spectral rev yet, so nothing here computes a verdict — the
//! call sites are wired and a debug-log sink is installed by default, so the
//! day the dep lands the only work is conversion + forwarding.
pub fn seam() {}
";

    #[test]
    fn scanner_catches_the_recognition_docstring_that_caused_this_guard() {
        let hits = scan_source(PRE_FIX_RECOGNITION_DOCSTRING);
        assert!(
            hits.iter().any(|h| h.pattern == "not in the pinned <dep>"),
            "the claim that cost nineteen days was split across two lines and must still \
             be caught: {hits:?}"
        );
        // …and the capability it denied is reachable at the pin, which is the
        // half that turns a hit into a failure.
        let root = repo_root();
        let sha = locked_spectral_rev(&root);
        let dir = pinned_spectral_dir(&root, &sha).expect("pinned spectral source located");
        let pinned = pinned_spectral_sources(&dir);
        assert!(
            find_in_pinned(&pinned, "pub fn recognize").is_some(),
            "Brain::recognize() must be present at pinned rev {sha} — it landed 2026-07-12, \
             nineteen days before the pin was cut"
        );
    }

    #[test]
    fn scanner_joins_wrapped_comments_and_never_crosses_code() {
        // Wrapped across three lines and two markers — still one sentence.
        let wrapped = "/// the tracker is\n/// not in the\n/// pinned rev\nfn f() {}\n";
        assert_eq!(scan_source(wrapped).len(), 1);

        // "not in the" and "pinned" on either side of real code must NOT pair.
        let split = "// it is not in the\nfn f() {}\n// pinned rev\n";
        assert!(scan_source(split).is_empty(), "matched across code");

        // A string literal containing the phrase is code, not a claim.
        let literal = "const S: &str = \"not in the pinned rev\";\n";
        assert!(scan_source(literal).is_empty(), "matched a string literal");
    }

    #[test]
    fn ordered_patterns_respect_the_span_and_report_the_right_line() {
        let near = "// once the spectral pack registry exists\n";
        let hits = scan_source(near);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 1);

        // Same two words, far apart: not a claim.
        let far = format!(
            "// spectral: once {}\n// the surface exists\n",
            "x ".repeat(60)
        );
        assert!(
            scan_source(&far).is_empty(),
            "paired across {MAX_SPAN} bytes"
        );
    }

    /// A first needle that does not pair must not abandon the file: the
    /// `assert_typed_from` claim sits behind several earlier "when"s, and an
    /// early-return scanner reported that file clean.
    #[test]
    fn an_unpaired_first_needle_does_not_end_the_search() {
        let src = "// when we get to it. when the sun shines. when the spectral pin lands.\n";
        assert_eq!(scan_source(src).len(), 1, "{:?}", scan_source(src));
    }

    /// The guard is about DEPENDENCIES. A comment about our own unfinished
    /// work reads identically and must stay quiet, or the allowlist becomes a
    /// dumping ground and stops meaning anything.
    #[test]
    fn a_claim_that_names_no_dependency_is_not_a_dependency_claim() {
        assert!(
            scan_source("// flags when every change lands in a test file\n").is_empty(),
            "our own behaviour, not a dependency"
        );
        assert!(
            scan_source("// the drop handler runs on a task; poll until it lands\n").is_empty(),
            "our own task, not a dependency"
        );
        // Name the dependency and the very same shape is a claim again.
        assert_eq!(
            scan_source("// poll until the spectral fix lands\n").len(),
            1
        );
    }

    /// A doc comment in the dependency that merely *mentions* a symbol must
    /// not read as the symbol existing — otherwise every claim looks stale.
    #[test]
    fn pinned_lookup_ignores_the_dependency_own_comments() {
        let sources = vec![
            (
                "crates/x/src/lib.rs".to_string(),
                "/// one day we add fn delete_triple here\npub fn other() {}\n".to_string(),
            ),
            (
                "crates/y/src/lib.rs".to_string(),
                "pub fn keep_triple() {}\n".to_string(),
            ),
        ];
        assert!(find_in_pinned(&sources, "fn delete_triple").is_none());
        assert_eq!(
            find_in_pinned(&sources, "fn keep_triple").as_deref(),
            Some("crates/y/src/lib.rs:1")
        );
    }
}
