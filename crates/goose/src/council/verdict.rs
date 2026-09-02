//! Machine-parseable chair verdict lines.
//!
//! Adapted from the Omnia Vault review (gavishap/omnia-vault, 2026-08-28): its
//! war-room and sparring loops end every deliberation with one fixed-marker,
//! constrained-value line (`INTEL VERDICT: INCORPORATE | WATCHLIST | PASS — …`,
//! `VERDICT: APPROVED|REVISE`), and — the half that makes it real — tooling
//! that *nags* when the line is absent. Their `parse_verdict` scans upward from
//! the bottom over the last few lines, tolerating trailing sign-offs, and
//! reports `NO-VERDICT-LINE` rather than shrugging. Failing closed is the point.
//!
//! Here the marker rides the chair's existing markdown, so nothing about the
//! stored schema changes: a report either ends with
//!
//! ```text
//! VERDICT: ACT — file the two homepage cards this week
//! ```
//!
//! or it carries the [`NO_VERDICT_FLAG`] line saying, in the record itself,
//! that the chair declined to rule. What we deliberately do NOT build here is
//! the full typed ChairAction schema — that is Council-redesign Stage 2.

/// The fixed marker. Case-insensitive on read, canonical on write.
pub const VERDICT_MARKER: &str = "VERDICT:";

/// The line appended to a report when the chair produced no parseable verdict,
/// even after one re-ask. Visible to the human, detectable by the parser (it is
/// deliberately NOT a verdict line, so re-parsing a stored report still fails).
pub const NO_VERDICT_FLAG: &str =
    "NO VERDICT LINE — the chair did not state a machine-parseable verdict \
     (VERDICT: ACT|WATCH|HOLD) after one re-ask. Treat the ruling as unresolved.";

/// How many non-empty trailing lines the parser will look through. Matches the
/// Omnia window: enough to tolerate a sign-off or a closing bullet, small
/// enough that a verdict buried mid-report is reported as misplaced, not found.
pub const TAIL_LINES: usize = 8;

/// The constrained value. Three options, because a weekly ruling that cannot
/// be reduced to one of these is exactly the "it depends" the Chair is
/// forbidden from smoothing into mush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// There is work worth starting now; the filed actions are real.
    Act,
    /// Nothing to start; keep the named thing under observation.
    Watch,
    /// Actively stop or defer — doing less this week is the ruling.
    Hold,
}

impl Verdict {
    pub const ALL: [Verdict; 3] = [Verdict::Act, Verdict::Watch, Verdict::Hold];

    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Act => "ACT",
            Verdict::Watch => "WATCH",
            Verdict::Hold => "HOLD",
        }
    }

    /// Strict token match, case-insensitive. `ACTION` is not `ACT`.
    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim().to_ascii_uppercase().as_str() {
            "ACT" => Some(Verdict::Act),
            "WATCH" => Some(Verdict::Watch),
            "HOLD" => Some(Verdict::Hold),
            _ => None,
        }
    }

    /// The allowed values, for prompts and nag text.
    pub fn allowed() -> String {
        Verdict::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// A parsed verdict line: the ruling plus the one-line reason after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChairVerdict {
    pub verdict: Verdict,
    /// Everything after the value, separator-stripped. May be empty.
    pub rationale: String,
}

impl ChairVerdict {
    /// The canonical single line, as written back into a report.
    pub fn render(&self) -> String {
        if self.rationale.is_empty() {
            format!("{VERDICT_MARKER} {}", self.verdict.as_str())
        } else {
            format!(
                "{VERDICT_MARKER} {} — {}",
                self.verdict.as_str(),
                self.rationale
            )
        }
    }
}

/// Why a verdict could not be read. Both variants are nag-worthy; neither is
/// ever silently downgraded to "fine".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictProblem {
    /// No line carrying the marker anywhere in the text.
    Missing,
    /// A marker line exists but does not parse, or sits outside the tail window.
    Malformed { line: String, reason: String },
}

impl VerdictProblem {
    /// One short human/model-readable sentence for the record.
    pub fn describe(&self) -> String {
        match self {
            VerdictProblem::Missing => "no VERDICT line".to_string(),
            VerdictProblem::Malformed { line, reason } => {
                format!("malformed VERDICT line ({reason}): {line}")
            }
        }
    }

    /// The nag: the re-ask sent back to the chair. Deliberately tiny — it asks
    /// for one line and nothing else, so a non-compliant chair costs one cheap
    /// call, not a second full synthesis.
    pub fn nag(&self) -> String {
        format!(
            "Your report is missing its ruling: {}. Reply with EXACTLY ONE line and nothing \
             else, in this form:\n{VERDICT_MARKER} {} — <one sentence>\nPick one value. Do not \
             restate the report. Do not hedge.",
            self.describe(),
            Verdict::allowed()
        )
    }
}

/// Strip markdown decoration a model may wrap the line in: list bullets,
/// heading hashes, block quotes, bold/italic runs, and trailing backticks.
fn undecorate(line: &str) -> String {
    let mut s = line.trim();
    loop {
        let before = s;
        s = s
            .trim_start_matches(['-', '*', '+', '#', '>', '`'])
            .trim_start();
        s = s.trim_end_matches(['`', '*', '_']).trim_end();
        if s == before {
            break;
        }
    }
    s.trim_matches('*').trim().to_string()
}

fn marker_body(line: &str) -> Option<&str> {
    let cleaned = line.trim_start();
    if cleaned.len() < VERDICT_MARKER.len() {
        return None;
    }
    let (head, rest) = cleaned.split_at(VERDICT_MARKER.len());
    head.eq_ignore_ascii_case(VERDICT_MARKER).then_some(rest)
}

/// Parse the chair's verdict line out of a report.
///
/// Scans upward from the bottom over the last [`TAIL_LINES`] non-empty lines.
/// A marker line found earlier in the document is reported as
/// [`VerdictProblem::Malformed`] (misplaced), never quietly accepted — the
/// verdict is supposed to be the last word.
pub fn parse(text: &str) -> Result<ChairVerdict, VerdictProblem> {
    let non_empty: Vec<String> = text
        .lines()
        .map(undecorate)
        .filter(|l| !l.is_empty())
        .collect();
    let window_start = non_empty.len().saturating_sub(TAIL_LINES);

    for (idx, line) in non_empty.iter().enumerate().rev() {
        let Some(body) = marker_body(line) else {
            continue;
        };
        if idx < window_start {
            return Err(VerdictProblem::Malformed {
                line: line.clone(),
                reason: format!("the verdict must be within the last {TAIL_LINES} lines"),
            });
        }
        return read_body(line, body);
    }
    Err(VerdictProblem::Missing)
}

fn read_body(line: &str, body: &str) -> Result<ChairVerdict, VerdictProblem> {
    let body = body.trim();
    if body.is_empty() {
        return Err(VerdictProblem::Malformed {
            line: line.to_string(),
            reason: format!("no value; expected one of {}", Verdict::allowed()),
        });
    }
    // The value is the first separator-delimited word: whitespace, an em/en
    // dash, a colon, a pipe or a comma all end it.
    let end = body
        .find(|c: char| c.is_whitespace() || matches!(c, '—' | '–' | ':' | '|' | ','))
        .unwrap_or(body.len());
    let (token, rest) = body.split_at(end);
    let Some(verdict) = Verdict::from_token(token) else {
        return Err(VerdictProblem::Malformed {
            line: line.to_string(),
            reason: format!(
                "\"{token}\" is not a verdict; expected one of {}",
                Verdict::allowed()
            ),
        });
    };
    let rationale = rest
        .trim_start_matches(|c: char| {
            c.is_whitespace() || matches!(c, '—' | '–' | '-' | ':' | '|' | ',')
        })
        .trim()
        .to_string();
    Ok(ChairVerdict { verdict, rationale })
}

/// The one-line instruction appended to the chair's system prompt.
pub fn prompt_clause() -> String {
    format!(
        "The LAST line of `markdown` MUST be a machine-parseable ruling, exactly: \
         `{VERDICT_MARKER} {} — <one sentence>`. Pick one value, no hedging; a report without \
         this line is sent back.",
        Verdict::allowed()
    )
}

/// How a verdict reads in a rendered report — the honest line either way.
pub fn render_line(text: &str) -> String {
    match parse(text) {
        Ok(v) => format!("Verdict: {} — {}", v.verdict.as_str(), v.rationale),
        Err(problem) => format!("Verdict: NOT STATED ({})", problem.describe()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_canonical_line() {
        let v = parse("# Report\n\nbody\n\nVERDICT: ACT — file the two homepage cards").unwrap();
        assert_eq!(v.verdict, Verdict::Act);
        assert_eq!(v.rationale, "file the two homepage cards");
        assert_eq!(v.render(), "VERDICT: ACT — file the two homepage cards");
    }

    #[test]
    fn tolerates_decoration_case_and_a_trailing_signoff() {
        let v = parse("body\n\n**verdict: hold - do less this week**\n\n_— the Chair_").unwrap();
        assert_eq!(v.verdict, Verdict::Hold);
        assert_eq!(v.rationale, "do less this week");

        let v = parse("- VERDICT: WATCH").unwrap();
        assert_eq!(v.verdict, Verdict::Watch);
        assert_eq!(v.rationale, "");
    }

    #[test]
    fn missing_marker_is_missing_not_ok() {
        assert_eq!(
            parse("all good, ship it").unwrap_err(),
            VerdictProblem::Missing
        );
        assert_eq!(parse("").unwrap_err(), VerdictProblem::Missing);
    }

    #[test]
    fn unconstrained_values_are_malformed_not_accepted() {
        for bad in [
            "VERDICT: MAYBE — who knows",
            "VERDICT: ACTION — do it",
            "VERDICT:",
        ] {
            let err = parse(bad).unwrap_err();
            assert!(
                matches!(err, VerdictProblem::Malformed { .. }),
                "{bad} must be malformed, got {err:?}"
            );
            assert!(err.nag().contains("ACT|WATCH|HOLD"));
        }
    }

    #[test]
    fn a_verdict_buried_above_the_tail_window_is_misplaced() {
        let mut text = String::from("VERDICT: ACT — early and wrong\n");
        for i in 0..TAIL_LINES + 2 {
            text.push_str(&format!("filler line {i}\n"));
        }
        let err = parse(&text).unwrap_err();
        match err {
            VerdictProblem::Malformed { ref reason, .. } => {
                assert!(reason.contains("last 8 lines"), "{reason}")
            }
            other => panic!("expected misplaced, got {other:?}"),
        }
    }

    #[test]
    fn the_last_verdict_wins_when_a_chair_repeats_itself() {
        let v = parse("VERDICT: WATCH — first\nnoise\nVERDICT: HOLD — final").unwrap();
        assert_eq!(v.verdict, Verdict::Hold);
        assert_eq!(v.rationale, "final");
    }

    #[test]
    fn render_line_is_honest_in_both_directions() {
        assert_eq!(render_line("VERDICT: ACT — go"), "Verdict: ACT — go");
        assert!(render_line("nothing here").starts_with("Verdict: NOT STATED"));
        assert!(render_line(NO_VERDICT_FLAG).starts_with("Verdict: NOT STATED"));
    }

    #[test]
    fn the_flag_line_is_not_itself_a_verdict() {
        // Critical: the flag must never satisfy the parser, or a flagged report
        // would read back as a ruling.
        assert!(parse(&format!("body\n\n{NO_VERDICT_FLAG}")).is_err());
    }

    #[test]
    fn prompt_clause_names_the_marker_and_the_values() {
        let clause = prompt_clause();
        assert!(clause.contains(VERDICT_MARKER));
        assert!(clause.contains("ACT|WATCH|HOLD"));
    }
}
