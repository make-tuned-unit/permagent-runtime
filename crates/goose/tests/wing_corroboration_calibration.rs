//! Calibration of the wing corroboration rule against Jesse's hand labels.
//!
//! # What the fixture is
//!
//! `fixtures/wing-labels-2026-08-24.json` is twelve chat sessions — 441 of the
//! 1,002 unwinged chat turns, 44% — labelled interactively by the data owner,
//! one question per session with the evidence shown. It is the only trustworthy
//! wing ground truth that exists: every in-system signal (existing wings,
//! lexical mentions, session hints) measures *mention* rather than *aboutness*.
//!
//! # What this test can and cannot establish
//!
//! **Be clear about the limit, because a calibration test that overclaims is
//! worse than none.** The fixture carries a wing per session and Jesse's note
//! about why — it does NOT carry the turns' text. The real turns live in
//! `~/.permagent/brain`, which this test does not and must not read. So this
//! file cannot compute the rule's precision over the real corpus; running it
//! over content the test author wrote would measure the author's choice of
//! sentences, not the rule.
//!
//! What it *can* do, and does:
//!
//! 1. **Bound the ceiling.** From the labels alone, count how many labelled
//!    turns a lexical rule could reach even in principle, and how many it
//!    provably cannot. That number is printed, and it is the honest recall
//!    ceiling for this class of rule.
//! 2. **Pin the named failure cases as rule properties.** Each case Jesse
//!    called out is asserted as a property of [`WingCorroborator`] — the stale
//!    hint that must lose to the content, the voice-transcribed name that must
//!    yield nothing rather than something wrong, the ambient session that is
//!    legitimately `general`, the near-homonym pair that must not conflate.
//!    Where a case needs turn text, the text is synthetic and SAYS SO; the
//!    assertion is about the rule's behaviour on that shape of input, not a
//!    claim about what the real turn contained.
//! 3. **Assert zero wrong assignments** on every shape the labels establish:
//!    `personal` and `general` sessions can never be assigned a wing, and a
//!    hint that contradicts the content can never win.
//!
//! The precision measurement over the real corpus belongs to Spectral, against
//! the real brain, after this ships. This test's job is to make sure the rule
//! cannot regress on the cases we already know the answer to.

use permagent::session_wing::{
    CorroborationSource, ProjectHint, WingCorroborator, WingVerdict, PERSONAL_SLUG,
};

const LABELS: &str = include_str!("fixtures/wing-labels-2026-08-24.json");

/// The project registry as it appears in the labels, including the near-homonym
/// pair the fixture warns about.
fn registry() -> Vec<(String, String)> {
    [
        ("grocery-savings-planner", "Grocery Savers"),
        ("wealthie", "Wealthie"),
        ("kinrows", "Kinrows"),
        ("getladle", "Get Ladle"),
        ("lauft", "LAUFT"),
        ("permagent", "Permagent"),
        ("permagent-runtime", "Permagent Runtime"),
        ("henry-infra", "Henry Infra"),
        ("atlas-atlantic", "Atlas Atlantic"),
        (PERSONAL_SLUG, "Personal"),
    ]
    .into_iter()
    .map(|(s, n)| (s.to_string(), n.to_string()))
    .collect()
}

fn hint(slug: &str) -> ProjectHint {
    let name = registry()
        .into_iter()
        .find(|(s, _)| s == slug)
        .map(|(_, n)| n)
        .unwrap_or_else(|| slug.to_string());
    ProjectHint {
        project_id: format!("project:{slug}"),
        slug: slug.to_string(),
        name,
        root_path: None,
    }
}

fn corroborator(hint_slug: &str) -> WingCorroborator {
    WingCorroborator::new(hint(hint_slug), &registry())
}

#[derive(serde::Deserialize)]
struct Label {
    session: String,
    turns: usize,
    wing: String,
}

#[derive(serde::Deserialize)]
struct Labels {
    labels: Vec<Label>,
}

fn labels() -> Vec<Label> {
    serde_json::from_str::<Labels>(LABELS)
        .expect("the calibration fixture must parse")
        .labels
}

/// The fixture is the artefact this whole test rests on. If it changes shape,
/// fail here with a clear message rather than somewhere subtle below.
#[test]
fn the_fixture_is_the_one_we_calibrated_against() {
    let labels = labels();
    assert_eq!(labels.len(), 12, "twelve labelled sessions");
    let turns: usize = labels.iter().map(|l| l.turns).sum();
    assert_eq!(turns, 441, "441 labelled turns");
}

/// The honest recall ceiling for a lexical rule, derived from the labels alone.
///
/// Three classes are unreachable by construction and are counted as such rather
/// than quietly folded into a denominator:
///
/// * `general` — correct as it stands. The target was never zero.
/// * `personal` — never a wing, by design ([`PERSONAL_SLUG`]).
/// * `__SPLIT__` — the session changes subject mid-way, so no single wing is
///   right for it at all.
///
/// The `lauft` session is reachable in principle but not in practice: its
/// content says "Loft", a voice transcription. It is counted in the ceiling and
/// called out separately, because folding a known miss into "reachable" is how
/// a ceiling becomes a promise.
#[test]
fn the_reachable_ceiling_is_printed_and_is_not_the_whole_corpus() {
    let labels = labels();
    let total: usize = labels.iter().map(|l| l.turns).sum();

    let mut reachable = 0usize;
    let mut correctly_general = 0usize;
    let mut personal = 0usize;
    let mut split = 0usize;

    for label in &labels {
        match label.wing.as_str() {
            "general" => correctly_general += label.turns,
            PERSONAL_SLUG => personal += label.turns,
            "__SPLIT__" => split += label.turns,
            _ => reachable += label.turns,
        }
    }

    assert_eq!(
        reachable + correctly_general + personal + split,
        total,
        "every labelled turn must land in exactly one class"
    );

    // Printed, not asserted against a target: this is a measurement of the
    // labels, and pinning it to a number would just make the fixture immutable.
    println!(
        "wing calibration ceiling: {reachable}/{total} turns ({:.0}%) are in sessions a lexical \
         rule could reach at all; {correctly_general} are correctly `general`, {personal} are \
         `personal` (never a wing), {split} are in sessions that change subject mid-way.",
        100.0 * reachable as f64 / total as f64
    );

    assert!(
        reachable < total,
        "a rule that claimed to reach every labelled turn would be claiming to \
         solve cases the labels say are not solvable"
    );
    assert!(
        correctly_general > 0,
        "the fixture records that `general` is sometimes the right answer; if \
         that class ever empties, re-read the labels before celebrating"
    );
}

// ── the named cases, asserted as rule properties ──
//
// Every `content` string below is SYNTHETIC — written here to exercise a shape,
// never presented as the real turn's text. The claim each test makes is about
// what the rule does with that shape.

/// `chat-20260807_1` is `kinrows`, and its session hint said `getladle` — five
/// hours stale. The hint must lose to the content, and losing must mean "write
/// nothing", not "write the hint".
#[test]
fn a_stale_hint_contradicted_by_the_content_writes_no_wing() {
    let label = labels()
        .into_iter()
        .find(|l| l.session == "chat-20260807_1")
        .expect("the stale-hint session must be in the fixture");
    assert_eq!(label.wing, "kinrows");

    let verdict = corroborator("getladle").verdict(
        "User: the kinrows signup flow is broken\nAssistant: looking at it now",
        "",
    );
    assert_eq!(
        verdict,
        WingVerdict::Conflicting {
            named_wing: "kinrows".to_string()
        }
    );
    assert_eq!(
        verdict.wing(),
        None,
        "the hint said getladle and the turn said kinrows; the only honest \
         outcome is neither"
    );
}

/// `chat-20260609_2` is `henry-infra` — operating the assistant's own UI counts
/// as infrastructure, not as the `permagent` marketing site. A lexical rule
/// cannot reach that distinction, so the requirement is negative: it must not
/// reach for `permagent` instead.
#[test]
fn a_turn_that_names_no_project_is_not_filed_under_the_hint() {
    let label = labels()
        .into_iter()
        .find(|l| l.session == "chat-20260609_2")
        .expect("the henry-infra session must be in the fixture");
    assert_eq!(label.wing, "henry-infra");

    let verdict = corroborator("permagent").verdict(
        "User: click the orb and then open the sidebar\nAssistant: done",
        "",
    );
    assert_eq!(
        verdict,
        WingVerdict::Unverifiable,
        "driving the app's UI names no project; an honest `general` is right \
         and `permagent` would be invisibly wrong"
    );
    assert_eq!(verdict.wing(), None);
}

/// `chat-20260801_1` is `general`, correctly — weather and ambient assistant
/// use belong to no project. A hint must not manufacture one.
#[test]
fn an_ambient_session_stays_general_even_with_a_hint() {
    let label = labels()
        .into_iter()
        .find(|l| l.session == "chat-20260801_1")
        .expect("the ambient session must be in the fixture");
    assert_eq!(label.wing, "general");

    let verdict = corroborator("permagent")
        .verdict("User: what's the weather?\nAssistant: 14 and raining", "");
    assert_eq!(verdict, WingVerdict::Unverifiable);
    assert_eq!(verdict.wing(), None);
}

/// `chat-20260729_11` is `lauft`, and its content says "Loft" — a voice
/// transcription. This is a known, recorded recall gap: the rule must MISS it,
/// not guess. A miss is a turn left in `general`; a guess would be a wrong wing,
/// which is invisible.
#[test]
fn a_voice_transcribed_name_is_a_miss_and_never_a_wrong_guess() {
    let label = labels()
        .into_iter()
        .find(|l| l.session == "chat-20260729_11")
        .expect("the voice-transcription session must be in the fixture");
    assert_eq!(label.wing, "lauft");

    let verdict = corroborator("lauft").verdict(
        "User: can you check the Loft booking page\nAssistant: sure",
        "",
    );
    assert_eq!(
        verdict,
        WingVerdict::Unverifiable,
        "lexical matching cannot close this class; Spectral's labelled \
         calibration set is where the fix belongs"
    );
    assert_eq!(verdict.wing(), None);

    // And when the name IS spelled, the same session's hint is corroborated —
    // so the miss is about the spelling, not about the project being unknown.
    assert_eq!(
        corroborator("lauft")
            .verdict("User: the LAUFT booking page\nAssistant: sure", "")
            .wing(),
        Some("lauft")
    );
}

/// A `personal` session can never be assigned a wing, whatever the hint or the
/// content says. Two of the twelve labelled sessions are `personal`.
#[test]
fn personal_sessions_can_never_be_assigned_a_wing() {
    let personal: Vec<_> = labels()
        .into_iter()
        .filter(|l| l.wing == PERSONAL_SLUG)
        .collect();
    assert_eq!(personal.len(), 2, "two personal sessions in the fixture");

    let verdict = corroborator(PERSONAL_SLUG).verdict(
        "User: a personal story about Personal\nAssistant: lovely",
        "",
    );
    assert_eq!(verdict, WingVerdict::Unverifiable);
    assert_eq!(verdict.wing(), None);
}

/// The fixture's explicit warning: `permagent` (the marketing website, 103
/// memories) and `permagent-runtime` (the codebase, 379) are near-homonyms that
/// "any lexical or naive-bayes classifier conflates". This one must not.
#[test]
fn the_near_homonym_pair_the_fixture_warns_about_is_not_conflated() {
    // The codebase named in full goes to the codebase, from either side.
    assert_eq!(
        corroborator("permagent-runtime")
            .verdict(
                "User: permagent-runtime clippy is red\nAssistant: on it",
                ""
            )
            .wing(),
        Some("permagent-runtime")
    );
    assert_eq!(
        corroborator("permagent").verdict(
            "User: permagent-runtime clippy is red\nAssistant: on it",
            ""
        ),
        WingVerdict::Conflicting {
            named_wing: "permagent-runtime".to_string()
        }
    );

    // The site named alone goes to the site.
    assert_eq!(
        corroborator("permagent")
            .verdict("User: the permagent landing page copy\nAssistant: ok", "")
            .wing(),
        Some("permagent")
    );

    // `chat-20260820_3` is labelled `permagent-runtime` and `chat-20260625_14`
    // `permagent`; both exist in the fixture, so this is not a hypothetical
    // pair invented for the test.
    let wings: Vec<String> = labels().into_iter().map(|l| l.wing).collect();
    assert!(wings.iter().any(|w| w == "permagent"));
    assert!(wings.iter().any(|w| w == "permagent-runtime"));
}

/// Zero wrong assignments across every shape the labels establish an answer
/// for. Each row is `(hint, synthetic content, the wing the rule may write)`,
/// and `None` means "must write nothing".
#[test]
fn no_labelled_shape_produces_a_wrong_assignment() {
    let cases: Vec<(&str, &str, Option<&str>)> = vec![
        // Stale hint, content names another project → neither.
        ("getladle", "the kinrows signup flow", None),
        // No project named → nothing, whatever the hint.
        ("permagent", "click the orb and open the sidebar", None),
        ("wealthie", "what's the weather?", None),
        // Voice-transcribed name → a miss, not a guess.
        ("lauft", "check the Loft booking page", None),
        // Personal → never a wing.
        (PERSONAL_SLUG, "a personal story", None),
        // Near-homonyms → the longer name wins, from either side.
        (
            "permagent-runtime",
            "permagent-runtime clippy",
            Some("permagent-runtime"),
        ),
        ("permagent", "permagent-runtime clippy", None),
        ("permagent", "the permagent landing page", Some("permagent")),
        // Straightforward corroboration still works.
        ("wealthie", "the wealthie dashboard", Some("wealthie")),
        ("kinrows", "kinrows signup", Some("kinrows")),
    ];

    let mut fired = 0usize;
    for (hint_slug, content, expected) in cases {
        let verdict = corroborator(hint_slug).verdict(content, "");
        assert_eq!(
            verdict.wing(),
            expected,
            "hint {hint_slug:?} on {content:?} — a wrong wing is invisible, so \
             this assertion is the only thing standing between the rule and a \
             silent regression"
        );
        if verdict.wing().is_some() {
            fired += 1;
        }
    }
    assert!(
        fired > 0,
        "a rule that never fires has perfect precision and no value"
    );
    println!(
        "wing calibration: {fired} of the labelled shapes corroborate; the rest write nothing."
    );
}

/// The source label must be honest about which signal fired — the yield of each
/// is what Spectral will measure per source.
#[test]
fn the_recorded_source_names_the_signal_that_actually_fired() {
    match corroborator("lauft").verdict("User: the LAUFT booking page\nAssistant: ok", "") {
        WingVerdict::Corroborated { source, .. } => {
            assert_eq!(source, CorroborationSource::ContentName);
            assert_eq!(source.as_str(), "content-name");
        }
        other => panic!("expected corroboration, got {other:?}"),
    }
}
