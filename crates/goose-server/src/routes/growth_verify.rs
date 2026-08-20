//! "Verify change" — evidence that the change landed, not a checkbox.
//!
//! The proposal's rule: verification "should mean the system *checked*,
//! otherwise it is a checkbox with extra steps", and `verified_by` is shown on
//! the card because "verified from a commit" and "you told me so" are different
//! claims that must not look identical.
//!
//! Strategy by `artifact_kind`, best available wins, falling back to explicit
//! self-attestation rather than blocking. Every attempt returns a [`Check`] —
//! including the ones that could not run — so a card can say *why* it could not
//! confirm rather than silently reading as "not done".

use crate::routes::analytics_verify::Check;
use permagent::growth::metrics::ANSWER_ENGINE_VISIT_EVENT;
use permagent::growth::store::{
    self as growth_store, GrowthActionRow, STATUS_DISMISSED, STATUS_DONE, STATUS_SUGGESTED,
    VERIFIED_BY_CONTENT, VERIFIED_BY_EVENT, VERIFIED_BY_GIT, VERIFIED_BY_SELF,
};
use permagent::projects::Project;
use sqlx::{Pool, Sqlite};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Stdio;

/// What the verify pass concluded.
#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    /// `None` when nothing could confirm the change.
    pub verified_by: Option<&'static str>,
    pub checks: Vec<Check>,
}

fn pass(id: &'static str, label: &'static str, detail: impl Into<String>) -> Check {
    Check {
        id,
        label,
        passed: true,
        detail: detail.into(),
    }
}

fn fail(id: &'static str, label: &'static str, detail: impl Into<String>) -> Check {
    Check {
        id,
        label,
        passed: false,
        detail: detail.into(),
    }
}

/// Run `git <args>` in `dir`; `Some(stdout)` on a clean exit (possibly empty),
/// `None` if git failed to launch or exited non-zero.
///
/// The empty/failed distinction is the whole point: `git log --since=…` with no
/// matching commits exits 0 with no output, which means "checked, nothing
/// found". A non-zero exit means "could not check", and the two must not render
/// the same way (mirrors `steward::hygiene::git_checked`, which is
/// `pub(crate)` to the permagent crate and so not reachable from here).
async fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Repo-relative paths the action's own text names.
///
/// The proposal asks for "a commit touching the named area", and the area is
/// only ever named in prose — the generator is told to "name the concrete
/// change — the route, the meta tags, the schema.org type"
/// (growth_actions.rs:318). So the paths are read back out of that prose. A
/// token counts only when it has a directory separator AND a file extension:
/// bare words like "the homepage" match half the repo, and a check that matches
/// everything confirms nothing.
pub fn named_paths(text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for raw in text.split(|c: char| c.is_whitespace() || "\"'`(),;<>[]{}".contains(c)) {
        let token = raw.trim_matches(|c: char| c == '.' || c == ':' || c == '*');
        if token.len() < 4 || !token.contains('/') {
            continue;
        }
        // Reject URLs: an href in a drafted post is not a repo path.
        if token.contains("://") || token.starts_with("//") {
            continue;
        }
        let Some(last) = token.rsplit('/').next() else {
            continue;
        };
        let dotted = last.rfind('.').is_some_and(|i| i > 0 && i + 1 < last.len());
        if !dotted {
            continue;
        }
        if !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
        {
            continue;
        }
        out.insert(token.trim_start_matches("./").to_string());
    }
    out.into_iter().collect()
}

/// Schema.org types a growth action actually proposes, and almost never appear
/// in a repo by accident. "Event" and "Product" are not on this list — they
/// match ordinary copy — and a check that matches everything confirms nothing.
const DISTINCTIVE_SCHEMA: &[&str] = &[
    "FAQPage",
    "BreadcrumbList",
    "HowTo",
    "QAPage",
    "SoftwareApplication",
    "JobPosting",
    "VideoObject",
    "HowToStep",
    "SpeakableSpecification",
    "ItemList",
];

/// A marker strong enough that finding it in the current tree means this
/// action's change has already landed.
///
/// Generic English is not a marker: "homepage" and "search" match half the
/// repo. The Steward will only auto-dismiss when one of these is both named by
/// the action and present in a file.
pub fn strong_markers(text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let lower = text.to_ascii_lowercase();
    for schema in DISTINCTIVE_SCHEMA {
        if text.contains(schema) {
            out.insert((*schema).to_string());
        }
    }
    if lower.contains("application/ld+json")
        || lower.contains("json-ld")
        || lower.contains("jsonld")
    {
        out.insert("application/ld+json".into());
    }
    let structured = lower.contains("schema.org")
        || lower.contains("structured data")
        || lower.contains("json-ld");
    if structured {
        for (word, spaced, compact) in [
            ("Event", r#""@type": "Event""#, r#""@type":"Event""#),
            ("Product", r#""@type": "Product""#, r#""@type":"Product""#),
            (
                "Organization",
                r#""@type": "Organization""#,
                r#""@type":"Organization""#,
            ),
            ("Article", r#""@type": "Article""#, r#""@type":"Article""#),
            ("WebSite", r#""@type": "WebSite""#, r#""@type":"WebSite""#),
        ] {
            if text.contains(word) {
                out.insert(spaced.into());
                out.insert(compact.into());
            }
        }
    }
    out.into_iter().collect()
}

/// Where a marker was found. `path` is repo-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub marker: String,
    pub path: String,
}

impl Presence {
    pub fn detail(&self) -> String {
        format!("{} in {}", self.marker, self.path)
    }
}

/// Scan already-read files. Pure, so the Steward's dismiss rule can be asserted
/// without a git repo on disk.
pub fn presence_in_files(markers: &[String], files: &[(String, String)]) -> Option<Presence> {
    if markers.is_empty() {
        return None;
    }
    for (path, content) in files {
        if is_noise_path(path) {
            continue;
        }
        for marker in markers {
            if content.contains(marker.as_str()) {
                return Some(Presence {
                    marker: marker.clone(),
                    path: path.clone(),
                });
            }
        }
    }
    None
}

fn is_noise_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("node_modules/")
        || lower.contains("/target/")
        || lower.contains("package-lock")
        || lower.ends_with(".lock")
        || lower.contains("changelog")
        || lower.contains("/.git/")
}

const READ_CAP: usize = 256 * 1024;

fn read_capped(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > READ_CAP {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// `git grep -l -F`; empty vec means checked, nothing found. `None` means git
/// could not run — the same distinction `git()` makes for log.
async fn git_grep_files(dir: &Path, needle: &str) -> Option<Vec<String>> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["grep", "-l", "-F", "-I", "-z", "--", needle])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    match out.status.code() {
        Some(0) => Some(
            String::from_utf8_lossy(&out.stdout)
                .split('\0')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        Some(1) => Some(Vec::new()),
        _ => None,
    }
}

fn action_text(title: &str, recommendation: &str, artifact: Option<&str>) -> String {
    format!("{title} {recommendation} {}", artifact.unwrap_or_default())
}

/// Is this action's change already in the project's working tree or HEAD?
///
/// Independent of when the action was suggested: `verify_git` only looks at
/// commits *since* `created_at`, which is why "Review again" kept proposing
/// work that had been sitting in the repo the whole time. This looks at the
/// files as they are.
pub async fn already_present(project: &Project, text: &str) -> Option<Presence> {
    let root = project.root_path.as_deref().filter(|p| !p.is_empty())?;
    let root = Path::new(root);
    let markers = strong_markers(text);
    if markers.is_empty() {
        return None;
    }

    let named: Vec<String> = named_paths(text)
        .into_iter()
        .filter(|rel| root.join(rel).is_file())
        .collect();

    let mut files: Vec<(String, String)> = Vec::new();
    let candidates: Vec<String> = if !named.is_empty() {
        named
    } else {
        let mut hits = BTreeSet::new();
        for marker in &markers {
            let Some(found) = git_grep_files(root, marker).await else {
                continue;
            };
            for rel in found {
                if !is_noise_path(&rel) {
                    hits.insert(rel);
                }
            }
            if hits.len() >= 12 {
                break;
            }
        }
        hits.into_iter().take(12).collect()
    };

    for rel in candidates {
        if files.len() >= 12 {
            break;
        }
        if let Some(content) = read_capped(&root.join(&rel)) {
            files.push((rel, content));
        }
    }
    presence_in_files(&markers, &files)
}

/// One suggested action the Steward took off the board because the change is
/// already in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DismissedPresence {
    pub title: String,
    pub detail: String,
}

/// Dismiss `suggested` / `done` actions whose change is already in this
/// project's repo.
///
/// Live experiments (`verified` / `measuring` / `judged`) are left alone — those
/// are being measured, not waiting on a decision. Dismissed rows stay dismissed
/// so the generator cannot re-propose them.
pub async fn dismiss_already_present(
    pool: &Pool<Sqlite>,
    project: &Project,
) -> Vec<DismissedPresence> {
    let rows = match growth_store::board(pool, &project.id).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                target: "permagentd::growth",
                "Steward could not read the growth board: {e}"
            );
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for row in rows {
        if row.status != STATUS_SUGGESTED && row.status != STATUS_DONE {
            continue;
        }
        let text = action_text(&row.title, &row.recommendation, row.artifact.as_deref());
        let Some(found) = already_present(project, &text).await else {
            continue;
        };
        match growth_store::set_status(pool, &project.id, &row.id, STATUS_DISMISSED, None).await {
            Ok(Some(_)) => {
                let detail = found.detail();
                tracing::info!(
                    target: "permagentd::growth",
                    "Steward dismissed \"{}\": already in the repo ({detail})",
                    row.title
                );
                out.push(DismissedPresence {
                    title: row.title,
                    detail,
                });
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(
                target: "permagentd::growth",
                "Steward could not dismiss \"{}\": {e}",
                row.title
            ),
        }
    }
    out
}

/// What the generator is shown about the repo, so "Review again" is a reading
/// of the current tree rather than a re-print of last week's suggestions.
///
/// `None` when there is no repo to read and nothing was dismissed — silence,
/// not an empty heading.
pub async fn render_codebase_brief(
    project: &Project,
    dismissed: &[DismissedPresence],
) -> Option<String> {
    let root = project.root_path.as_deref().filter(|p| !p.is_empty());
    let mut out = String::from(
        "This project's git repo, as it is right now. Do NOT propose a change that is \
         already in the tree.\n",
    );
    let mut has_repo = false;
    if let Some(root) = root {
        let root = Path::new(root);
        if git(root, &["rev-parse", "--git-dir"]).await.is_some() {
            has_repo = true;
            if let Some(head) = git(root, &["rev-parse", "--short", "HEAD"]).await {
                out.push_str(&format!("HEAD {head}"));
                if let Some(subject) = git(root, &["log", "-1", "--format=%s"]).await {
                    out.push_str(&format!(" \"{subject}\""));
                }
                out.push('\n');
            }
            if let Some(log) = git(root, &["log", "-8", "--format=%h %s"]).await {
                if !log.is_empty() {
                    out.push_str("Recent commits:\n");
                    for line in log.lines() {
                        out.push_str(&format!("- {line}\n"));
                    }
                }
            }
        }
    }
    if !dismissed.is_empty() {
        out.push_str("The Steward dismissed suggested actions already present in the tree:\n");
        for row in dismissed {
            out.push_str(&format!("- \"{}\" — {}\n", row.title, row.detail));
        }
    }
    if !has_repo && dismissed.is_empty() {
        return None;
    }
    Some(out)
}

/// One commit, as `git log` reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub files: Vec<String>,
}

impl Commit {
    /// The short sha the user would recognise from a git UI.
    fn short(&self) -> &str {
        self.sha.get(..8).unwrap_or(&self.sha)
    }
}

/// Parse `git log --format=%x00%H%x1f%s --name-only` into commits and their
/// files.
///
/// One read, one source of truth. `verify_git` used to run two `git log` calls —
/// one for subjects, one for changed files — and a commit landing between them
/// would make the two disagree, so the check could report "3 commits" while
/// matching a path from four. The NUL record separator is what makes a single
/// call parseable: a commit subject may contain anything except a NUL, so
/// splitting on newlines could not tell a subject from a filename.
pub fn parse_commits(raw: &str) -> Vec<Commit> {
    let mut out = Vec::new();
    for record in raw.split('\u{0}') {
        let record = record.trim_matches('\n');
        if record.trim().is_empty() {
            continue;
        }
        let mut lines = record.lines();
        let Some(header) = lines.next() else {
            continue;
        };
        let (sha, subject) = match header.split_once('\u{1f}') {
            Some((sha, subject)) => (sha.trim(), subject.trim()),
            // A header with no separator is not a commit record; skipping it is
            // safer than inventing an empty subject for it.
            None => continue,
        };
        if sha.is_empty() {
            continue;
        }
        out.push(Commit {
            sha: sha.to_string(),
            subject: subject.to_string(),
            // A merge commit reports no files under --name-only, which is not an
            // error: it is a commit that changed nothing on its own.
            files: lines
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        });
    }
    out
}

/// Did any commit change a file the action named, and which one?
///
/// Matching is suffix-based in both directions so `src/pages/index.astro` in the
/// prose matches `apps/site/src/pages/index.astro` in the repo, and a bare
/// `index.astro` matches too. The owning commit comes back with the match so a
/// passing check can name the sha and the subject rather than saying only that
/// something, somewhere, matched.
fn touches_named_path<'a>(
    commits: &'a [Commit],
    named: &[String],
) -> Option<(&'a Commit, String, String)> {
    for commit in commits {
        for want in named {
            for got in &commit.files {
                if got == want || got.ends_with(&format!("/{want}")) || want.ends_with(got.as_str())
                {
                    return Some((commit, want.clone(), got.clone()));
                }
            }
        }
    }
    None
}

/// The three things a git check can conclude, as pure functions of what was
/// found.
///
/// These are separate from [`verify_git`] because they are the whole
/// user-visible output of the check and the only part of it a unit test can
/// reach: `verify_git` needs a `Project`, a repo on disk and a subprocess. When
/// the strings were inline, the tests that claimed to cover them re-typed the
/// `format!` in the test body and asserted on their own copy — so deleting the
/// sha from the real string left them green. Both now call these.
fn untargeted_detail(commits: &[Commit]) -> String {
    format!(
        "{} commit(s) since the action was issued, most recently {} \"{}\". The action names no \
         file path, so this confirms work happened here, not that this change is the one that \
         landed.",
        commits.len(),
        commits[0].short(),
        commits[0].subject
    )
}

fn passing_detail(commit: &Commit, want: &str, got: &str) -> String {
    format!(
        "Commit {} \"{}\" changed {got}, which the action named as {want}.",
        commit.short(),
        commit.subject
    )
}

fn missing_detail(commits: &[Commit], named: &[String]) -> String {
    format!(
        "{} commit(s) landed since the action was issued ({}) but none touched what it named \
         ({}). Commit the change, or verify another way.",
        commits.len(),
        commits
            .iter()
            .take(4)
            .map(Commit::short)
            .collect::<Vec<_>>()
            .join(", "),
        named.join(", ")
    )
}

/// `prompt` actions: a commit in the project's repo since the action was issued.
async fn verify_git(project: &Project, action: &GrowthActionRow) -> Check {
    const ID: &str = "git_commit";
    const LABEL: &str = "A commit in this project's repo since the action was issued";

    let Some(root) = project.root_path.as_deref().filter(|p| !p.is_empty()) else {
        return fail(
            ID,
            LABEL,
            "This project has no root path, so there is no repo to read. Set the project's local \
             folder, or verify another way.",
        );
    };
    let root = Path::new(root);
    // "not a repo" must render as CANNOT VERIFY, never as NOT DONE.
    if git(root, &["rev-parse", "--git-dir"]).await.is_none() {
        return fail(
            ID,
            LABEL,
            format!(
                "{} is not a git repository, so commits cannot be read.",
                root.display()
            ),
        );
    }

    let since = format!("--since={}", action.created_at);
    // ONE read: subjects and changed files in the same log, so the two branches
    // below can never describe different sets of commits.
    let Some(raw) = git(
        root,
        &["log", &since, "--format=%x00%H%x1f%s", "--name-only"],
    )
    .await
    else {
        return fail(ID, LABEL, "git log failed, so commits could not be read.");
    };
    let commits = parse_commits(&raw);
    if commits.is_empty() {
        return fail(
            ID,
            LABEL,
            format!(
                "No commits in {} since the action was issued ({}).",
                root.display(),
                action.created_at
            ),
        );
    }

    let named = named_paths(&format!(
        "{} {}",
        action.recommendation,
        action.artifact.as_deref().unwrap_or_default()
    ));
    if named.is_empty() {
        // Honest about the weaker claim: any commit, not a targeted one.
        return pass(ID, LABEL, untargeted_detail(&commits));
    }

    match touches_named_path(&commits, &named) {
        // A pass has to say what it found. A bare green tick is indistinguishable
        // from a check that matched the wrong thing, and the user cannot audit a
        // verification whose evidence it never shows them.
        Some((commit, want, got)) => pass(ID, LABEL, passing_detail(commit, &want, &got)),
        None => fail(ID, LABEL, missing_detail(&commits, &named)),
    }
}

/// `post` actions: a traffic source or answer-engine visit that was absent
/// before the action was issued.
///
/// "Before" is ALL recorded history, not a matched window. That asymmetry is
/// deliberate and only safe in this direction: the claim being made is "this
/// source has never appeared until now", and widening the before side can only
/// make that harder to satisfy. A matched 7-day before window would flag a
/// source the site sees every fortnight as brand new.
async fn verify_event(pool: &Pool<Sqlite>, project_id: &str, action: &GrowthActionRow) -> Check {
    const ID: &str = "new_traffic_source";
    const LABEL: &str = "A traffic source or answer-engine visit that was not there before";

    let sources = |before: bool| async move {
        let sql = if before {
            "SELECT DISTINCT coalesce(nullif(utm_source, ''), referrer) FROM analytics_events
              WHERE project_id = ?1 AND is_bot = 0 AND created_at < ?2
                AND coalesce(nullif(utm_source, ''), referrer) IS NOT NULL"
        } else {
            "SELECT DISTINCT coalesce(nullif(utm_source, ''), referrer) FROM analytics_events
              WHERE project_id = ?1 AND is_bot = 0 AND created_at >= ?2
                AND coalesce(nullif(utm_source, ''), referrer) IS NOT NULL"
        };
        sqlx::query_scalar::<_, String>(sql)
            .bind(project_id)
            .bind(&action.created_at)
            .fetch_all(pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>()
    };

    let before = sources(true).await;
    let after = sources(false).await;
    let fresh: Vec<&String> = after.difference(&before).collect();

    let aeo: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM analytics_events
          WHERE project_id = ?1 AND kind = 'event' AND name = ?3 AND is_bot = 0
            AND created_at >= ?2",
    )
    .bind(project_id)
    .bind(&action.created_at)
    .bind(ANSWER_ENGINE_VISIT_EVENT)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    if !fresh.is_empty() {
        return pass(
            ID,
            LABEL,
            format!(
                "{} source(s) appeared since the action was issued that had never been seen \
                 before: {}.",
                fresh.len(),
                fresh
                    .iter()
                    .take(4)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    if aeo > 0 {
        return pass(
            ID,
            LABEL,
            format!("{aeo} answer-engine visit(s) recorded since the action was issued."),
        );
    }
    fail(
        ID,
        LABEL,
        format!(
            "No new traffic source and no answer-engine visit since the action was issued ({}). \
             This is also what a published post with no reach yet looks like.",
            action.created_at
        ),
    )
}

/// Any action: the live page actually contains the change.
///
/// Needs the caller to say what to look for. Without a substring there is
/// nothing to assert, and a fetch that only proves the site is up would be a
/// green tick for nothing.
async fn verify_content(project: &Project, expect: Option<&str>) -> Check {
    const ID: &str = "live_content";
    const LABEL: &str = "The live page contains the change";

    let Some(expect) = expect.map(str::trim).filter(|s| !s.is_empty()) else {
        return fail(
            ID,
            LABEL,
            "No text to look for was given, so there is nothing to assert against the live page.",
        );
    };
    let Some(url) = project.site_url.as_deref().filter(|u| !u.is_empty()) else {
        return fail(
            ID,
            LABEL,
            "This project has no site URL, so the live page cannot be fetched.",
        );
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => return fail(ID, LABEL, format!("Could not build an HTTP client: {e}")),
    };
    let body = match client.get(url).send().await {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        Ok(r) => {
            return fail(
                ID,
                LABEL,
                format!(
                    "{url} returned HTTP {}, so its content could not be read.",
                    r.status()
                ),
            );
        }
        Err(e) => return fail(ID, LABEL, format!("Could not fetch {url}: {e}")),
    };

    if body.contains(expect) {
        pass(ID, LABEL, format!("{url} contains \"{expect}\"."))
    } else {
        fail(
            ID,
            LABEL,
            format!(
                "{url} was fetched but does not contain \"{expect}\". Deploy the change, or check \
                 the text is exactly as it appears on the page."
            ),
        )
    }
}

/// Try every strategy that applies, in the order the proposal ranks them for
/// this `artifact_kind`, and report all of them.
///
/// All checks run even after one passes: the card shows what was and was not
/// confirmed, and a user who sees "commit found, live page does not contain it"
/// learns something a single green tick would have hidden.
/// Whether the `content` strategy has anything to assert.
///
/// It needs the caller to say what to look for, and no caller in the product
/// does — `expectSubstring` appears nowhere in the UI, which posts only
/// `targetBody()` or `{...targetBody(), selfAttested: true}`. So the check ran
/// on every `post` action and every artifact-less one and could only come back
/// red, with the detail "No text to look for was given". A user reads a red
/// "The live page contains the change" as "my change is not live"; it meant
/// "nobody told me what to look for". A check that cannot conclude is not
/// evidence, so it is skipped rather than shown as a failure. The strategy
/// stays wired for a caller that does supply a substring.
fn can_check_content(expect: Option<&str>) -> bool {
    !expect.map(str::trim).unwrap_or_default().is_empty()
}

pub async fn verify(
    pool: &Pool<Sqlite>,
    project: &Project,
    action: &GrowthActionRow,
    expect_substring: Option<&str>,
    self_attested: bool,
) -> VerifyOutcome {
    let kind = action.artifact_kind.as_deref().unwrap_or("none");
    // Preference order per kind. `post` copy never lands as a commit, and a
    // `prompt` that changed a repo may not be deployed yet, so the order is not
    // the same for both.
    let order: &[&str] = match kind {
        "post" => &[VERIFIED_BY_EVENT, VERIFIED_BY_CONTENT],
        "prompt" => &[VERIFIED_BY_GIT, VERIFIED_BY_CONTENT],
        _ => &[VERIFIED_BY_CONTENT, VERIFIED_BY_GIT],
    };

    let mut checks = Vec::new();
    let mut verified_by = None;
    for strategy in order {
        if *strategy == VERIFIED_BY_CONTENT && !can_check_content(expect_substring) {
            continue;
        }
        let check = match *strategy {
            VERIFIED_BY_GIT => verify_git(project, action).await,
            VERIFIED_BY_EVENT => verify_event(pool, &project.id, action).await,
            _ => verify_content(project, expect_substring).await,
        };
        if check.passed && verified_by.is_none() {
            verified_by = Some(match *strategy {
                VERIFIED_BY_GIT => VERIFIED_BY_GIT,
                VERIFIED_BY_EVENT => VERIFIED_BY_EVENT,
                _ => VERIFIED_BY_CONTENT,
            });
        }
        checks.push(check);
    }

    if verified_by.is_none() && self_attested {
        checks.push(pass(
            "self_attested",
            "You told me it landed",
            "Nothing could be confirmed automatically, so this is recorded as your word. The card \
             says so, and it is a weaker claim than a commit or a live-page match.",
        ));
        verified_by = Some(VERIFIED_BY_SELF);
    }

    VerifyOutcome {
        verified_by,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_repo_paths_out_of_the_actions_prose() {
        let text = "Open src/pages/index.astro and add an FAQPage block, then update \
                    apps/site/README.md. See https://schema.org/FAQPage for the shape.";
        assert_eq!(
            named_paths(text),
            vec![
                "apps/site/README.md".to_string(),
                "src/pages/index.astro".to_string()
            ]
        );
    }

    /// A check that matches everything confirms nothing, so bare prose and URLs
    /// must not become paths.
    #[test]
    fn prose_and_urls_are_not_treated_as_paths() {
        assert!(named_paths("Rewrite the homepage hero and the about page").is_empty());
        assert!(named_paths("Link to https://example.com/blog/post from the nav").is_empty());
        assert!(named_paths("Improve conversion and/or retention").is_empty());
    }

    /// A two-commit log exactly as `git log --format=%x00%H%x1f%s --name-only`
    /// emits it: a NUL before each record, the sha and subject separated by
    /// U+001F, then the changed files one per line.
    fn fixture_log() -> String {
        concat!(
            "\u{0}8f2a1c3390ab\u{1f}Add FAQ block to the homepage\n",
            "src/pages/index.astro\n",
            "src/components/Faq.astro\n",
            "\u{0}1122334455ff\u{1f}Merge branch 'main'\n",
            "\u{0}aabbccddeeff\u{1f}Bump deps\n",
            "package.json\n"
        )
        .to_string()
    }

    #[test]
    fn parses_the_null_separated_log_into_commits_and_their_files() {
        let commits = parse_commits(&fixture_log());
        assert_eq!(commits.len(), 3);
        assert_eq!(commits[0].sha, "8f2a1c3390ab");
        assert_eq!(commits[0].subject, "Add FAQ block to the homepage");
        assert_eq!(
            commits[0].files,
            vec!["src/pages/index.astro", "src/components/Faq.astro"]
        );
        // A merge commit reports no files. That is a commit that changed nothing
        // on its own, not a parse failure.
        assert_eq!(commits[1].subject, "Merge branch 'main'");
        assert!(commits[1].files.is_empty());
        assert_eq!(commits[2].files, vec!["package.json"]);
        assert!(parse_commits("").is_empty());
        assert!(parse_commits("   \n").is_empty());
    }

    /// The prose names a repo-relative path; the repo reports one relative to
    /// its own root. Both directions of suffix must match or every real
    /// monorepo fails to verify.
    #[test]
    fn a_named_path_matches_the_repos_own_spelling() {
        let commits = parse_commits(&fixture_log());
        let (commit, want, got) =
            touches_named_path(&commits, &["src/pages/index.astro".to_string()]).unwrap();
        assert_eq!(commit.sha, "8f2a1c3390ab");
        assert_eq!(want, "src/pages/index.astro");
        assert_eq!(got, "src/pages/index.astro");

        let monorepo = parse_commits(
            "\u{0}deadbeefcafe\u{1f}Ship it\napps/site/src/pages/index.astro\npackage.json\n",
        );
        let (_, want, got) =
            touches_named_path(&monorepo, &["src/pages/index.astro".to_string()]).unwrap();
        assert_eq!(want, "src/pages/index.astro");
        assert_eq!(got, "apps/site/src/pages/index.astro");
        assert!(touches_named_path(&monorepo, &["src/pages/about.astro".to_string()]).is_none());
    }

    /// REGRESSION. The passing detail used to read "A commit since the action
    /// was issued changed X, which it named as Y" — no sha, no subject, so a
    /// user could not tell WHICH commit was credited, and could not tell a
    /// correct match from a coincidental one. It carried no sha because the
    /// only `git log` that knew the shas was a separate call whose output was
    /// thrown away.
    #[test]
    fn a_passing_git_check_names_the_commit_and_the_path() {
        let commits = parse_commits(&fixture_log());
        let (commit, want, got) =
            touches_named_path(&commits, &["src/pages/index.astro".to_string()]).unwrap();
        // Calls the production formatter. This test used to re-type the
        // `format!` here and assert on its own copy, so deleting `commit.short()`
        // from the real string left it green — the check it claims to cover was
        // never executed by anything.
        let detail = passing_detail(commit, &want, &got);
        assert!(detail.contains("8f2a1c33"), "{detail}");
        assert!(detail.contains("Add FAQ block to the homepage"), "{detail}");
        assert!(detail.contains("src/pages/index.astro"), "{detail}");
        assert!(detail.contains("which the action named as"), "{detail}");
    }

    /// The failure rendering is unchanged except that it now names the shas it
    /// looked through: a failed check still has to say what it saw, or "not
    /// found" is indistinguishable from "not checked".
    #[test]
    fn a_commit_that_touches_nothing_named_still_says_what_it_saw() {
        let commits = parse_commits(&fixture_log());
        let named = vec!["src/pages/pricing.astro".to_string()];
        assert!(touches_named_path(&commits, &named).is_none());
        // As above: the production formatter, not a copy of it.
        let detail = missing_detail(&commits, &named);
        assert!(detail.contains("3 commit(s)"), "{detail}");
        assert!(detail.contains("8f2a1c33"), "{detail}");
        assert!(detail.contains("src/pages/pricing.astro"), "{detail}");
    }

    /// REGRESSION. `verify_content` fails with "No text to look for was given"
    /// unless the caller supplies `expectSubstring`, and no caller does — the
    /// panel posts `targetBody()` or `{...targetBody(), selfAttested: true}`,
    /// and `expectSubstring` appears nowhere in the UI source. So the check ran
    /// on every `post` action and every artifact-less one, and could only ever
    /// come back red. A user cannot tell that from a change that did not ship.
    #[test]
    fn a_check_with_nothing_to_look_for_is_not_run_at_all() {
        // The production guard, not a copy of it: `verify` skips the strategy
        // on exactly this condition.
        for expect in [None, Some(""), Some("   ")] {
            assert!(
                !can_check_content(expect),
                "content must be skipped for {expect:?}"
            );
        }
        assert!(
            can_check_content(Some("FAQPage")),
            "and must still run for a caller that says what to look for"
        );
    }

    /// The third branch, which had no test at all: an action that names no file
    /// path still gets a pass, and that pass must not overclaim. A user who
    /// reads "confirmed" here has only been told a commit exists, not that it
    /// is the right one, and the string is the only place that distinction is
    /// made.
    #[test]
    fn a_pass_with_no_named_path_says_it_only_proves_work_happened() {
        let commits = parse_commits(&fixture_log());
        let detail = untargeted_detail(&commits);
        assert!(detail.contains("3 commit(s)"), "{detail}");
        assert!(detail.contains("8f2a1c33"), "{detail}");
        assert!(
            detail.contains("names no file path"),
            "the weaker claim must be stated: {detail}"
        );
        assert!(
            detail.contains("not that this change is the one that landed"),
            "{detail}"
        );
    }

    /// Generic English is not a marker. "Rewrite the homepage" is the 08-14
    /// action the Steward cannot honestly auto-dismiss: nothing in that prose
    /// is distinctive enough to grep for without matching half the repo.
    #[test]
    fn a_homepage_rewrite_names_no_strong_marker() {
        let text = "Rewrite the homepage (/) to reduce 13-pageview entry bounce and funnel \
                    users to category or search. The homepage does not send entering users \
                    anywhere. Give it category and search entry points.";
        assert!(
            strong_markers(text).is_empty(),
            "a check that matches everything confirms nothing: {:?}",
            strong_markers(text)
        );
        assert!(presence_in_files(
            &strong_markers(text),
            &[("src/pages/index.astro".into(), "<h1>Welcome</h1>".into(),)]
        )
        .is_none());
    }

    /// The other 08-14 action: FAQPage in the tree IS the change.
    #[test]
    fn faqpage_in_a_named_file_is_already_present() {
        let text = "Add structured data (schema.org Event + FAQPage) to event detail pages \
                    in src/pages/events/[slug].astro to enable answer-engine visibility.";
        let markers = strong_markers(text);
        assert!(markers.iter().any(|m| m == "FAQPage"), "{markers:?}");
        let found = presence_in_files(
            &markers,
            &[(
                "src/pages/events/[slug].astro".into(),
                r#"<script type="application/ld+json">{"@type":"FAQPage"}</script>"#.into(),
            )],
        )
        .expect("FAQPage in the named file is the change");
        assert_eq!(found.marker, "FAQPage");
        assert_eq!(found.path, "src/pages/events/[slug].astro");
        assert_eq!(found.detail(), "FAQPage in src/pages/events/[slug].astro");
    }

    #[test]
    fn a_lockfile_hit_is_not_the_change() {
        let markers = vec!["FAQPage".into()];
        assert!(presence_in_files(
            &markers,
            &[("package-lock.json".into(), r#"{"FAQPage": true}"#.into(),)],
        )
        .is_none());
    }

    fn sh(dir: &std::path::Path, cmd: &str) {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "command failed: {cmd}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn project_at(root: &std::path::Path) -> permagent::projects::Project {
        permagent::projects::Project {
            id: "p1".into(),
            user_id: "u".into(),
            slug: "p".into(),
            name: "P".into(),
            description: String::new(),
            status: "active".into(),
            root_path: Some(root.display().to_string()),
            site_url: None,
            repo_url: None,
            notes: String::new(),
            metadata_json: serde_json::json!({}),
            graph_entity_id: None,
            tags: vec![],
            created_at: String::new(),
            updated_at: String::new(),
            last_opened_at: String::new(),
        }
    }

    /// End to end against a real checkout: the files as they are, not commits
    /// since the action was issued. That window is why Review kept re-proposing
    /// work that was already in the tree.
    #[tokio::test]
    async fn already_present_reads_the_current_tree_not_commits_since_the_card() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sh(dir, "git init -q -b main");
        std::fs::create_dir_all(dir.join("src/pages")).unwrap();
        std::fs::write(
            dir.join("src/pages/event.astro"),
            r#"<script type="application/ld+json">{"@type":"FAQPage"}</script>"#,
        )
        .unwrap();
        sh(dir, "git add -A");
        sh(
            dir,
            "git -c user.email=t@t -c user.name=t commit -q -m 'Add FAQPage schema'",
        );

        let project = project_at(dir);
        let text = "Add structured data (schema.org Event + FAQPage) to event detail pages";
        let found = already_present(&project, text)
            .await
            .expect("FAQPage is in HEAD");
        assert_eq!(found.marker, "FAQPage");
        assert!(found.path.ends_with("event.astro"), "{}", found.path);

        // No marker, no dismiss — a rewrite with nothing to grep for.
        assert!(already_present(&project, "Rewrite the homepage hero")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn the_codebase_brief_names_head_and_what_the_steward_dismissed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sh(dir, "git init -q -b main");
        std::fs::write(dir.join("README.md"), "hi\n").unwrap();
        sh(dir, "git add -A");
        sh(
            dir,
            "git -c user.email=t@t -c user.name=t commit -q -m 'Add FAQPage schema'",
        );
        let project = project_at(dir);
        let text = render_codebase_brief(
            &project,
            &[DismissedPresence {
                title: "Add FAQPage schema".into(),
                detail: "FAQPage in src/pages/event.astro".into(),
            }],
        )
        .await
        .expect("a repo produces a brief");
        assert!(text.contains("already in the tree"), "{text}");
        assert!(text.contains("Add FAQPage schema"), "{text}");
        assert!(text.contains("The Steward dismissed"), "{text}");
        assert!(text.contains("FAQPage in src/pages/event.astro"), "{text}");
    }
}
