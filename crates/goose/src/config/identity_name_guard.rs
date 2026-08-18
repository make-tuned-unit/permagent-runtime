//! **Identity-name guard (#986).** The primary agent's name is per-user
//! (`PrimaryPersona::display_name()`), and the developer's own name is not a
//! fact about the product. Neither may be a literal in shipped source. Two
//! local assertions already pinned this for the persona block
//! (`agent_identity.rs`) and the verification digest (`digest.rs`); this is
//! the repo-wide version, in the same spirit as the phantom-tool guard in
//! `agents::self_knowledge` — scan the real artefact, fail loudly, and make
//! every exemption a written decision.
//!
//! What it scans: every `.rs`, `.ts` and `.tsx` file under `crates/*/src` and
//! `ui/command-center/src`, resolved from `CARGO_MANIFEST_DIR` so it runs
//! from any cwd. `node_modules`, `dist`, `target`, `vendor` and `.git` are
//! never descended into. Insta `.snap` files are not scanned by construction
//! (extension filter): the prompt snapshots are *derived* from live rendering
//! — "You are Aria." there is the default persona interpolated, and any
//! literal leak in a snapshot necessarily originates in a scanned source
//! string, so scanning them would double-report the same fault.
//!
//! Two rules, deliberately asymmetric:
//! - The **legacy default agent name** fails when it appears in *code* — string
//!   literals, JSX text, identifiers — outside comments and outside test code
//!   (Rust: everything after the file's `#[cfg(test)] mod …` line; TS: files
//!   named `*.test.*` / `*.spec.*` / under `__tests__/`). Comments may still
//!   say "Henry" as developer shorthand for the orchestrator role, and tests
//!   may configure a persona of any name to prove the plumbing.
//! - The **developer's name** fails *anywhere* — comments and tests included.
//!   Rulings are cited by date ("ruling 2026-07-03"), fixtures use neutral
//!   names, prompts say "the user".
//!
//! Matching is case-sensitive and whole-word (`[A-Za-z0-9_]` boundaries), so
//! the stable id keys `henry` / `'jesse'` / `ACTOR_JESSE` and identifiers such
//! as `HenryHUD` are never hits — ids and keys are load-bearing and stay.
//!
//! The lexer is a small string-aware comment stripper, not a parser. Known
//! line-local blind spots: a `//` or quote inside a regex literal, or an
//! apostrophe in JSX text, hides the rest of *that line* (only). It cannot
//! produce a false positive.

use std::path::{Path, PathBuf};

/// The out-of-box persona name this repo shipped under before it became
/// per-user config. Not the current default ("Aria" — see
/// `PrimaryPersona::default()`); the historical one that leaked into copy.
const LEGACY_AGENT_NAME: &str = "Henry";

/// The developer's first name. Never a universal fact about the product.
const DEVELOPER_NAME: &str = "Jesse";

/// Files exempt from the scan, each with its reason. Every entry is a
/// decision, not a convenience — add one only when the literal is the point.
///
/// - `crates/goose/src/config/identity_name_guard.rs`: this file. It has to
///   name the strings it hunts.
const ALLOWLIST: &[&str] = &["crates/goose/src/config/identity_name_guard.rs"];

const SKIP_DIRS: &[&str] = &["node_modules", "dist", "target", "vendor", ".git"];

#[derive(Debug, PartialEq, Eq)]
enum Lang {
    Rust,
    TypeScript,
}

fn lang_of(path: &Path) -> Option<Lang> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(Lang::Rust),
        Some("ts") | Some("tsx") => Some(Lang::TypeScript),
        _ => None,
    }
}

fn is_ts_test_file(rel: &str) -> bool {
    rel.contains("/__tests__/")
        || rel.ends_with(".test.ts")
        || rel.ends_with(".test.tsx")
        || rel.ends_with(".spec.ts")
        || rel.ends_with(".spec.tsx")
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Byte offsets of every whole-word, case-sensitive occurrence of `needle`.
fn word_hits(hay: &str, needle: &str) -> Vec<usize> {
    let bytes = hay.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_word_char(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word_char(bytes[end]);
        if before_ok && after_ok {
            out.push(start);
        }
        from = end;
    }
    out
}

/// Where the trailing test module of a Rust file starts — the first
/// `#[cfg(test)]` line that is immediately followed by a `mod` line. Only that
/// shape counts (a `#[cfg(test)] use …` near the top must not blank the file).
fn rust_test_cutoff(src: &str) -> usize {
    let mut offset = 0;
    let mut lines = src.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let opens_test_mod = lines.peek().is_some_and(|next| {
            let t = next.trim_start();
            t.starts_with("mod ") || t.starts_with("pub mod ") || t.starts_with("pub(crate) mod ")
        });
        if line.trim() == "#[cfg(test)]" && opens_test_mod {
            return offset;
        }
        offset += line.len();
    }
    src.len()
}

/// Byte ranges of `src` that are NOT comments — string-aware, so a `//` inside
/// a string literal (URLs) does not swallow the rest of the line, and a Rust
/// char literal `'"'` does not open a phantom string.
fn code_regions(src: &str, lang: Lang) -> Vec<(usize, usize)> {
    let b = src.as_bytes();
    let n = b.len();
    let ts = lang == Lang::TypeScript;
    let mut out = Vec::new();
    let mut seg = 0;
    let mut i = 0;
    while i < n {
        if b[i..].starts_with(b"//") {
            out.push((seg, i));
            i = src[i..].find('\n').map(|j| i + j).unwrap_or(n);
            seg = i;
            continue;
        }
        if b[i..].starts_with(b"/*") {
            out.push((seg, i));
            i = src[i + 2..].find("*/").map(|j| i + 2 + j + 2).unwrap_or(n);
            seg = i;
            continue;
        }
        if !ts && b[i] == b'r' && i + 1 < n && (b[i + 1] == b'"' || b[i + 1] == b'#') {
            // Raw string r"…" / r#"…"# — skip to its matching close.
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
        if !ts && b[i] == b'\'' {
            // Char literal ('x', '\n', '"') vs lifetime ('a): only the literal
            // forms carry a closing quote within the next few bytes.
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
        let c = b[i];
        if c == b'"' || (ts && (c == b'\'' || c == b'`')) {
            let mut j = i + 1;
            while j < n {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == c {
                    break;
                }
                // TS quote strings never span lines; an unterminated one is a
                // regex or the like — stop at the newline.
                if ts && c != b'`' && b[j] == b'\n' {
                    break;
                }
                j += 1;
            }
            i = (j + 1).min(n);
            continue;
        }
        i += 1;
    }
    out.push((seg, n));
    out
}

fn line_of(src: &str, offset: usize) -> usize {
    src[..offset].matches('\n').count() + 1
}

/// Scan one file's contents. Returns `(which name, line)` per hit.
fn scan_source(rel: &str, src: &str) -> Vec<(&'static str, usize)> {
    let Some(lang) = lang_of(Path::new(rel)) else {
        return Vec::new();
    };
    let mut hits = Vec::new();

    // Developer name: anywhere at all.
    for off in word_hits(src, DEVELOPER_NAME) {
        hits.push(("developer name", line_of(src, off)));
    }

    // Legacy agent name: code (not comments), not test code.
    let scanned: &str = match lang {
        Lang::TypeScript if is_ts_test_file(rel) => "",
        Lang::TypeScript => src,
        Lang::Rust => &src[..rust_test_cutoff(src)],
    };
    for (a, z) in code_regions(scanned, lang) {
        for off in word_hits(&scanned[a..z], LEGACY_AGENT_NAME) {
            hits.push(("agent name", line_of(src, a + off)));
        }
    }
    hits.sort();
    hits
}

fn repo_root() -> PathBuf {
    // crates/goose → repo root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

fn shipped_source_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("crates/ exists") {
        let src = entry.expect("readable crate dir").path().join("src");
        if src.is_dir() {
            roots.push(src);
        }
    }
    roots.push(root.join("ui/command-center/src"));
    roots
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        // `symlink_metadata` so a symlinked node_modules is not followed.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            let name = entry.file_name();
            if SKIP_DIRS.iter().any(|s| name == *s) {
                continue;
            }
            walk(&path, out);
        } else if meta.is_file() && lang_of(&path).is_some() {
            out.push(path);
        }
    }
}

/// **The guard.** No shipped source names the legacy default agent or the
/// developer, outside the written allowlist.
#[test]
fn shipped_source_never_hardcodes_agent_or_developer_name() {
    let root = repo_root();
    let mut files = Vec::new();
    for src_root in shipped_source_roots(&root) {
        walk(&src_root, &mut files);
    }
    assert!(
        files.len() > 100,
        "scan found only {} files under {} — the walk is broken, not the code",
        files.len(),
        root.display()
    );

    let mut offenders = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("under repo root")
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWLIST.contains(&rel.as_str()) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue; // binary / non-UTF-8: nothing to scan
        };
        for (which, line) in scan_source(&rel, &src) {
            offenders.push(format!("{rel}:{line}: {which}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "shipped source hardcodes the agent's or the developer's name (#986). \
         The agent's name is per-user — render it through the persona seam \
         (`PrimaryPersona::display_name()` on the Rust side, the store's `agentName` / \
         `useOrchestratorName` in the UI) or drop the self-reference; the developer's \
         name is not a product fact — say \"the user\", cite rulings by date, and give \
         fixtures neutral names. If the literal is genuinely the point, add the file to \
         ALLOWLIST with a written reason.\n{}",
        offenders.join("\n")
    );
}

/// **Test-of-the-test.** The scanner must catch the historical leak shapes and
/// must ignore the shapes the rules exempt. If the lexer ever loosens into a
/// no-op — or tightens into flagging ids — this fails.
#[test]
fn identity_name_scanner_flags_leaks_and_spares_ids_comments_and_tests() {
    let agent = LEGACY_AGENT_NAME;
    let dev = DEVELOPER_NAME;

    // Rust: string literal (incl. a `\`-continued line), a doc comment, an id
    // key, a char-literal quote before the hit, and a trailing test module.
    let rs = format!(
        "//! {agent} presides here (developer shorthand — allowed).\n\
         const ID: &str = \"henry\";\n\
         fn f() {{ let q = '\"'; let s = \"ask {agent} to \\\n    read this\"; }}\n\
         /// {dev}'s ruling (comment — developer name is never allowed).\n\
         #[cfg(test)]\n\
         mod tests {{ const P: &str = \"{agent}\"; }}\n"
    );
    let hits = scan_source("crates/x/src/lib.rs", &rs);
    assert_eq!(
        hits,
        vec![("agent name", 3), ("developer name", 5)],
        "rust sample: {hits:?}"
    );

    // Rust: `#[cfg(test)] use …` at the top must NOT blank the file.
    let rs2 = format!("#[cfg(test)]\nuse foo::bar;\nconst S: &str = \"Stop {agent}\";\n");
    assert_eq!(
        scan_source("crates/x/src/a.rs", &rs2),
        vec![("agent name", 3)]
    );

    // TS/TSX: JSX text, a template literal, a `{/* */}` JSX comment, a `//`
    // inside a URL string, and a non-word identifier.
    let tsx = format!(
        "// {agent} HUD (comment)\n\
         const url = 'https://x.test/{agent}'; // still a hit: it is a string\n\
         export function A() {{ return <div>{{/* {agent} sits here */}}Have {agent} set it up</div>; }}\n\
         const t = `Stop ${{name}} — {agent}`;\n\
         const HenryHUD = 1;\n"
    );
    let hits = scan_source("ui/command-center/src/A.tsx", &tsx);
    assert_eq!(
        hits,
        vec![("agent name", 2), ("agent name", 3), ("agent name", 4)],
        "tsx sample: {hits:?}"
    );

    // TS test files: agent name is fixture-legal, developer name never is.
    let test_ts = format!("const s = {{ agentName: '{agent}', who: '{dev}' }};\n");
    assert_eq!(
        scan_source("ui/command-center/src/lib/x.test.ts", &test_ts),
        vec![("developer name", 1)]
    );

    // Lowercase id keys and the audit actor constant are never hits.
    assert!(scan_source(
        "crates/x/src/ids.rs",
        "const A: &str = \"henry\"; const B: &str = \"jesse\"; const ACTOR_JESSE: &str = \"jesse\";\n"
    )
    .is_empty());
}
