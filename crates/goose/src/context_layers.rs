//! Progressive context layers — Abstract / Overview / Full.
//!
//! Token-budgeted assemble over recall hits. Deterministic, embedding-free:
//! Librarian FACTS (or a first-sentence fallback) is Abstract; a short lead of
//! the body is Overview; the body is Full. The function never talks to Brain,
//! a vector index, or an LLM.
//!
//! Home of this feature is macOS Command Center (Brain / Build / Inbox /
//! Projects). iOS is a companion, not the debut surface.

use serde::{Deserialize, Serialize};

/// How deep a hit is loaded. Abstract first; deepen only while budget remains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextLayer {
    #[default]
    Abstract,
    Overview,
    Full,
}

impl ContextLayer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Abstract => "abstract",
            Self::Overview => "overview",
            Self::Full => "full",
        }
    }
}

/// Token budget for one assemble pass. ~4 chars ≈ 1 token (no tokenizer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssembleBudget {
    pub tokens: usize,
}

impl AssembleBudget {
    pub const REPLY: Self = Self { tokens: 320 };
    pub const SEARCH: Self = Self { tokens: 480 };
    pub const AMBIENT: Self = Self { tokens: 240 };

    pub fn chars(self) -> usize {
        self.tokens.saturating_mul(4)
    }
}

/// One recall hit as assemble sees it — no Spectral types, no I/O.
#[derive(Debug, Clone)]
pub struct AssembleSource<'a> {
    pub key: &'a str,
    /// Librarian FACTS / description. Prefer this for Abstract.
    pub abstract_text: Option<&'a str>,
    pub content: &'a str,
    pub score: f64,
}

/// One hit after assemble: the layer chosen and the text that layer uses.
#[derive(Debug, Clone, PartialEq)]
pub struct LayeredHit {
    pub key: String,
    pub layer: ContextLayer,
    pub text: String,
    pub score: f64,
    /// Quiet “why this?” — lexical score + layer, never a vector distance.
    pub why: String,
}

/// First sentence, or the whole string if it has no terminator.
fn first_sentence(s: &str) -> &str {
    let s = s.trim();
    s.find(['.', '!', '?'])
        // `..=i` is a BYTE range; `find` returns the byte offset of an ASCII
        // terminator, so `i + 1` is always a char boundary here — `get` states
        // that instead of relying on it, and degrades to the whole string.
        .and_then(|i| s.get(..=i))
        .map(str::trim)
        .filter(|t| t.len() >= 8)
        .unwrap_or(s)
}

fn overview_lead(s: &str) -> &str {
    let s = s.trim();
    if s.chars().count() <= 400 {
        return s;
    }
    let end = s.char_indices().nth(400).map(|(i, _)| i).unwrap_or(s.len());
    s.get(..end).unwrap_or(s)
}

fn take_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn abstract_of(src: &AssembleSource<'_>) -> String {
    src.abstract_text
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| first_sentence(src.content))
        .to_string()
}

/// Fill Abstracts for every hit, then deepen top hits to Overview, then Full,
/// stopping when the character budget is spent. Hits stay in input order
/// (caller already ranked).
pub fn assemble(sources: &[AssembleSource<'_>], budget: AssembleBudget) -> Vec<LayeredHit> {
    let mut remaining = budget.chars();
    let mut out: Vec<LayeredHit> = Vec::with_capacity(sources.len());

    for src in sources {
        if remaining == 0 {
            break;
        }
        let abstract_text = abstract_of(src);
        let cost = abstract_text.chars().count();
        let text = if cost > remaining {
            take_chars(&abstract_text, remaining)
        } else {
            abstract_text
        };
        let used = text.chars().count();
        remaining -= used;
        out.push(LayeredHit {
            key: src.key.to_string(),
            layer: ContextLayer::Abstract,
            why: format!(
                "score {:.2} · {}",
                src.score,
                ContextLayer::Abstract.as_str()
            ),
            text,
            score: src.score,
        });
    }

    deepen(&mut out, sources, ContextLayer::Overview, &mut remaining);
    deepen(&mut out, sources, ContextLayer::Full, &mut remaining);
    out
}

fn deepen(
    out: &mut [LayeredHit],
    sources: &[AssembleSource<'_>],
    target: ContextLayer,
    remaining: &mut usize,
) {
    for (hit, src) in out.iter_mut().zip(sources.iter()) {
        let next = match target {
            ContextLayer::Overview => overview_lead(src.content).to_string(),
            ContextLayer::Full => src.content.trim().to_string(),
            ContextLayer::Abstract => continue,
        };
        let next_len = next.chars().count();
        let current_len = hit.text.chars().count();
        if next_len <= current_len {
            continue;
        }
        let extra = next_len.saturating_sub(current_len);
        if extra > *remaining {
            continue;
        }
        *remaining -= extra;
        hit.layer = target;
        hit.text = next;
        hit.why = format!("score {:.2} · {}", src.score, target.as_str());
    }
}

/// Render layered hits for a system-prompt prefix.
pub fn render_prompt(hits: &[LayeredHit]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::from("Relevant memories from past context:\n");
    for hit in hits {
        out.push_str(&format!("- [{}] {}\n", hit.layer.as_str(), hit.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src<'a>(
        key: &'a str,
        abs: Option<&'a str>,
        content: &'a str,
        score: f64,
    ) -> AssembleSource<'a> {
        AssembleSource {
            key,
            abstract_text: abs,
            content,
            score,
        }
    }

    #[test]
    fn three_layers_exist() {
        assert_eq!(ContextLayer::Abstract.as_str(), "abstract");
        assert_eq!(ContextLayer::Overview.as_str(), "overview");
        assert_eq!(ContextLayer::Full.as_str(), "full");
    }

    #[test]
    fn assemble_is_pure_of_hits_and_budget() {
        let long_a = format!(
            "We decided to use Clerk for auth after comparing options. {}",
            "Long body. ".repeat(80)
        );
        let hits = [
            src("a", Some("FACTS: Clerk for auth."), &long_a, 0.9),
            src(
                "b",
                None,
                "Browser navigated to mail.google.com in tab one.",
                0.8,
            ),
            src(
                "c",
                Some("FACTS: Goal parked."),
                "The goal parked because verify failed twice on the same test.",
                0.7,
            ),
        ];
        let tight = assemble(&hits, AssembleBudget { tokens: 20 });
        assert!(!tight.is_empty());
        assert!(
            tight.iter().all(|h| h.layer == ContextLayer::Abstract),
            "tiny budget must stay on abstracts: {:?}",
            tight.iter().map(|h| h.layer).collect::<Vec<_>>()
        );

        let wide = assemble(&hits, AssembleBudget { tokens: 4000 });
        assert_eq!(wide.len(), 3);
        assert!(
            wide.iter().any(|h| h.layer != ContextLayer::Abstract),
            "large budget must deepen at least one hit"
        );
        assert_eq!(wide[0].text, hits[0].content.trim());
        assert_eq!(wide[0].layer, ContextLayer::Full);
    }

    #[test]
    fn remember_then_recall_ships_abstracts_under_budget() {
        // N2: three stored hits, 200-token budget → only Abstracts in the prompt.
        let long = "Body that must not leak into a tight assemble. ".repeat(40);
        let a = format!(
            "FACTS: Clerk is the auth provider. {}",
            "Abs A. ".repeat(30)
        );
        let b = format!(
            "FACTS: Inbox files become searchable Brain memories. {}",
            "Abs B. ".repeat(30)
        );
        let c = format!(
            "FACTS: Premature done is held until verify passes. {}",
            "Abs C. ".repeat(30)
        );
        let hits = [
            src("doc:p:a.md", Some(&a), &long, 0.9),
            src("doc:p:b.md", Some(&b), &long, 0.8),
            src("doc:p:c.md", Some(&c), &long, 0.7),
        ];
        let layered = assemble(&hits, AssembleBudget { tokens: 200 });
        assert_eq!(layered.len(), 3);
        assert!(
            layered.iter().all(|h| h.layer == ContextLayer::Abstract),
            "200-token budget must stay on abstracts: {:?}",
            layered.iter().map(|h| h.layer).collect::<Vec<_>>()
        );
        let prompt = render_prompt(&layered);
        assert!(prompt.contains("[abstract]"));
        assert!(!prompt.contains("[full]"));
        assert!(!prompt.contains("must not leak"));
    }

    #[test]
    fn full_is_not_chosen_when_overview_fits_a_medium_budget() {
        let long = "A".repeat(800);
        let hits = [src("only", Some("FACTS: short."), &long, 1.0)];
        // Overview lead is 400 chars; Full is 800. Budget of ~120 tokens (480
        // chars) pays Abstract + Overview extra, not Full.
        let mid = assemble(&hits, AssembleBudget { tokens: 120 });
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].layer, ContextLayer::Overview);
        assert!(mid[0].text.len() < long.len());
    }

    #[test]
    fn module_has_no_embedder_or_foreign_memory_deps() {
        let prod = include_str!("context_layers.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for needle in [
            "use spectral",
            "tiktoken",
            "openviking",
            "viking://",
            "switchyard",
            "ort::",
            "openai",
        ] {
            assert!(
                !prod.to_ascii_lowercase().contains(needle),
                "context_layers must not depend on {needle}"
            );
        }
    }

    #[test]
    fn oversized_first_abstract_never_exceeds_budget() {
        let huge = "memory ".repeat(200);
        let hits = [src("huge", Some(&huge), "body", 1.0)];
        let layered = assemble(&hits, AssembleBudget { tokens: 3 });
        assert_eq!(layered.len(), 1);
        assert!(layered[0].text.chars().count() <= 12);
    }

    #[test]
    fn unicode_budget_and_overview_are_character_bounded() {
        let abstract_text = "🧠".repeat(40);
        let content = "界".repeat(800);
        let hits = [src("unicode", Some(&abstract_text), &content, 1.0)];
        let layered = assemble(&hits, AssembleBudget { tokens: 5 });
        assert_eq!(layered[0].text.chars().count(), 20);

        let overview = overview_lead(&content);
        assert_eq!(overview.chars().count(), 400);
        assert!(content.starts_with(overview));
    }

    #[test]
    fn duplicate_keys_deepen_against_their_own_source() {
        let first = "first ".repeat(100);
        let second = "second ".repeat(100);
        let hits = [
            src("same", Some("A"), &first, 1.0),
            src("same", Some("B"), &second, 0.9),
        ];
        let layered = assemble(&hits, AssembleBudget { tokens: 1_000 });
        assert_eq!(layered[0].text, first.trim());
        assert_eq!(layered[1].text, second.trim());
    }
}
