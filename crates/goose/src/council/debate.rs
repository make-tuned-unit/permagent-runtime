//! Two-round debate plus chair synthesis. Callers inject [`MemberCaller`]
//! so tests never hit a live provider.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::membership::Member;
use super::verdict;

pub const MEMBER_TIMEOUT_SECS: u64 = 90;
pub const MAX_ACTIONS: usize = 5;

/// How many trailing report lines the verdict re-ask quotes back. Enough for
/// the chair to recognize its own ruling, short enough that the nag stays a
/// cheap call rather than a second synthesis.
const NAG_TAIL_LINES: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Round1Take {
    #[serde(default)]
    pub projects_need_attention: Vec<String>,
    #[serde(default)]
    pub signs_to_recognize: Vec<String>,
    #[serde(default)]
    pub missing_patterns: Vec<String>,
    #[serde(default)]
    pub promising_analytics: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Round2Take {
    #[serde(default)]
    pub votes: Vec<String>,
    #[serde(default)]
    pub dissent: Option<String>,
    #[serde(default)]
    pub revised: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChairReport {
    #[serde(default)]
    pub headline: String,
    #[serde(default)]
    pub markdown: String,
    #[serde(default)]
    pub consensus: Vec<String>,
    #[serde(default)]
    pub dissent: Vec<Value>,
    #[serde(default)]
    pub actions: Vec<ChairAction>,
    /// True when the chair never produced a parseable `VERDICT:` line, even
    /// after one re-ask. The report's markdown then carries
    /// [`verdict::NO_VERDICT_FLAG`] in place of a ruling, so the gap is durable
    /// in the record rather than inferred later. Not a stored column: the
    /// markdown itself is the record, and `verdict::parse` reads it back.
    #[serde(default)]
    pub verdict_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChairAction {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub project_name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct MemberResult {
    pub member: Member,
    pub status: String,
    pub raw: Option<String>,
    pub parsed: Option<Value>,
    pub error: Option<String>,
}

#[async_trait::async_trait]
pub trait MemberCaller: Send + Sync {
    async fn complete(
        &self,
        provider: &str,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<String, String>;
}

pub struct LiveCaller;

#[async_trait::async_trait]
impl MemberCaller for LiveCaller {
    async fn complete(
        &self,
        provider: &str,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<String, String> {
        let p = crate::providers::create_with_named_model(provider, model, Vec::new())
            .await
            .map_err(|e| format!("provider init failed: {e}"))?;
        let msg = crate::conversation::message::Message::user().with_text(user);
        let (response, _usage) = p
            .complete_fast("council", system, std::slice::from_ref(&msg), &[])
            .await
            .map_err(|e| format!("model call failed: {e}"))?;
        Ok(response.as_concat_text())
    }
}

pub fn extract_json(text: &str) -> Option<Value> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    serde_json::from_str(text.get(start..=end)?).ok()
}

pub fn parse_round1(text: &str) -> Option<Round1Take> {
    extract_json(text).and_then(|v| serde_json::from_value(v).ok())
}

pub fn parse_round2(text: &str) -> Option<Round2Take> {
    extract_json(text).and_then(|v| serde_json::from_value(v).ok())
}

pub fn parse_chair(text: &str) -> ChairReport {
    if let Some(v) = extract_json(text) {
        if let Ok(mut report) = serde_json::from_value::<ChairReport>(v) {
            if report.actions.len() > MAX_ACTIONS {
                report.actions.truncate(MAX_ACTIONS);
            }
            if report.markdown.trim().is_empty() {
                report.markdown = text.to_string();
            }
            if report.headline.trim().is_empty() {
                report.headline = "Weekly council report".to_string();
            }
            return report;
        }
    }
    ChairReport {
        headline: "Weekly council report".to_string(),
        markdown: text.to_string(),
        consensus: Vec::new(),
        dissent: Vec::new(),
        actions: Vec::new(),
        verdict_missing: false,
    }
}

pub fn round1_system() -> &'static str {
    "You are a member of a Council of LLMs advising one builder about their week. \
     You see a factual brief of their projects, boards, activity, analytics and open decisions. \
     Reply with ONLY JSON: \
     {\"projects_need_attention\":[string],\"signs_to_recognize\":[string],\
      \"missing_patterns\":[string],\"promising_analytics\":[string],\"confidence\":0.0}. \
     Be specific. Name projects. Do not invent numbers that were not in the brief. \
     You are one voice; another model will chair a synthesis."
}

pub fn round2_system() -> &'static str {
    "You already filed an independent take. Now you see the other council members' summaries. \
     Reply with ONLY JSON: \
     {\"votes\":[string],\"dissent\":string,\"revised\":string}. \
     votes: which peer claims you endorse (quote them briefly). \
     dissent: the ONE thing you would bet against the majority on, or null. \
     revised: a short restatement of your position after hearing the others."
}

pub fn chair_system() -> String {
    format!(
        "You chair a Council of LLMs. You have the same factual brief the members saw, \
         plus their round-1 takes and round-2 rebuttals. Write a weekly report the builder \
         can digest and act on. Reply with ONLY JSON: \
         {{\"headline\":string,\"markdown\":string,\"consensus\":[string],\
          \"dissent\":[{{\"model\":string,\"claim\":string}}],\
          \"actions\":[{{\"project_id\":string,\"project_name\":string,\"title\":string,\"description\":string}}]}}. \
         headline: <= 80 characters. markdown: the full report in markdown, with named dissent. \
         actions: at most 5, each a concrete next step tied to a real project_id from the brief. \
         You MAY advise. Prefer fewer, sharper actions. {}",
        verdict::prompt_clause()
    )
}

/// The re-ask system prompt. Deliberately narrow: one line, nothing else.
pub fn verdict_nag_system() -> &'static str {
    "You chaired a council report that did not end with its required ruling. Reply with \
     exactly one line and nothing else — no preamble, no JSON, no markdown fences."
}

pub fn summarize_round1(member: &Member, take: &Round1Take) -> String {
    format!(
        "### {} / {}\nattention: {}\nsigns: {}\nmissing: {}\nanalytics: {}\nconfidence: {:?}",
        member.display_name,
        member.model,
        take.projects_need_attention.join("; "),
        take.signs_to_recognize.join("; "),
        take.missing_patterns.join("; "),
        take.promising_analytics.join("; "),
        take.confidence
    )
}

async fn call_one(
    caller: &dyn MemberCaller,
    member: &Member,
    system: &str,
    user: &str,
) -> MemberResult {
    let fut = caller.complete(&member.provider, &member.model, system, user);
    match tokio::time::timeout(std::time::Duration::from_secs(MEMBER_TIMEOUT_SECS), fut).await {
        Ok(Ok(raw)) => MemberResult {
            member: member.clone(),
            status: "ok".to_string(),
            raw: Some(raw),
            parsed: None,
            error: None,
        },
        Ok(Err(e)) => MemberResult {
            member: member.clone(),
            status: "error".to_string(),
            raw: None,
            parsed: None,
            error: Some(e),
        },
        Err(_) => MemberResult {
            member: member.clone(),
            status: "timeout".to_string(),
            raw: None,
            parsed: None,
            error: Some(format!("timed out after {MEMBER_TIMEOUT_SECS}s")),
        },
    }
}

pub async fn run_round1(
    caller: &dyn MemberCaller,
    members: &[Member],
    brief_markdown: &str,
) -> Vec<MemberResult> {
    let mut futs = Vec::new();
    for m in members {
        let m = m.clone();
        let brief = brief_markdown.to_string();
        // Sequential join of spawned tasks so a hung member cannot stall the rest.
        futs.push(async move { call_one(caller, &m, round1_system(), &brief).await });
    }
    let mut out = Vec::new();
    for fut in futs {
        let mut r = fut.await;
        if r.status == "ok" {
            if let Some(raw) = &r.raw {
                r.parsed = parse_round1(raw).and_then(|t| serde_json::to_value(t).ok());
            }
        }
        out.push(r);
    }
    out
}

pub async fn run_round1_parallel(
    caller: &dyn MemberCaller,
    members: &[Member],
    brief_markdown: &str,
) -> Vec<MemberResult> {
    let handles: Vec<_> = members
        .iter()
        .map(|m| {
            let m = m.clone();
            let brief = brief_markdown.to_string();
            async move { call_one(caller, &m, round1_system(), &brief).await }
        })
        .collect();
    let mut out = futures::future::join_all(handles).await;
    for r in &mut out {
        if r.status == "ok" {
            if let Some(raw) = &r.raw {
                r.parsed = parse_round1(raw).and_then(|t| serde_json::to_value(t).ok());
            }
        }
    }
    out
}

pub async fn run_round2_parallel(
    caller: &dyn MemberCaller,
    round1: &[MemberResult],
) -> Vec<MemberResult> {
    let survivors: Vec<&MemberResult> = round1.iter().filter(|r| r.status == "ok").collect();
    if survivors.len() < 2 {
        return Vec::new();
    }
    let peer_digest: String = survivors
        .iter()
        .map(|r| {
            format!(
                "### {} / {}\n{}",
                r.member.display_name,
                r.member.model,
                r.raw.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let handles: Vec<_> = survivors
        .iter()
        .map(|r| {
            let member = r.member.clone();
            let own = r.raw.clone().unwrap_or_default();
            let digest = peer_digest.clone();
            async move {
                let user = format!(
                    "Your round-1 take:\n{own}\n\nPeer takes:\n{digest}\n\nNow vote and, if you must, dissent."
                );
                call_one(caller, &member, round2_system(), &user).await
            }
        })
        .collect();
    let mut out = futures::future::join_all(handles).await;
    for r in &mut out {
        if r.status == "ok" {
            if let Some(raw) = &r.raw {
                r.parsed = parse_round2(raw).and_then(|t| serde_json::to_value(t).ok());
            }
        }
    }
    out
}

pub async fn run_chair(
    caller: &dyn MemberCaller,
    chair: &Member,
    brief_markdown: &str,
    round1: &[MemberResult],
    round2: &[MemberResult],
) -> Result<ChairReport, String> {
    let mut user = String::from("## Brief\n\n");
    user.push_str(brief_markdown);
    user.push_str("\n\n## Round 1\n\n");
    for r in round1 {
        user.push_str(&format!(
            "### {} / {} ({})\n{}\n\n",
            r.member.display_name,
            r.member.model,
            r.status,
            r.raw.as_deref().unwrap_or(r.error.as_deref().unwrap_or(""))
        ));
    }
    if !round2.is_empty() {
        user.push_str("## Round 2\n\n");
        for r in round2 {
            user.push_str(&format!(
                "### {} / {} ({})\n{}\n\n",
                r.member.display_name,
                r.member.model,
                r.status,
                r.raw.as_deref().unwrap_or(r.error.as_deref().unwrap_or(""))
            ));
        }
    }
    let raw = caller
        .complete(&chair.provider, &chair.model, &chair_system(), &user)
        .await?;
    let mut report = parse_chair(&raw);
    apply_verdict_gate(caller, chair, &mut report).await;
    Ok(report)
}

/// The nag. A chair report whose ruling is absent or unparseable is never
/// silently accepted: it costs one narrow re-ask, and if that also fails the
/// gap is written into the report itself as [`verdict::NO_VERDICT_FLAG`] and
/// flagged on [`ChairReport::verdict_missing`], so the briefing and the
/// rendered report both say the chair did not rule.
async fn apply_verdict_gate(caller: &dyn MemberCaller, chair: &Member, report: &mut ChairReport) {
    let problem = match verdict::parse(&report.markdown) {
        Ok(_) => return,
        Err(problem) => problem,
    };
    tracing::warn!(
        target: "permagent::council",
        "chair verdict unusable ({}); re-asking once", problem.describe()
    );
    let appended = match ask_for_verdict_line(caller, chair, &report.markdown, &problem).await {
        Some(v) => v.render(),
        None => {
            report.verdict_missing = true;
            verdict::NO_VERDICT_FLAG.to_string()
        }
    };
    report.markdown = format!("{}\n\n{}", report.markdown.trim_end(), appended);
}

/// One bounded re-ask for the verdict line alone. Returns `None` when the chair
/// errors, times out, or answers with something the strict parser still
/// rejects — the caller then flags rather than retrying again.
async fn ask_for_verdict_line(
    caller: &dyn MemberCaller,
    chair: &Member,
    markdown: &str,
    problem: &verdict::VerdictProblem,
) -> Option<verdict::ChairVerdict> {
    let lines: Vec<&str> = markdown.lines().collect();
    let tail = lines[lines.len().saturating_sub(NAG_TAIL_LINES)..].join("\n");
    let user = format!("{}\n\nYour report ended:\n\n{tail}", problem.nag());
    let result = call_one(caller, chair, verdict_nag_system(), &user).await;
    if result.status != "ok" {
        return None;
    }
    verdict::parse(result.raw.as_deref()?).ok()
}

/// True when at least one member answered round 1.
pub fn any_ok(round: &[MemberResult]) -> bool {
    round.iter().any(|r| r.status == "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Scripted {
        replies: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl MemberCaller for Scripted {
        async fn complete(
            &self,
            _provider: &str,
            _model: &str,
            _system: &str,
            _user: &str,
        ) -> Result<String, String> {
            let mut q = self.replies.lock().unwrap();
            if q.is_empty() {
                return Err("empty script".into());
            }
            Ok(q.remove(0))
        }
    }

    fn member(p: &str) -> Member {
        Member {
            provider: p.into(),
            display_name: p.into(),
            model: "m".into(),
        }
    }

    #[test]
    fn extracts_json_from_fenced_prose() {
        let text = "Sure.\n```json\n{\"projects_need_attention\":[\"Permagent\"],\"signs_to_recognize\":[],\"missing_patterns\":[],\"promising_analytics\":[],\"confidence\":0.8}\n```";
        let take = parse_round1(text).unwrap();
        assert_eq!(take.projects_need_attention, vec!["Permagent"]);
        assert_eq!(take.confidence, Some(0.8));
    }

    #[test]
    fn one_member_error_does_not_block_parse_of_the_rest() {
        let ok = MemberResult {
            member: member("a"),
            status: "ok".into(),
            raw: Some("{\"projects_need_attention\":[\"X\"],\"signs_to_recognize\":[],\"missing_patterns\":[],\"promising_analytics\":[]}".into()),
            parsed: None,
            error: None,
        };
        let err = MemberResult {
            member: member("b"),
            status: "error".into(),
            raw: None,
            parsed: None,
            error: Some("boom".into()),
        };
        assert!(any_ok(&[ok, err]));
    }

    #[test]
    fn chair_caps_actions_at_five() {
        let actions: Vec<String> = (0..8).map(|i| format!("{{\"title\":\"a{i}\"}}")).collect();
        let json = format!(
            "{{\"headline\":\"H\",\"markdown\":\"# hi\",\"consensus\":[],\"dissent\":[],\"actions\":[{}]}}",
            actions.join(",")
        );
        let report = parse_chair(&json);
        assert_eq!(report.actions.len(), MAX_ACTIONS);
        assert_eq!(report.headline, "H");
    }

    /// RED-FIRST (a): an unparseable / absent verdict must NOT be silently
    /// accepted. Before the nag path existed, `run_chair` returned the report
    /// verbatim and nothing anywhere noticed the chair never ruled.
    #[tokio::test]
    async fn chair_without_a_verdict_line_is_flagged_not_silently_accepted() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nEverything looks fine.\",\
                  \"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                // The re-ask is answered with prose, not a verdict line.
                "I would rather not commit.".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        assert!(
            report.markdown.contains("NO VERDICT LINE"),
            "an absent verdict must surface, got: {}",
            report.markdown
        );
        assert!(report.verdict_missing);
        // The flag must not read back as a ruling.
        assert!(verdict::parse(&report.markdown).is_err());
        // The original report is preserved, not replaced by the complaint.
        assert!(report.markdown.contains("Everything looks fine."));
    }

    /// The nag recovers: one narrow re-ask, and the canonical line is appended.
    #[tokio::test]
    async fn a_missing_verdict_is_re_asked_once_and_recovered() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nBody.\",\
                  \"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                "VERDICT: HOLD — do less this week and finish the migration".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        assert!(!report.verdict_missing);
        assert!(!report.markdown.contains("NO VERDICT LINE"));
        let v = verdict::parse(&report.markdown).unwrap();
        assert_eq!(v.verdict, verdict::Verdict::Hold);
        assert_eq!(v.rationale, "do less this week and finish the migration");
    }

    /// A compliant chair costs exactly one call — the nag never fires.
    #[tokio::test]
    async fn a_ruled_report_is_not_re_asked() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nBody.\\n\\nVERDICT: ACT — \
                  file the two homepage cards\",\"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                // Deliberately poisoned: if this is ever consumed, the assert
                // below fails.
                "VERDICT: HOLD — the nag fired when it should not have".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        assert!(!report.verdict_missing);
        let v = verdict::parse(&report.markdown).unwrap();
        assert_eq!(v.verdict, verdict::Verdict::Act);
        assert_eq!(v.rationale, "file the two homepage cards");
    }

    /// A malformed value is treated exactly like an absent one — the strict
    /// parser is what makes the field queryable.
    #[tokio::test]
    async fn a_malformed_verdict_value_takes_the_nag_path() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nVERDICT: MAYBE — it depends\",\
                  \"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                "VERDICT: WATCH — nothing to start, keep an eye on churn".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        let v = verdict::parse(&report.markdown).unwrap();
        assert_eq!(v.verdict, verdict::Verdict::Watch);
    }

    #[test]
    fn the_chair_prompt_states_the_marker_convention() {
        let system = chair_system();
        assert!(system.contains(verdict::VERDICT_MARKER));
        assert!(system.contains("ACT|WATCH|HOLD"));
    }

    #[tokio::test]
    async fn round1_survives_a_failing_member() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"projects_need_attention\":[\"P\"],\"signs_to_recognize\":[],\"missing_patterns\":[],\"promising_analytics\":[]}".into(),
            ]),
        };
        // Second complete will fail (empty script) — only one reply queued, two members.
        let out = run_round1_parallel(&caller, &[member("ok"), member("fail")], "brief").await;
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|r| r.status == "ok"));
        assert!(out.iter().any(|r| r.status == "error"));
    }
}
