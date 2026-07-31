//! Install verification — the loud failure signal analytics does not have.
//!
//! Every failure mode of a relay install is silent: fire-and-forget beacons,
//! 202 responses, empty catch blocks, and a 401 that looks identical whether
//! the key is wrong, unset, or set on the wrong service. On the first real
//! install that combination cost roughly three hours, and the brief's prose
//! verification steps — any one of which would have caught it — were skipped by
//! the agent, which then reported success.
//!
//! So the check is a single call that returns pass/fail per assertion, run
//! against the DEPLOYED origin from the machine that will do the draining:
//!
//!   POST /api/projects/{id}/analytics/first_party/verify { "origin": "https://…" }
//!
//! It deliberately does NOT trust the site's own report. It fetches the served
//! HTML, inspects the CSP, posts a real beacon, and exercises the drain with a
//! correct and an incorrect key.

use crate::routes::first_party_analytics::{COLLECT_PATH, DRAIN_PATH};
use serde::Serialize;

/// One assertion's outcome. `detail` always explains a failure in terms of what
/// to change, never just what was observed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: &'static str,
    pub label: &'static str,
    pub passed: bool,
    pub detail: String,
}

impl Check {
    fn pass(id: &'static str, label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            id,
            label,
            passed: true,
            detail: detail.into(),
        }
    }
    fn fail(id: &'static str, label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            id,
            label,
            passed: false,
            detail: detail.into(),
        }
    }
}

/// Does this HTML carry our snippet, pointing at the relative collect path?
///
/// Checks for the path rather than the whole script because the agent is
/// allowed to reformat, and for an absolute host because that is the specific
/// substitution seen in the wild — `http://127.0.0.1:3001/...`, which beacons
/// to each VISITOR'S own machine and is mixed-content-blocked besides.
pub fn inspect_html(html: &str) -> Result<(), String> {
    if !html.contains(COLLECT_PATH) {
        return Err(format!(
            "the snippet is missing — no reference to {COLLECT_PATH} in the served HTML. \
             If the app prerenders, check that post-processing is not stripping <script> tags."
        ));
    }
    // An absolute URL immediately before the collect path means the endpoint was
    // rewritten to a host.
    for scheme in ["http://", "https://"] {
        if let Some(idx) = html.find(scheme) {
            // `find` returns a char boundary, so these slices cannot split a
            // character — but take them through `get` so the invariant is
            // enforced rather than assumed. This parses HTML from arbitrary
            // sites, where multi-byte characters are a certainty.
            let Some(tail) = html.get(idx..) else {
                continue;
            };
            if let Some(end) = tail.find(COLLECT_PATH) {
                // Only flag when the absolute URL is the endpoint itself
                // (no intervening quote or tag boundary).
                let Some(between) = tail.get(..end) else {
                    continue;
                };
                if !between.contains('"') && !between.contains('\'') && !between.contains('<') {
                    // Truncate the preview by CHARACTERS. The previous
                    // `&tail[..between.len().min(48)]` sliced by BYTE count and
                    // panicked outright on any multi-byte character inside the
                    // first 48 bytes — an em-dash or non-ASCII domain in real
                    // served HTML was enough to take the verify route down.
                    let preview: String = between.chars().take(48).collect();
                    return Err(format!(
                        "the collect endpoint was rewritten to an absolute URL ({preview}…). It MUST \
                         stay the relative path {COLLECT_PATH}: an absolute host beacons to the \
                         visitor's own machine and is blocked as mixed content from HTTPS."
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Would this CSP block an inline script?
///
/// Returns Ok when there is no policy at all, when `unsafe-inline` is present,
/// or when a hash/nonce is present (we cannot verify WHICH hash without
/// executing, so a present hash is treated as deliberate). The failure case is
/// the one that actually bit: a script-src with neither.
pub fn inspect_csp(csp: Option<&str>) -> Result<(), String> {
    let Some(policy) = csp else { return Ok(()) };
    let lower = policy.to_ascii_lowercase();
    // Find script-src, falling back to default-src, which also governs scripts.
    let directive = lower
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("script-src"))
        .or_else(|| {
            lower
                .split(';')
                .map(str::trim)
                .find(|d| d.starts_with("default-src"))
        });
    let Some(directive) = directive else {
        return Ok(());
    };
    if directive.contains("'unsafe-inline'")
        || directive.contains("'sha256-")
        || directive.contains("'nonce-")
    {
        return Ok(());
    }
    Err(format!(
        "Content-Security-Policy would block the inline snippet — `{}` permits neither a hash \
         nor a nonce nor 'unsafe-inline'. This produces ZERO events with no error anywhere. Add \
         a sha256 hash derived from the built HTML at runtime (not a hardcoded literal), or a \
         nonce, or serve the snippet as a first-party .js file.",
        directive.trim()
    ))
}

/// Interpret a drain response. The 401 case is where the guidance matters:
/// a wrong key and a key set on the wrong service are indistinguishable over
/// the wire, so the fix list has to be enumerated rather than guessed.
pub fn interpret_drain(status: u16, body_is_json: bool) -> Result<(), String> {
    match status {
        200 if body_is_json => Ok(()),
        200 => Err("drain returned 200 but the body is not JSON — it must be \
                    { \"events\": [...] }"
            .to_string()),
        401 | 403 => Err(
            "drain rejected the key (401). In order of likelihood: PERMAGENT_ANALYTICS_KEY is \
             set on the DATABASE service instead of the service running the app; the service \
             was not redeployed after the variable was added; the value does not match; or the \
             variable is unset and the route is correctly failing closed."
                .to_string(),
        ),
        404 => Err(format!(
            "drain route not found at {DRAIN_PATH} — it may be registered after an SPA \
             catch-all, which would swallow it."
        )),
        other => Err(format!("drain returned {other}")),
    }
}

/// A wrong key MUST be rejected. A drain that answers 200 to anything is an
/// open data endpoint.
pub fn interpret_wrong_key(status: u16) -> Result<(), String> {
    if status == 401 || status == 403 {
        Ok(())
    } else {
        Err(format!(
            "drain returned {status} for a WRONG key — it must fail closed with 401, or anyone \
             can read your analytics."
        ))
    }
}

/// Exactly one pageview per load is the assertion that catches the
/// double-counting class of bug.
pub fn interpret_pageview_delta(delta: i64) -> Result<(), String> {
    match delta {
        1 => Ok(()),
        0 => Err(
            "the beacon was accepted but no row appeared — the collector is not writing \
                  to the table the drain reads from."
                .to_string(),
        ),
        n if n > 1 => Err(format!(
            "one page load produced {n} pageviews. The snippet is counting twice — usually a \
             missing pathname dedupe with both pushState and replaceState hooked, or the \
             snippet injected on more than one layout."
        )),
        n => Err(format!("pageview count went backwards ({n})")),
    }
}

/// Overall verdict: every check must pass.
pub fn verdict(checks: &[Check]) -> bool {
    !checks.is_empty() && checks.iter().all(|c| c.passed)
}

/// Human-readable summary, for the agent to paste back and the user to read.
pub fn summarize(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        out.push_str(if c.passed { "PASS  " } else { "FAIL  " });
        out.push_str(c.label);
        if !c.passed {
            out.push_str("\n      ");
            out.push_str(&c.detail);
        }
        out.push('\n');
    }
    out.push_str(if verdict(checks) {
        "\nAll checks passed — this install is live."
    } else {
        "\nNOT verified. Fix the failures above and re-run."
    });
    out
}

/// Build the check list from raw observations. Pure, so the whole decision
/// table is testable without a network.
#[allow(clippy::too_many_arguments)]
pub fn build_checks(
    html_by_route: &[(String, Result<String, String>)],
    csp: Option<&str>,
    beacon_status: Option<u16>,
    pageview_delta: Option<i64>,
    drain: Option<(u16, bool)>,
    wrong_key_status: Option<u16>,
) -> Vec<Check> {
    let mut checks = Vec::new();

    // The snippet must be on MORE THAN ONE route: injecting it into a single
    // page is a common partial install that looks fine on the home page.
    let mut html_ok = true;
    let mut html_detail = String::new();
    for (route, html) in html_by_route {
        match html {
            Err(e) => {
                html_ok = false;
                html_detail.push_str(&format!("{route}: could not fetch ({e}). "));
            }
            Ok(body) => {
                if let Err(e) = inspect_html(body) {
                    html_ok = false;
                    html_detail.push_str(&format!("{route}: {e} "));
                }
            }
        }
    }
    if html_by_route.len() < 2 {
        html_detail.push_str("checked fewer than two routes. ");
    }
    checks.push(if html_ok {
        Check::pass(
            "snippet",
            "Snippet present on every checked route, with a relative endpoint",
            format!("{} routes checked", html_by_route.len()),
        )
    } else {
        Check::fail(
            "snippet",
            "Snippet present on every checked route",
            html_detail,
        )
    });

    checks.push(match inspect_csp(csp) {
        Ok(()) => Check::pass(
            "csp",
            "Content-Security-Policy permits the snippet",
            csp.map(|_| "policy present and permissive for this script")
                .unwrap_or("no policy set")
                .to_string(),
        ),
        Err(e) => Check::fail("csp", "Content-Security-Policy permits the snippet", e),
    });

    checks.push(match beacon_status {
        Some(202) => Check::pass("beacon", "Collector accepts a beacon", "202"),
        Some(s) => Check::fail(
            "beacon",
            "Collector accepts a beacon",
            format!(
                "POST {COLLECT_PATH} returned {s}, expected 202. A 404 usually means the route \
                 is registered after the SPA catch-all; a 415 means the body parser rejects \
                 sendBeacon's text/plain content type."
            ),
        ),
        None => Check::fail("beacon", "Collector accepts a beacon", "not attempted"),
    });

    checks.push(match pageview_delta.map(interpret_pageview_delta) {
        Some(Ok(())) => Check::pass("count", "One page load records exactly one pageview", "+1"),
        Some(Err(e)) => Check::fail("count", "One page load records exactly one pageview", e),
        None => Check::fail(
            "count",
            "One page load records exactly one pageview",
            "not attempted",
        ),
    });

    checks.push(match drain.map(|(s, j)| interpret_drain(s, j)) {
        Some(Ok(())) => Check::pass("drain", "Drain returns JSON with the key", "200"),
        Some(Err(e)) => Check::fail("drain", "Drain returns JSON with the key", e),
        None => Check::fail("drain", "Drain returns JSON with the key", "not attempted"),
    });

    checks.push(match wrong_key_status.map(interpret_wrong_key) {
        Some(Ok(())) => Check::pass("drain_auth", "Drain rejects a wrong key", "401"),
        Some(Err(e)) => Check::fail("drain_auth", "Drain rejects a wrong key", e),
        None => Check::fail("drain_auth", "Drain rejects a wrong key", "not attempted"),
    });

    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html_with_snippet() -> String {
        format!("<html><head><script>var E=\"{COLLECT_PATH}\";</script></head></html>")
    }

    #[test]
    fn accepts_a_correct_install() {
        assert!(inspect_html(&html_with_snippet()).is_ok());
    }

    #[test]
    fn catches_a_missing_snippet() {
        let err = inspect_html("<html><head></head></html>").unwrap_err();
        assert!(err.contains("missing"), "{err}");
        // Names the prerender hazard, because that is a real way it goes missing.
        assert!(err.contains("prerender"), "{err}");
    }

    /// The exact substitution seen in the wild.
    #[test]
    fn catches_an_absolute_endpoint() {
        let html = format!("<script>var E=\"http://127.0.0.1:3001{COLLECT_PATH}\";</script>");
        let err = inspect_html(&html).unwrap_err();
        assert!(err.contains("absolute"), "{err}");
        assert!(err.contains("mixed content"), "{err}");
    }

    #[test]
    fn a_site_wide_https_url_elsewhere_is_not_flagged() {
        // A canonical link or asset URL must not trip the absolute-endpoint check.
        let html = format!(
            "<link rel=\"canonical\" href=\"https://example.com/\"><script>var E=\"{COLLECT_PATH}\";</script>"
        );
        assert!(inspect_html(&html).is_ok());
    }

    // ── CSP ──

    #[test]
    fn no_csp_is_fine() {
        assert!(inspect_csp(None).is_ok());
    }

    /// THE case: helmet's default. It produced zero events and no error.
    #[test]
    fn catches_the_policy_that_blocked_the_first_real_install() {
        let err = inspect_csp(Some("default-src 'self'; script-src 'self'")).unwrap_err();
        assert!(err.contains("ZERO events"), "{err}");
        assert!(err.contains("hash"), "{err}");
        assert!(
            !err.contains("hardcoded literal") || err.contains("not a hardcoded"),
            "{err}"
        );
    }

    #[test]
    fn accepts_a_hash_a_nonce_or_unsafe_inline() {
        for policy in [
            "script-src 'self' 'sha256-abc123='",
            "script-src 'self' 'nonce-r4nd0m'",
            "script-src 'self' 'unsafe-inline'",
        ] {
            assert!(inspect_csp(Some(policy)).is_ok(), "{policy}");
        }
    }

    #[test]
    fn falls_back_to_default_src_when_there_is_no_script_src() {
        assert!(inspect_csp(Some("default-src 'self'")).is_err());
        assert!(inspect_csp(Some("default-src 'self' 'unsafe-inline'")).is_ok());
    }

    #[test]
    fn a_policy_without_script_governance_is_fine() {
        assert!(inspect_csp(Some("img-src 'self'; style-src 'self'")).is_ok());
    }

    // ── drain ──

    #[test]
    fn drain_401_enumerates_the_causes_it_cannot_distinguish() {
        let err = interpret_drain(401, false).unwrap_err();
        // The wrong-service case cost the most time on the first install, so it
        // is named first.
        assert!(err.contains("DATABASE service"), "{err}");
        assert!(err.contains("redeployed"), "{err}");
    }

    #[test]
    fn drain_404_names_the_catch_all_hazard() {
        assert!(interpret_drain(404, false)
            .unwrap_err()
            .contains("catch-all"));
    }

    #[test]
    fn drain_must_reject_a_wrong_key() {
        assert!(interpret_wrong_key(401).is_ok());
        let err = interpret_wrong_key(200).unwrap_err();
        assert!(err.contains("anyone can read"), "{err}");
    }

    // ── pageview count ──

    #[test]
    fn catches_the_double_count() {
        let err = interpret_pageview_delta(2).unwrap_err();
        assert!(err.contains("counting twice"), "{err}");
        assert!(err.contains("dedupe"), "{err}");
    }

    #[test]
    fn catches_a_beacon_that_writes_nowhere() {
        assert!(interpret_pageview_delta(0)
            .unwrap_err()
            .contains("no row appeared"));
    }

    #[test]
    fn one_is_the_only_pass() {
        assert!(interpret_pageview_delta(1).is_ok());
    }

    // ── assembly ──

    #[test]
    fn a_fully_correct_install_verifies() {
        let checks = build_checks(
            &[
                ("/".into(), Ok(html_with_snippet())),
                ("/about".into(), Ok(html_with_snippet())),
            ],
            None,
            Some(202),
            Some(1),
            Some((200, true)),
            Some(401),
        );
        assert!(verdict(&checks), "{}", summarize(&checks));
    }

    /// The first real install: snippet present, everything "looked right", and
    /// the CSP silently blocked it. Verification must fail loudly.
    #[test]
    fn the_first_real_install_fails_verification() {
        let checks = build_checks(
            &[
                ("/".into(), Ok(html_with_snippet())),
                ("/deals".into(), Ok(html_with_snippet())),
            ],
            Some("script-src 'self'"),
            Some(202),
            Some(2), // and it was double-counting too
            Some((401, false)),
            Some(401),
        );
        assert!(!verdict(&checks));
        let summary = summarize(&checks);
        assert!(summary.contains("NOT verified"), "{summary}");
        // All three real defects are named in one pass, which is the point.
        assert!(summary.contains("Content-Security-Policy"), "{summary}");
        assert!(summary.contains("counting twice"), "{summary}");
        assert!(summary.contains("DATABASE service"), "{summary}");
    }

    #[test]
    fn a_single_route_check_is_not_enough() {
        let checks = build_checks(
            &[("/".into(), Ok(html_with_snippet()))],
            None,
            Some(202),
            Some(1),
            Some((200, true)),
            Some(401),
        );
        // Passes the assertions it could make, but says so.
        let snippet = checks.iter().find(|c| c.id == "snippet").unwrap();
        assert!(snippet.detail.contains('1'), "{}", snippet.detail);
    }

    #[test]
    fn an_empty_check_list_is_never_a_pass() {
        assert!(!verdict(&[]));
    }
}
