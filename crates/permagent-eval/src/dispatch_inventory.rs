//! Static inventory of provider and external-process dispatch seams.
//!
//! This is deliberately a source audit, not a runtime billing implementation.
//! Every production seam must carry a nearby, reviewable marker before the
//! strict/promotion check will call it wrapped or explicitly non-paid.  An
//! unmarked seam is reported as unwrapped; it is never hidden by a heuristic.

use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const RULESET_VERSION: &str = "dispatch-inventory.v2";
const MARKER_PREFIX: &str = "permagent-dispatch:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeamKind {
    Complete,
    CompleteFast,
    StreamSplit,
    RemoteGenerationOrSampling,
    ExternalProcessSpawn,
}

impl SeamKind {
    fn rank(self) -> u8 {
        match self {
            Self::Complete => 0,
            Self::CompleteFast => 1,
            Self::StreamSplit => 2,
            Self::RemoteGenerationOrSampling => 3,
            Self::ExternalProcessSpawn => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeamClassification {
    Wrapped,
    ExplicitlyExcluded,
    Unwrapped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DispatchSeam {
    pub path: String,
    pub line: usize,
    pub kind: SeamKind,
    pub symbol: String,
    pub classification: SeamClassification,
    /// The nearby marker text, when present. This is evidence for the
    /// classification and is intentionally retained in the report.
    pub marker: Option<String>,
    pub marker_id: Option<String>,
    /// All provider and remote seams are potentially paid. Process spawning is
    /// also marked paid-capable because a CLI can own a metered model.
    pub paid_capable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DispatchInventory {
    pub ruleset: String,
    pub root: String,
    pub seams: Vec<DispatchSeam>,
}

impl DispatchInventory {
    /// Strict promotion gate. Unknown/unwrapped paid-capable seams are a hard
    /// failure; the returned list is stable and suitable for CI output.
    pub fn strict_failures(&self) -> Vec<&DispatchSeam> {
        self.seams
            .iter()
            .filter(|seam| {
                seam.paid_capable && seam.classification == SeamClassification::Unwrapped
            })
            .collect()
    }

    pub fn validate_promotion(&self) -> Result<(), String> {
        let failures = self.strict_failures();
        if failures.is_empty() {
            return Ok(());
        }
        let details = failures
            .iter()
            .map(|seam| {
                format!(
                    "{}:{} {} ({})",
                    seam.path,
                    seam.line,
                    seam.symbol,
                    kind_name(seam.kind)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "dispatch inventory promotion blocked: {} unwrapped paid-capable seam(s): {details}",
            failures.len()
        ))
    }
}

pub fn scan_production_rust(root: impl AsRef<Path>) -> Result<DispatchInventory, String> {
    let root = root.as_ref();
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)
        .map_err(|e| format!("scanning {}: {e}", root.display()))?;
    files.sort();

    let mut seams = Vec::new();
    for path in files {
        scan_file(root, &path, &mut seams)?;
    }
    seams.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.kind.rank().cmp(&b.kind.rank()))
            .then(a.symbol.cmp(&b.symbol))
    });
    Ok(DispatchInventory {
        ruleset: RULESET_VERSION.to_string(),
        root: root.to_string_lossy().into_owned(),
        seams,
    })
}

fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if root.is_file() {
        if root.extension().and_then(|v| v.to_str()) == Some("rs") {
            out.push(root.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out)?;
        } else if path.extension().and_then(|v| v.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn scan_file(root: &Path, path: &Path, out: &mut Vec<DispatchSeam>) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let masked = mask_comments_and_literals(&source);
    let original_lines: Vec<&str> = source.lines().collect();
    let masked_lines: Vec<&str> = masked.lines().collect();
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let mut brace_depth = 0i32;
    let mut test_depth: Option<i32> = None;
    let mut pending_test_attr = false;
    let mut markers: Vec<(usize, ParsedMarker, bool)> = Vec::new();
    for (index, code) in masked_lines.iter().enumerate() {
        let line_number = index + 1;
        let original = original_lines.get(index).copied().unwrap_or_default();
        if original.to_ascii_lowercase().contains(MARKER_PREFIX) && !in_test_attribute(original) {
            let parsed =
                parse_marker(original).map_err(|e| format!("{}:{}: {e}", relative, line_number))?;
            if markers
                .iter()
                .any(|(_, existing, _)| existing.id == parsed.id)
            {
                return Err(format!(
                    "{}:{}: duplicate dispatch marker seam='{}'",
                    relative, line_number, parsed.id
                ));
            }
            markers.push((index, parsed, false));
        }
        if original.contains("#[cfg(test)]") {
            pending_test_attr = true;
        }
        let opens = code.chars().filter(|c| *c == '{').count() as i32;
        let closes = code.chars().filter(|c| *c == '}').count() as i32;
        let entered_test_block = pending_test_attr && opens > 0;
        if entered_test_block {
            test_depth = Some(brace_depth + opens);
            pending_test_attr = false;
        }
        // If the cfg attribute opens and closes a one-line test module, the
        // current line is already test code. For a multi-line module, the
        // attribute line itself is also excluded from detection.
        let in_tests = pending_test_attr
            || (test_depth.is_some_and(|depth| brace_depth >= depth))
            || entered_test_block;
        if !in_tests {
            let marker = if is_candidate_line(code) {
                nearby_marker(&original_lines, index, &mut markers)
            } else {
                None
            };
            detect_provider_calls(code, &relative, line_number, marker, out);
            // A process can be constructed many lines before it is spawned;
            // a proximity window would create a dangerous false negative.
            // Exclude only syntactically certain async task spawns. An
            // uncertain `.spawn()` remains an unwrapped paid-capable seam.
            if is_process_spawn(code) {
                push_seam(
                    out,
                    &relative,
                    line_number,
                    SeamKind::ExternalProcessSpawn,
                    process_symbol(code),
                    marker,
                );
            }
        }
        brace_depth += opens - closes;
        if test_depth.is_some_and(|depth| brace_depth < depth) {
            test_depth = None;
        }
    }
    if let Some((_, marker, _)) = markers.iter().find(|(_, _, used)| !used) {
        return Err(format!(
            "{}: unused dispatch marker for seam '{}'",
            relative, marker.id
        ));
    }
    Ok(())
}

fn in_test_attribute(line: &str) -> bool {
    line.contains("#[cfg(test)]")
}

fn detect_provider_calls(
    code: &str,
    path: &str,
    line: usize,
    marker: Option<&ParsedMarker>,
    out: &mut Vec<DispatchSeam>,
) {
    // Calls only: requiring `(` and excluding `fn name` avoids counting trait
    // declarations as dispatch seams. `complete_fast` is checked first.
    if code.contains(".complete_fast(") {
        push_seam(
            out,
            path,
            line,
            SeamKind::CompleteFast,
            "complete_fast",
            marker,
        );
    }
    if code.contains(".stream_split(") {
        push_seam(
            out,
            path,
            line,
            SeamKind::StreamSplit,
            "stream_split",
            marker,
        );
    }
    if code.contains(".complete(") {
        push_seam(out, path, line, SeamKind::Complete, "complete", marker);
    }
    // Keep this list qualified. Bare `sample(` / `sampling(` also describe
    // local orchestration helpers and capability declarations (for example
    // `run_best_of_n_sampling` and MCP's `enable_sampling`). A future remote
    // adapter should use one of these provider/client-qualified shapes so it
    // cannot disappear from the audit by being mistaken for local code.
    let remote = [
        ".generate_content(",
        ".generate_stream(",
        ".sampling(",
        ".responses.create(",
    ];
    if remote.iter().any(|needle| code.contains(needle)) {
        push_seam(
            out,
            path,
            line,
            SeamKind::RemoteGenerationOrSampling,
            "remote_generation_or_sampling",
            marker,
        );
    }
}

fn is_candidate_line(code: &str) -> bool {
    code.contains(".complete(")
        || code.contains(".complete_fast(")
        || code.contains(".stream_split(")
        || code.contains(".generate_content(")
        || code.contains(".generate_stream(")
        || code.contains(".responses.create(")
        || code.contains(".sampling(")
        || is_process_spawn(code)
}

fn is_process_spawn(code: &str) -> bool {
    // `std::process::Command::spawn` takes no arguments. Requiring the empty
    // call shape keeps provider/process launches visible while excluding
    // logical dispatcher methods such as `engine.spawn(task)`, ACP's
    // `cl.spawn(rx, init_tx)`, and thread builders whose closure is passed to
    // `spawn` on the following expression line.
    has_zero_arg_spawn(code)
        && !is_definitely_async_task_spawn(code)
        && !is_known_non_process_spawn(code)
}

fn has_zero_arg_spawn(code: &str) -> bool {
    let mut offset = 0;
    // `get` rather than a slice index: every offset here lands on an ASCII
    // boundary today, but a panic in the inventory scanner would take out the
    // whole dispatch audit for one stray byte.
    while let Some(relative) = code.get(offset..).and_then(|rest| rest.find(".spawn")) {
        let start = offset + relative + ".spawn".len();
        let rest = code.get(start..).unwrap_or("").trim_start();
        if rest.starts_with("()") {
            return true;
        }
        offset = start;
        if offset >= code.len() {
            break;
        }
    }
    false
}

fn is_known_non_process_spawn(code: &str) -> bool {
    // These are typed in-process/task dispatches, not OS process launches.
    // Keep the exceptions narrow: an unknown `.spawn()` remains visible and
    // therefore cannot hide a newly-added external worker or provider CLI.
    [
        "engine.spawn(",
        "std::thread::Builder::new().spawn(",
        "sidecar_runtime().spawn(",
    ]
    .iter()
    .any(|needle| code.contains(needle))
}

fn is_definitely_async_task_spawn(code: &str) -> bool {
    // `std::process::Command::spawn` is zero-argument. A receiver invoking
    // `.spawn(async ...` is therefore syntactically an async task spawn even
    // when the executor is held in a variable such as `handle`; excluding it
    // does not reintroduce the old command-construction proximity heuristic.
    if code.contains(".spawn(async") {
        return true;
    }
    [
        "tokio::spawn(",
        "tokio::task::spawn(",
        "tokio::task::spawn_blocking(",
        "async_std::task::spawn(",
        "async_std::task::spawn_blocking(",
        "actix_rt::spawn(",
    ]
    .iter()
    .any(|prefix| code.contains(prefix))
}

fn process_symbol(code: &str) -> &'static str {
    if code.contains("command.spawn(") {
        "command.spawn"
    } else if code.contains("child.spawn(") {
        "child.spawn"
    } else {
        "cmd.spawn"
    }
}

fn push_seam(
    out: &mut Vec<DispatchSeam>,
    path: &str,
    line: usize,
    kind: SeamKind,
    symbol: impl Into<String>,
    marker: Option<&ParsedMarker>,
) {
    let classification = match marker.map(|m| m.kind) {
        Some(MarkerKind::Wrapped) => SeamClassification::Wrapped,
        Some(MarkerKind::Excluded) => SeamClassification::ExplicitlyExcluded,
        None => SeamClassification::Unwrapped,
    };
    out.push(DispatchSeam {
        path: path.to_string(),
        line,
        kind,
        symbol: symbol.into(),
        classification,
        marker: marker.map(|m| m.raw.clone()),
        marker_id: marker.map(|m| m.id.clone()),
        paid_capable: true,
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    Wrapped,
    Excluded,
}

#[derive(Debug, Clone)]
struct ParsedMarker {
    id: String,
    kind: MarkerKind,
    raw: String,
}

fn parse_marker(line: &str) -> Result<ParsedMarker, String> {
    let start = line
        .to_ascii_lowercase()
        .find(MARKER_PREFIX)
        .ok_or_else(|| "dispatch marker prefix missing".to_string())?;
    let raw = line.get(start..).unwrap_or("").trim().to_string();
    let fields: Vec<(&str, &str)> = raw
        .get(MARKER_PREFIX.len()..)
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|part| part.split_once('='))
        .collect();
    let get = |key: &str| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    let id = get("seam")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "marker requires seam=<stable-id>".to_string())?
        .to_string();
    let class = get("class").ok_or_else(|| "marker requires class=wrapped|excluded".to_string())?;
    let kind = match class {
        "wrapped" if get("contract").is_some_and(|v| !v.is_empty()) => MarkerKind::Wrapped,
        "excluded"
            if get("reason").is_some_and(|v| !v.is_empty())
                && get("authority").is_some_and(|v| !v.is_empty()) =>
        {
            MarkerKind::Excluded
        }
        "wrapped" => return Err("wrapped marker requires contract=<wrapper-contract>".to_string()),
        "excluded" => {
            return Err("excluded marker requires reason=<...> and authority=<...>".to_string())
        }
        other => return Err(format!("unknown marker class '{other}'")),
    };
    Ok(ParsedMarker { id, kind, raw })
}

fn nearby_marker<'a>(
    lines: &[&str],
    index: usize,
    markers: &'a mut [(usize, ParsedMarker, bool)],
) -> Option<&'a ParsedMarker> {
    let start = index.saturating_sub(2);
    let end = (index + 3).min(lines.len());
    let hit = markers
        .iter_mut()
        .find(|(marker_index, _, used)| *marker_index >= start && *marker_index < end && !*used)?;
    hit.2 = true;
    Some(&hit.1)
}

fn kind_name(kind: SeamKind) -> &'static str {
    match kind {
        SeamKind::Complete => "complete",
        SeamKind::CompleteFast => "complete_fast",
        SeamKind::StreamSplit => "stream_split",
        SeamKind::RemoteGenerationOrSampling => "remote_generation_or_sampling",
        SeamKind::ExternalProcessSpawn => "external_process_spawn",
    }
}

/// Replace comments, strings and character literals with spaces while keeping
/// newlines and line count. This prevents examples, prompt text, and comments
/// from becoming fake seams while retaining source line numbers.
fn mask_comments_and_literals(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    let mut block_depth = 0usize;
    while i < bytes.len() {
        if block_depth > 0 {
            if bytes[i..].starts_with(b"/*") {
                block_depth += 1;
                out.push_str("  ");
                i += 2;
            } else if bytes[i..].starts_with(b"*/") {
                block_depth -= 1;
                out.push_str("  ");
                i += 2;
            } else {
                out.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            out.push_str("  ");
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            block_depth = 1;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if bytes[i] == b'r' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let hashes = j - i - 1;
                let closing = format!("\"{}", "#".repeat(hashes));
                let end = source
                    .get(j + 1..)
                    .and_then(|rest| rest.find(&closing))
                    .map(|offset| j + 1 + offset + closing.len())
                    .unwrap_or(bytes.len());
                for byte in &bytes[i..end] {
                    out.push(if *byte == b'\n' { '\n' } else { ' ' });
                }
                i = end;
                continue;
            }
        }
        // A single quote begins a character literal only when it is not a
        // Rust lifetime (`'static`, `'a`, ...). Treating every apostrophe as
        // a literal used to mask large portions of files containing
        // lifetimes, which could hide the `#[cfg(test)]` boundary from the
        // brace tracker and make test-only process spawns look like
        // production seams.
        let char_literal = bytes[i] == b'\''
            && i + 1 < bytes.len()
            && !((bytes[i + 1] as char).is_ascii_alphabetic() || bytes[i + 1] == b'_');
        if bytes[i] == b'"' || char_literal {
            let quote = bytes[i];
            out.push(' ');
            i += 1;
            while i < bytes.len() {
                let byte = bytes[i];
                out.push(if byte == b'\n' { '\n' } else { ' ' });
                i += 1;
                if byte == b'\\' && i < bytes.len() {
                    out.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
                    i += 1;
                } else if byte == quote {
                    break;
                }
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(source: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.rs");
        let mut file = fs::File::create(path).unwrap();
        file.write_all(source.as_bytes()).unwrap();
        dir
    }

    #[test]
    fn comments_strings_and_tests_are_not_inventory_seams() {
        let dir = fixture(
            r#"// .complete_fast( fake )
const TEXT: &str = ".stream_split(fake) https://host/api/generate";
fn production() { provider.complete_fast(); }
#[cfg(test)]
mod tests { #[test] fn fake() { provider.complete(); std::process::Command::new("x").spawn(); } }
"#,
        );
        let report = scan_production_rust(dir.path()).unwrap();
        assert_eq!(report.seams.len(), 1);
        assert_eq!(report.seams[0].kind, SeamKind::CompleteFast);
        assert_eq!(
            report.seams[0].classification,
            SeamClassification::Unwrapped
        );
    }

    #[test]
    fn multiline_cfg_test_modules_and_functions_are_excluded_only_inside_their_body() {
        let dir = fixture(
            r#"fn lifetime<'a>(value: &'a str) -> &'a str { value }

#[cfg(test)]
mod tests {
    fn fake() {
        provider.complete();
        std::process::Command::new("x").spawn();
    }
}

#[cfg(test)]
fn another_fake()
{
    provider.stream_split();
}

fn production() {
    provider.complete();
}
"#,
        );
        let report = scan_production_rust(dir.path()).unwrap();
        assert_eq!(report.seams.len(), 1);
        assert_eq!(report.seams[0].line, 18);
        assert_eq!(report.seams[0].kind, SeamKind::Complete);
    }

    #[test]
    fn markers_classify_and_strict_mode_fails_unwrapped_paid_seams() {
        let dir = fixture(
            r#"// permagent-dispatch: seam=wrapped_complete class=wrapped contract=provider_meter
fn wrapped() { provider.complete(); }
// permagent-dispatch: seam=excluded_stream class=excluded reason=on_device authority=typed_local
fn excluded() { provider.stream_split(); }
fn bypass() { provider.complete_fast(); }
"#,
        );
        let report = scan_production_rust(dir.path()).unwrap();
        assert_eq!(report.seams.len(), 3);
        assert_eq!(report.seams[0].classification, SeamClassification::Wrapped);
        assert_eq!(
            report.seams[1].classification,
            SeamClassification::ExplicitlyExcluded
        );
        assert_eq!(
            report.seams[2].classification,
            SeamClassification::Unwrapped
        );
        assert_eq!(report.strict_failures().len(), 1);
        assert!(report.validate_promotion().is_err());
    }

    #[test]
    fn output_is_stably_sorted_and_ruleset_is_versioned() {
        let dir = fixture(
            "fn z() { std::process::Command::new(\"x\").spawn(); }\nfn a() { p.complete(); }\n",
        );
        let first = scan_production_rust(dir.path()).unwrap();
        let second = scan_production_rust(dir.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.ruleset, RULESET_VERSION);
        assert!(first.seams[0].line < first.seams[1].line);
    }

    #[test]
    fn malformed_duplicate_and_unused_markers_are_rejected() {
        let malformed =
            fixture("// permagent-dispatch: seam=x class=wrapped\nfn x() { p.complete(); }\n");
        let error = scan_production_rust(malformed.path()).unwrap_err();
        assert!(error.contains("contract"), "{error}");

        let unused =
            fixture("// permagent-dispatch: seam=unused class=wrapped contract=x\nfn x() {}\n");
        let error = scan_production_rust(unused.path()).unwrap_err();
        assert!(error.contains("unused dispatch marker"), "{error}");

        let duplicate = fixture(
            "// permagent-dispatch: seam=x class=wrapped contract=x\n// permagent-dispatch: seam=x class=wrapped contract=x\nfn x() { p.complete(); }\n",
        );
        let error = scan_production_rust(duplicate.path()).unwrap_err();
        assert!(error.contains("duplicate dispatch marker"), "{error}");
    }

    #[test]
    fn distant_command_construction_still_records_process_spawn() {
        let mut source =
            String::from("fn launch() {\n    let command = Command::new(\"agent\");\n");
        for _ in 0..30 {
            source.push_str("    let _padding = 1;\n");
        }
        source.push_str("    command.spawn();\n}\n");
        let dir = fixture(&source);
        let report = scan_production_rust(dir.path()).unwrap();
        assert!(report
            .seams
            .iter()
            .any(|seam| { seam.kind == SeamKind::ExternalProcessSpawn && seam.line > 24 }));
    }

    #[test]
    fn executor_handle_spawn_with_async_block_is_not_a_process_seam() {
        let dir = fixture(
            "fn schedule(handle: Handle) { handle.spawn(async move { work().await; }); }\n",
        );
        let report = scan_production_rust(dir.path()).unwrap();
        assert!(report.seams.is_empty());
    }

    #[test]
    fn local_sampling_and_task_dispatch_helpers_are_not_remote_or_process_seams() {
        let dir = fixture(
            r#"fn local() {
    run_best_of_n_sampling();
    sample();
    sampler.sample();
    client.enable_sampling();
    engine.spawn(task);
    acp_client.spawn(rx, init_tx);
    std::thread::Builder::new().spawn(move || work());
    sidecar_runtime().spawn(with_sidecar());
}
"#,
        );
        let report = scan_production_rust(dir.path()).unwrap();
        assert!(
            report.seams.is_empty(),
            "unexpected seams: {:?}",
            report.seams
        );
    }

    #[test]
    fn qualified_remote_sampling_and_unknown_process_receivers_remain_visible() {
        let dir = fixture(
            r#"fn production(provider: Provider, command: Command) {
    provider.sampling();
    command.spawn();
}
"#,
        );
        let report = scan_production_rust(dir.path()).unwrap();
        assert_eq!(report.seams.len(), 2);
        assert!(report
            .seams
            .iter()
            .any(|seam| seam.kind == SeamKind::RemoteGenerationOrSampling));
        assert!(report
            .seams
            .iter()
            .any(|seam| seam.kind == SeamKind::ExternalProcessSpawn));
    }
}
