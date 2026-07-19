//! On-disk `SKILL.md` folders — the open **agentskills.io** standard.
//!
//! A skill is a folder named `<name>/` containing a `SKILL.md` file: YAML
//! frontmatter (`name` + `description` required; optional `license`,
//! `compatibility`, `metadata`, `allowed-tools`) followed by a Markdown body,
//! plus optional bundled `scripts/`, `references/`, `assets/`. This is the
//! source-of-truth format for Permagent skills, making them portable in and out
//! of Claude Code, Cursor, Codex, Hermes, and any other agentskills.io-compatible
//! client.
//!
//! Reading reuses [`crate::agents::platform_extensions::parse_frontmatter`] — the
//! exact parser the `skills` platform extension already uses to discover on-disk
//! skills — so anything this module writes is discoverable + loadable by the live
//! `load_skill` path, and anything that path can read, this module can read too.
//!
//! The `skills` DB table is an INDEX over these folders (fast lookup + the
//! repetition-detection loop). The on-disk folder is authoritative; see
//! [`crate::skills::export_skill_to_disk`] / [`crate::skills::reconcile_skills_to_disk`].

use crate::agents::platform_extensions::parse_frontmatter;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Max `name` length (agentskills.io spec).
pub const MAX_NAME_LEN: usize = 64;
/// Max `description` length (agentskills.io spec).
pub const MAX_DESCRIPTION_LEN: usize = 1024;
/// Max `compatibility` length (agentskills.io spec).
pub const MAX_COMPATIBILITY_LEN: usize = 500;

/// The standard `SKILL.md` frontmatter. `name`/`description` are required; the
/// rest are optional per the spec. `metadata` values are kept as
/// [`serde_yaml::Value`] rather than `String` so we can read externally-authored
/// skills that write, e.g., `version: 1.0` (a number) without failing the parse —
/// interop robustness. Our own writes always use string values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMdMeta {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_yaml::Value>,
    #[serde(
        default,
        rename = "allowed-tools",
        skip_serializing_if = "Option::is_none"
    )]
    pub allowed_tools: Option<String>,
}

impl SkillMdMeta {
    /// Read a `metadata` entry as a string, coercing scalars (numbers/bools) so
    /// externally-authored skills that used an unquoted value still resolve.
    pub fn metadata_str(&self, key: &str) -> Option<String> {
        self.metadata.get(key).and_then(yaml_value_as_string)
    }

    /// The `metadata.version` field (agentskills.io stores version under
    /// `metadata`, not as a top-level field), if present.
    pub fn version(&self) -> Option<String> {
        self.metadata_str("version")
    }

    /// The Permagent index id embedded in `metadata.permagent_id`, if this skill
    /// was written by us. Absent for externally-authored skills.
    pub fn permagent_id(&self) -> Option<String> {
        self.metadata_str("permagent_id")
    }
}

fn yaml_value_as_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A parsed on-disk skill folder.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    pub meta: SkillMdMeta,
    pub body: String,
    /// Bundled files (scripts/references/assets/…), relative to the skill dir.
    pub supporting_files: Vec<PathBuf>,
    pub dir: PathBuf,
}

// ── Name / description normalization ────────────────────────────────────────

/// Turn a human display name into a spec-valid skill `name`: unicode-lowercased,
/// ASCII-alphanumerics kept, every run of other characters collapsed to a single
/// hyphen, no leading/trailing/consecutive hyphens, truncated to
/// [`MAX_NAME_LEN`]. Falls back to `"skill"` when nothing survives. The result
/// always satisfies [`is_valid_skill_name`] and is used as BOTH the frontmatter
/// `name` and the folder name (the spec requires them to match).
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_hyphen = false;
    for ch in name.chars() {
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() {
            if pending_hyphen && !out.is_empty() {
                out.push('-');
            }
            pending_hyphen = false;
            out.push(lc);
        } else {
            // Any non-alphanumeric run becomes at most one hyphen (added lazily
            // so we never emit a trailing hyphen).
            pending_hyphen = true;
        }
    }
    if out.len() > MAX_NAME_LEN {
        out.truncate(MAX_NAME_LEN);
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out.is_empty() {
        out.push_str("skill");
    }
    out
}

/// Whether `name` is a valid agentskills.io skill name: 1–64 chars, ASCII
/// lowercase alphanumerics and hyphens only, no leading/trailing hyphen, no
/// consecutive hyphens.
pub fn is_valid_skill_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Coerce a description to the spec: trimmed, non-empty (falls back to a name-
/// derived line since `description` is required), truncated to
/// [`MAX_DESCRIPTION_LEN`] characters.
pub fn sanitize_description(description: &str, fallback_name: &str) -> String {
    let trimmed = description.trim();
    let text = if trimmed.is_empty() {
        format!("Saved skill: {fallback_name}.")
    } else {
        trimmed.to_string()
    };
    text.chars().take(MAX_DESCRIPTION_LEN).collect()
}

/// Build a standard [`SkillMdMeta`] from already-valid parts. `extra` is folded
/// into `metadata` as string values (e.g. `version`, `source`, `permagent_id`,
/// `display_name`).
pub fn build_meta(name: &str, description: &str, extra: BTreeMap<String, String>) -> SkillMdMeta {
    let metadata = extra
        .into_iter()
        .map(|(k, v)| (k, serde_yaml::Value::String(v)))
        .collect();
    SkillMdMeta {
        name: name.to_string(),
        description: description.to_string(),
        license: None,
        compatibility: None,
        metadata,
        allowed_tools: None,
    }
}

// ── Validate / render / parse ───────────────────────────────────────────────

/// Validate frontmatter against the agentskills.io spec.
pub fn validate_meta(meta: &SkillMdMeta) -> Result<(), String> {
    if !is_valid_skill_name(&meta.name) {
        return Err(format!(
            "invalid skill name '{}': must be 1-{MAX_NAME_LEN} chars, lowercase \
             alphanumerics and single hyphens, no leading/trailing hyphen",
            meta.name
        ));
    }
    if meta.description.trim().is_empty() {
        return Err("skill description must be non-empty".to_string());
    }
    if meta.description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(format!(
            "skill description exceeds {MAX_DESCRIPTION_LEN} characters"
        ));
    }
    if let Some(c) = &meta.compatibility {
        if c.chars().count() > MAX_COMPATIBILITY_LEN {
            return Err(format!(
                "skill compatibility exceeds {MAX_COMPATIBILITY_LEN} characters"
            ));
        }
    }
    Ok(())
}

/// Render frontmatter + body into `SKILL.md` file contents.
pub fn render_skill_md(meta: &SkillMdMeta, body: &str) -> Result<String, String> {
    let yaml = serde_yaml::to_string(meta).map_err(|e| format!("serialize frontmatter: {e}"))?;
    // serde_yaml 0.9 does not emit document markers, but strip a leading `---`
    // and trailing `...`/whitespace defensively so we never double the fence.
    let yaml = yaml.strip_prefix("---\n").unwrap_or(&yaml);
    let yaml = yaml.trim_end().trim_end_matches("...").trim_end();
    let body = body.trim();
    if body.is_empty() {
        Ok(format!("---\n{yaml}\n---\n"))
    } else {
        Ok(format!("---\n{yaml}\n---\n\n{body}\n"))
    }
}

/// Parse `SKILL.md` contents into frontmatter + body, using the same parser the
/// live `load_skill` discovery path uses.
pub fn parse_skill_md(content: &str) -> Result<(SkillMdMeta, String), String> {
    parse_frontmatter::<SkillMdMeta>(content)
        .map_err(|e| format!("parse SKILL.md frontmatter: {e}"))?
        .ok_or_else(|| "SKILL.md is missing its YAML frontmatter (--- ... ---)".to_string())
}

// ── Folder I/O ──────────────────────────────────────────────────────────────

/// Write a standards-valid `SKILL.md` folder at `dir`. `dir`'s basename MUST
/// equal `meta.name` (the spec requires the folder name to match). Creates the
/// directory if needed and returns the path to the written `SKILL.md`.
pub fn write_skill_folder(dir: &Path, meta: &SkillMdMeta, body: &str) -> Result<PathBuf, String> {
    validate_meta(meta)?;
    let dir_name = dir.file_name().and_then(|n| n.to_str());
    if dir_name != Some(meta.name.as_str()) {
        return Err(format!(
            "skill folder '{}' must be named after the skill '{}' (agentskills.io: \
             name must match the parent directory)",
            dir.display(),
            meta.name
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create skill dir {}: {e}", dir.display()))?;
    let content = render_skill_md(meta, body)?;
    let skill_md = dir.join("SKILL.md");
    std::fs::write(&skill_md, content).map_err(|e| format!("write {}: {e}", skill_md.display()))?;
    Ok(skill_md)
}

/// Read a skill folder: parse its `SKILL.md` and enumerate bundled files.
pub fn read_skill_folder(dir: &Path) -> Result<ParsedSkill, String> {
    let skill_md = dir.join("SKILL.md");
    let content = std::fs::read_to_string(&skill_md)
        .map_err(|e| format!("read {}: {e}", skill_md.display()))?;
    let (meta, body) = parse_skill_md(&content)?;
    let mut supporting_files = Vec::new();
    collect_supporting_files(dir, dir, &mut supporting_files);
    supporting_files.sort();
    Ok(ParsedSkill {
        meta,
        body,
        supporting_files,
        dir: dir.to_path_buf(),
    })
}

fn collect_supporting_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skip = matches!(
                path.file_name().and_then(|n| n.to_str()),
                Some(".git") | Some(".hg") | Some(".svn")
            );
            if !skip {
                collect_supporting_files(root, &path, out);
            }
        } else if path.is_file() {
            // The top-level manifest is not a "supporting" file.
            let is_root_manifest = path.parent() == Some(root)
                && path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md");
            if is_root_manifest {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn slugify_normalizes_to_spec() {
        assert_eq!(slugify("Weekly Report!"), "weekly-report");
        assert_eq!(slugify("  --Hello__World--  "), "hello-world");
        assert_eq!(slugify("PDF Processing"), "pdf-processing");
        assert_eq!(slugify("a   b   c"), "a-b-c");
        assert_eq!(slugify("already-valid-name"), "already-valid-name");
        // Nothing survives → deterministic fallback.
        assert_eq!(slugify("***"), "skill");
        assert_eq!(slugify(""), "skill");
        // Long names truncate to the limit with no trailing hyphen.
        let long = slugify(&"word ".repeat(40));
        assert!(long.len() <= MAX_NAME_LEN);
        assert!(!long.ends_with('-'));
        assert!(is_valid_skill_name(&long));
    }

    #[test]
    fn slugify_output_is_always_valid() {
        let very_long = "x".repeat(200);
        for input in [
            "Weekly Report!",
            "  --Hello__World--  ",
            "café output",
            "___",
            "UPPER",
            "trailing---",
            very_long.as_str(),
        ] {
            assert!(
                is_valid_skill_name(&slugify(input)),
                "slugify({input:?}) produced an invalid name"
            );
        }
    }

    #[test]
    fn valid_name_predicate() {
        assert!(is_valid_skill_name("pdf-processing"));
        assert!(is_valid_skill_name("data-analysis"));
        assert!(is_valid_skill_name("skill1"));
        // Invalid per spec.
        assert!(!is_valid_skill_name("PDF-Processing")); // uppercase
        assert!(!is_valid_skill_name("-pdf")); // leading hyphen
        assert!(!is_valid_skill_name("pdf-")); // trailing hyphen
        assert!(!is_valid_skill_name("pdf--processing")); // double hyphen
        assert!(!is_valid_skill_name("")); // empty
        assert!(!is_valid_skill_name(&"a".repeat(65))); // too long
        assert!(!is_valid_skill_name("has space"));
    }

    #[test]
    fn description_is_sanitized() {
        assert_eq!(sanitize_description("  hi  ", "x"), "hi");
        // Empty → non-empty fallback (description is required).
        assert_eq!(
            sanitize_description("   ", "my-skill"),
            "Saved skill: my-skill."
        );
        // Truncated to the limit.
        let long = sanitize_description(&"z".repeat(5000), "x");
        assert_eq!(long.chars().count(), MAX_DESCRIPTION_LEN);
    }

    #[test]
    fn render_parse_round_trip() {
        let meta = build_meta(
            "weekly-report",
            "Draft the weekly status report. Use when the user asks for the weekly update.",
            BTreeMap::from([
                ("version".to_string(), "1".to_string()),
                ("source".to_string(), "permagent".to_string()),
                ("display_name".to_string(), "Weekly Report".to_string()),
            ]),
        );
        let body = "## Approach\n\n1. Gather the week's commits.\n2. Summarize by theme.";
        let rendered = render_skill_md(&meta, body).unwrap();
        // Standard shape: frontmatter fence, then body.
        assert!(rendered.starts_with("---\n"));
        assert!(rendered.contains("name: weekly-report"));
        assert!(rendered.contains("description:"));

        let (parsed, parsed_body) = parse_skill_md(&rendered).unwrap();
        assert_eq!(parsed.name, "weekly-report");
        assert_eq!(parsed.description, meta.description);
        assert_eq!(parsed.version().as_deref(), Some("1"));
        assert_eq!(
            parsed.metadata_str("display_name").as_deref(),
            Some("Weekly Report")
        );
        assert_eq!(parsed_body, body);
    }

    #[test]
    fn write_read_folder_round_trip_with_bundled_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("weekly-report");
        let meta = build_meta(
            "weekly-report",
            "Draft the weekly status report.",
            BTreeMap::from([("version".to_string(), "2".to_string())]),
        );
        let body = "Do the weekly report.";
        write_skill_folder(&dir, &meta, body).unwrap();

        // Bundle optional resources like the spec's scripts/ + references/.
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/build.sh"), "echo hi").unwrap();
        std::fs::create_dir_all(dir.join("references")).unwrap();
        std::fs::write(dir.join("references/REFERENCE.md"), "# ref").unwrap();

        let parsed = read_skill_folder(&dir).unwrap();
        assert_eq!(parsed.meta.name, "weekly-report");
        assert_eq!(parsed.meta.description, "Draft the weekly status report.");
        assert_eq!(parsed.meta.version().as_deref(), Some("2"));
        assert_eq!(parsed.body, body);
        assert!(parsed
            .supporting_files
            .contains(&PathBuf::from("scripts/build.sh")));
        assert!(parsed
            .supporting_files
            .contains(&PathBuf::from("references/REFERENCE.md")));
        // The manifest itself is not a supporting file.
        assert!(!parsed.supporting_files.contains(&PathBuf::from("SKILL.md")));
    }

    #[test]
    fn reads_externally_authored_skill() {
        // A hand-written skill exactly as a Claude Code / Cursor / Codex author
        // would ship it: minimal frontmatter (name + description only), a body,
        // and a bundled script. No Permagent metadata at all.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("pdf-processing");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: pdf-processing\ndescription: Extract text and tables from PDFs. Use when \
             handling PDF documents.\n---\n\nUse scripts/extract.py to pull text.\n",
        )
        .unwrap();
        std::fs::write(dir.join("scripts/extract.py"), "print('x')").unwrap();

        let parsed = read_skill_folder(&dir).unwrap();
        assert_eq!(parsed.meta.name, "pdf-processing");
        assert!(parsed
            .meta
            .description
            .starts_with("Extract text and tables"));
        assert_eq!(parsed.meta.version(), None); // external → no metadata
        assert_eq!(parsed.meta.permagent_id(), None);
        assert!(parsed.body.contains("scripts/extract.py"));
        assert!(parsed
            .supporting_files
            .contains(&PathBuf::from("scripts/extract.py")));
    }

    #[test]
    fn validate_rejects_bad_frontmatter() {
        // Bad name.
        let mut m = build_meta("Bad Name", "ok", BTreeMap::new());
        assert!(validate_meta(&m).is_err());
        // Empty description.
        m = build_meta("good-name", "   ", BTreeMap::new());
        assert!(validate_meta(&m).is_err());
        // Valid.
        m = build_meta("good-name", "a real description", BTreeMap::new());
        assert!(validate_meta(&m).is_ok());
    }

    #[test]
    fn write_rejects_name_dir_mismatch() {
        let tmp = TempDir::new().unwrap();
        let meta = build_meta("weekly-report", "desc", BTreeMap::new());
        // Folder basename does not match the skill name → rejected.
        let err = write_skill_folder(&tmp.path().join("something-else"), &meta, "body");
        assert!(err.is_err());
    }
}
