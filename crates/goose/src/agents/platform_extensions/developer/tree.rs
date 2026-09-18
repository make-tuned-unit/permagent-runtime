use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

/// Depth used when the caller doesn't specify one.
const DEFAULT_DEPTH: u32 = 4;
/// Hard ceiling on depth regardless of what's requested — including the
/// historical "0 means unlimited" request. A43: a single `tree path: "/"`
/// call walked the whole host filesystem and dumped 2,843 lines / 45k tokens,
/// forcing a context compaction. Depth is bounded no matter what's asked for.
const MAX_DEPTH: u32 = 10;
/// Entries (files + directories) shown before the rest collapse into a
/// trailing "truncated" summary line.
const DEFAULT_LIMIT: usize = 400;
/// Hard ceiling on an explicit `limit`.
const MAX_LIMIT: usize = 2000;

/// Directories skipped by default regardless of .gitignore — dependency/build
/// output that is never useful in a structural listing and can be huge.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hermit",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "dist",
    "build",
];

fn default_depth() -> u32 {
    DEFAULT_DEPTH
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TreeParams {
    /// Directory to list. Relative paths resolve against the working
    /// directory. Empty, "/", or omitted defaults to the working directory;
    /// an absolute path outside the working directory is refused.
    #[serde(default)]
    pub path: String,
    /// Max depth to descend (default 4, hard cap 10).
    #[serde(default = "default_depth")]
    pub depth: u32,
    /// Max entries (files + directories) shown before truncating (default
    /// 400, hard cap 2000).
    #[serde(default)]
    pub limit: Option<u32>,
}

pub struct TreeTool;

impl TreeTool {
    pub fn new() -> Self {
        Self
    }

    pub fn tree_with_cwd(&self, params: TreeParams, working_dir: Option<&Path>) -> CallToolResult {
        let cwd = working_dir
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        let root = match resolve_root(&params.path, &cwd) {
            Ok(root) => root,
            Err(message) => {
                return CallToolResult::error(vec![Content::text(message).with_priority(0.0)]);
            }
        };

        let limit = params
            .limit
            .map(|l| (l as usize).clamp(1, MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);

        self.tree_at(root, params.depth, limit)
    }

    fn tree_at(&self, root: PathBuf, depth: u32, limit: usize) -> CallToolResult {
        if !root.exists() {
            return CallToolResult::error(vec![Content::text(format!(
                "Path does not exist: {}",
                root.display()
            ))
            .with_priority(0.0)]);
        }

        if !root.is_dir() {
            return CallToolResult::error(vec![Content::text(format!(
                "Path is not a directory: {}",
                root.display()
            ))
            .with_priority(0.0)]);
        }

        let max_depth = Some(effective_depth(depth));

        let collected = collect_tree(&root, max_depth, limit);
        let mut tree = collected.tree;
        tree.compute_total_lines();

        let mut output = String::new();
        tree.render_into(0, &mut output);
        if output.is_empty() {
            output.push_str("(empty directory)\n");
        }
        if collected.truncated_more > 0 {
            output.push_str(&format!(
                "… truncated: {} more entries; pass depth/limit or a subpath\n",
                collected.truncated_more
            ));
        }

        CallToolResult::success(vec![Content::text(output).with_priority(0.0)])
    }
}

impl Default for TreeTool {
    fn default() -> Self {
        Self::new()
    }
}

fn effective_depth(depth: u32) -> usize {
    if depth == 0 {
        MAX_DEPTH as usize
    } else {
        depth.min(MAX_DEPTH) as usize
    }
}

/// Resolve the caller-supplied `path` against the working directory (R44a's
/// `safe_probe_path` idea, applied to a tree root instead of a probe file):
/// empty or `/` defaults to the working directory; a relative path joins it
/// but can't escape via `..`; an absolute path is only honored when it's
/// already under the working directory. Anything else is refused as a COST
/// guard, not a sandbox: `tree` exists to give a small, cheap map of the
/// workspace, and walking an arbitrary host path produces output nobody asked
/// for. `shell`/`read_file` remain the way to look at other paths.
fn resolve_root(path: &str, working_dir: &Path) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok(working_dir.to_path_buf());
    }

    let supplied = Path::new(trimmed);
    if supplied.is_absolute() {
        return if supplied.starts_with(working_dir) {
            Ok(supplied.to_path_buf())
        } else {
            Err(format!(
                "tree: `{trimmed}` is outside the workspace root ({}); tree lists only under \
                 the workspace root to keep output small — use shell or read_file for other \
                 paths, or leave path empty to list the workspace root.",
                working_dir.display()
            ))
        };
    }

    if supplied
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(format!(
            "tree: `{trimmed}` cannot contain parent-directory traversal (`..`); pass a path \
             under the workspace root ({}).",
            working_dir.display()
        ));
    }

    Ok(working_dir.join(supplied))
}

#[derive(Default)]
struct DirectoryNode {
    dirs: BTreeMap<String, DirectoryNode>,
    files: BTreeMap<String, usize>,
    total_lines: usize,
}

impl DirectoryNode {
    fn insert_dir(&mut self, components: &[String]) {
        let mut node = self;
        for component in components {
            node = node.dirs.entry(component.clone()).or_default();
        }
    }

    fn insert_file(&mut self, components: &[String], line_count: usize) {
        if components.is_empty() {
            return;
        }

        let mut node = self;
        for component in &components[..components.len() - 1] {
            node = node.dirs.entry(component.clone()).or_default();
        }

        let filename = components[components.len() - 1].clone();
        node.files.insert(filename, line_count);
    }

    fn compute_total_lines(&mut self) -> usize {
        let dir_lines: usize = self
            .dirs
            .values_mut()
            .map(DirectoryNode::compute_total_lines)
            .sum();
        let file_lines: usize = self.files.values().copied().sum();
        self.total_lines = dir_lines + file_lines;
        self.total_lines
    }

    fn render_into(&self, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);

        for (name, dir) in &self.dirs {
            out.push_str(&format!(
                "{}{}/  {}\n",
                indent,
                name,
                format_lines(dir.total_lines)
            ));
            dir.render_into(depth + 1, out);
        }

        for (name, line_count) in &self.files {
            out.push_str(&format!(
                "{}{}  {}\n",
                indent,
                name,
                format_lines(*line_count)
            ));
        }
    }
}

struct CollectResult {
    tree: DirectoryNode,
    /// Entries beyond `limit` that were counted but not inserted into the
    /// tree (so their contents are never read).
    truncated_more: usize,
}

fn collect_tree(root: &Path, max_depth: Option<usize>, limit: usize) -> CollectResult {
    let mut builder = WalkBuilder::new(root);
    builder.git_ignore(true);
    builder.git_exclude(true);
    builder.git_global(true);
    builder.require_git(false);
    builder.ignore(true);
    builder.hidden(true);
    builder.filter_entry(|entry| {
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            if let Some(name) = entry.file_name().to_str() {
                return !SKIP_DIRS.contains(&name);
            }
        }
        true
    });

    if let Some(depth) = max_depth {
        builder.max_depth(Some(depth + 1));
    }

    let mut tree = DirectoryNode::default();
    let mut shown = 0usize;
    let mut more = 0usize;

    for entry in builder.build().flatten() {
        let path = entry.path();
        if path == root {
            continue;
        }

        let rel = match path.strip_prefix(root) {
            Ok(rel) => rel,
            Err(_) => continue,
        };

        let components = match relative_components(rel) {
            Some(components) => components,
            None => continue,
        };

        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let is_file = entry.file_type().is_some_and(|t| t.is_file());
        if !is_dir && !is_file {
            continue;
        }

        if shown >= limit {
            more += 1;
            continue;
        }
        shown += 1;

        if is_dir {
            tree.insert_dir(&components);
        } else {
            tree.insert_file(&components, count_file_lines(path));
        }
    }

    CollectResult {
        tree,
        truncated_more: more,
    }
}

fn relative_components(path: &Path) -> Option<Vec<String>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
            _ => return None,
        }
    }

    if components.is_empty() {
        None
    } else {
        Some(components)
    }
}

fn count_file_lines(path: &Path) -> usize {
    match fs::read_to_string(path) {
        Ok(content) => content.lines().count(),
        Err(_) => 0,
    }
}

fn format_lines(lines: usize) -> String {
    if lines >= 1000 {
        format!("[{}K]", lines / 1000)
    } else {
        format!("[{}]", lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::RawContent;
    use tempfile::TempDir;

    fn extract_text(result: &CallToolResult) -> &str {
        match &result.content[0].raw {
            RawContent::Text(t) => &t.text,
            _ => panic!("expected text"),
        }
    }

    fn params(path: &str, depth: u32) -> TreeParams {
        TreeParams {
            path: path.to_string(),
            depth,
            limit: None,
        }
    }

    fn setup_tree() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("tests")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(dir.path().join("tests/test.rs"), "#[test]\nfn t() {}\n").unwrap();
        dir
    }

    #[test]
    fn tree_lists_files_and_directories() {
        let dir = setup_tree();
        let tool = TreeTool::new();

        let result = tool.tree_with_cwd(params("", 2), Some(dir.path()));

        let text = extract_text(&result);
        assert!(text.contains("src/"));
        assert!(text.contains("tests/"));
        assert!(text.contains("main.rs"));
    }

    #[test]
    fn tree_respects_depth() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
        fs::write(dir.path().join("a/b/c/deep.rs"), "fn deep() {}\n").unwrap();

        let tool = TreeTool::new();
        let result = tool.tree_with_cwd(params("", 1), Some(dir.path()));

        let text = extract_text(&result);
        assert!(text.contains("a/"));
        assert!(text.contains("b/"));
        assert!(!text.contains("deep.rs"));
    }

    #[test]
    fn tree_uses_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored/\n*.log\n").unwrap();
        fs::create_dir_all(dir.path().join("ignored")).unwrap();
        fs::write(dir.path().join("ignored/secret.rs"), "fn secret() {}\n").unwrap();
        fs::write(dir.path().join("visible.rs"), "fn visible() {}\n").unwrap();
        fs::write(dir.path().join("debug.log"), "hidden\n").unwrap();

        let tool = TreeTool::new();
        let result = tool.tree_with_cwd(params("", 2), Some(dir.path()));

        let text = extract_text(&result);
        assert!(text.contains("visible.rs"));
        assert!(!text.contains("ignored"));
        assert!(!text.contains("debug.log"));
    }

    #[test]
    fn empty_path_defaults_to_working_dir() {
        let dir = setup_tree();
        let tool = TreeTool::new();

        let result = tool.tree_with_cwd(params("", 2), Some(dir.path()));

        let text = extract_text(&result);
        assert!(text.contains("src/"));
        assert!(text.contains("main.rs"));
    }

    #[test]
    fn root_slash_defaults_to_working_dir() {
        let dir = setup_tree();
        let tool = TreeTool::new();

        let result = tool.tree_with_cwd(params("/", 2), Some(dir.path()));

        let text = extract_text(&result);
        assert!(text.contains("src/"));
        assert!(text.contains("main.rs"));
    }

    #[test]
    fn absolute_path_outside_working_dir_is_refused() {
        let dir = setup_tree();
        let outside = tempfile::tempdir().unwrap();
        let tool = TreeTool::new();

        let result = tool.tree_with_cwd(
            params(&outside.path().display().to_string(), 2),
            Some(dir.path()),
        );

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("outside the workspace root"));
        assert!(text.contains(&dir.path().display().to_string()));
    }

    #[test]
    fn parent_dir_escape_is_refused() {
        let dir = setup_tree();
        let tool = TreeTool::new();

        let result = tool.tree_with_cwd(params("../etc", 2), Some(dir.path()));

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains(".."));
    }

    #[test]
    fn large_tree_is_truncated_with_trailer() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..1000 {
            fs::write(dir.path().join(format!("file{i:04}.txt")), "x\n").unwrap();
        }

        let tool = TreeTool::new();
        let result = tool.tree_with_cwd(params("", 2), Some(dir.path()));

        let text = extract_text(&result);
        assert!(text.contains("… truncated:"));
        assert!(text.contains("more entries"));
        // At most `limit` entries were actually rendered as tree lines.
        let rendered_lines = text.lines().filter(|l| !l.starts_with('\u{2026}')).count();
        assert!(rendered_lines <= DEFAULT_LIMIT);
    }

    #[test]
    fn explicit_limit_is_honored() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..50 {
            fs::write(dir.path().join(format!("file{i:04}.txt")), "x\n").unwrap();
        }

        let tool = TreeTool::new();
        let result = tool.tree_with_cwd(
            TreeParams {
                path: String::new(),
                depth: 2,
                limit: Some(10),
            },
            Some(dir.path()),
        );

        let text = extract_text(&result);
        assert!(text.contains("… truncated: 40 more entries"));
    }

    #[test]
    fn ignored_build_dirs_are_skipped_by_default() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("node_modules/some-pkg")).unwrap();
        fs::write(
            dir.path().join("node_modules/some-pkg/index.js"),
            "module.exports = {};\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::write(dir.path().join("target/debug/build.log"), "ok\n").unwrap();
        fs::write(dir.path().join("src.rs"), "fn main() {}\n").unwrap();

        let tool = TreeTool::new();
        let result = tool.tree_with_cwd(params("", 3), Some(dir.path()));

        let text = extract_text(&result);
        assert!(text.contains("src.rs"));
        assert!(!text.contains("node_modules"));
        assert!(!text.contains("target/"));
        assert!(!text.contains("build.log"));
    }

    #[test]
    fn depth_zero_is_capped_not_unlimited() {
        let dir = tempfile::tempdir().unwrap();
        let mut nested = dir.path().to_path_buf();
        for i in 0..(MAX_DEPTH as usize + 5) {
            nested = nested.join(format!("d{i}"));
        }
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("deep.rs"), "fn deep() {}\n").unwrap();

        let tool = TreeTool::new();
        let result = tool.tree_with_cwd(params("", 0), Some(dir.path()));

        let text = extract_text(&result);
        assert!(!text.contains("deep.rs"));
    }
}
