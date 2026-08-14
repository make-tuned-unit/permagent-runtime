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
    GrowthActionRow, VERIFIED_BY_CONTENT, VERIFIED_BY_EVENT, VERIFIED_BY_GIT, VERIFIED_BY_SELF,
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

/// Did any commit change a file the action named?
///
/// Matching is suffix-based in both directions so `src/pages/index.astro` in the
/// prose matches `apps/site/src/pages/index.astro` in the repo, and a bare
/// `index.astro` matches too.
fn touches_named_path(changed: &[String], named: &[String]) -> Option<(String, String)> {
    for want in named {
        for got in changed {
            if got == want || got.ends_with(&format!("/{want}")) || want.ends_with(got.as_str()) {
                return Some((want.clone(), got.clone()));
            }
        }
    }
    None
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
    let Some(subjects) = git(root, &["log", &since, "--format=%s"]).await else {
        return fail(ID, LABEL, "git log failed, so commits could not be read.");
    };
    let commits: Vec<&str> = subjects.lines().filter(|l| !l.trim().is_empty()).collect();
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
        return pass(
            ID,
            LABEL,
            format!(
                "{} commit(s) since the action was issued, most recently \"{}\". The action names \
                 no file path, so this confirms work happened here, not that this change is the \
                 one that landed.",
                commits.len(),
                commits[0]
            ),
        );
    }

    let changed = git(root, &["log", &since, "--name-only", "--format="])
        .await
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    match touches_named_path(&changed, &named) {
        Some((want, got)) => pass(
            ID,
            LABEL,
            format!(
                "A commit since the action was issued changed {got}, which it named as {want}."
            ),
        ),
        None => fail(
            ID,
            LABEL,
            format!(
                "{} commit(s) landed since the action was issued, but none touched what it named \
                 ({}). Commit the change, or verify another way.",
                commits.len(),
                named.join(", ")
            ),
        ),
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
            )
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

    /// The prose names a repo-relative path; the repo reports one relative to
    /// its own root. Both directions of suffix must match or every real
    /// monorepo fails to verify.
    #[test]
    fn a_named_path_matches_the_repos_own_spelling() {
        let changed = vec![
            "apps/site/src/pages/index.astro".to_string(),
            "package.json".to_string(),
        ];
        assert_eq!(
            touches_named_path(&changed, &["src/pages/index.astro".to_string()]),
            Some((
                "src/pages/index.astro".to_string(),
                "apps/site/src/pages/index.astro".to_string()
            ))
        );
        assert!(touches_named_path(&changed, &["src/pages/about.astro".to_string()]).is_none());
    }
}
