//! Grammar-constrained (GBNF) decoding for the local inference tier.
//!
//! Small local models are unreliable at emitting well-formed tool calls; the
//! cost-router flags their `edit_format_reliability` as sub-0.97. Constrained
//! decoding masks, at every sampling step, any token that would break the
//! required structure — taking structured-output validity from near-zero to
//! ~99% for small models, which is exactly the metric that gates local editors.
//!
//! We do **not** hand-write a JSON-Schema -> GBNF converter. llama.cpp already
//! computes a model/template-aware tool-call grammar from the tool definitions
//! and hands it back on [`ChatTemplateResult`] (`grammar`, `grammar_lazy`,
//! `grammar_triggers`). We consume that and build the matching sampler, mapping
//! the triggers exactly as llama.cpp's own `common_sampler_init` does.
//!
//! ## Discipline: constrain the ENVELOPE, not the reasoning
//!
//! We only ever apply the **lazy** (reason-then-format) grammar. A lazy grammar
//! stays completely inert until the model emits a tool-call trigger (e.g.
//! `<tool_call>`); everything the model reasons/writes before that point is left
//! unconstrained. We deliberately never apply an eager, whole-output grammar:
//! over-rigid whole-message JSON carries a large accuracy tax, and on this path
//! (tool_choice = auto, no response_format) llama.cpp never requests one anyway
//! — every chat format sets `grammar_lazy = true` for `tool_choice = auto`.

use llama_cpp_2::model::{ChatTemplateResult, GrammarTrigger, GrammarTriggerType, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

/// Env var to disable local-tier grammar-constrained decoding.
const GRAMMAR_ENV: &str = "PERMAGENT_LOCAL_GRAMMAR";

/// Root rule name llama.cpp uses for the grammars it generates.
const GRAMMAR_ROOT: &str = "root";

/// Whether grammar-constrained decoding is enabled for the local tier.
///
/// ON by default — it is a reliability mechanism, not a model/vendor choice
/// (consistent with defaulting the verify-loop on). Set
/// `PERMAGENT_LOCAL_GRAMMAR=off` (also accepts `0`/`false`/`no`) to disable.
pub(super) fn local_grammar_enabled() -> bool {
    grammar_enabled_from_env_value(std::env::var(GRAMMAR_ENV).ok().as_deref())
}

/// Pure parser for the env value, split out so it can be tested without touching
/// the process environment (global `set_var` races under parallel `cargo test`).
fn grammar_enabled_from_env_value(value: Option<&str>) -> bool {
    match value {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        ),
        None => true,
    }
}

/// Decide whether a template result's grammar should be applied.
///
/// True iff enabled AND the template produced a non-empty tool grammar AND it is
/// the lazy (reason-then-format) variety. Eager whole-output grammars are never
/// applied — see the module discipline note.
fn should_apply_tool_grammar(enabled: bool, grammar: Option<&str>, grammar_lazy: bool) -> bool {
    enabled && grammar.is_some_and(|g| !g.trim().is_empty()) && grammar_lazy
}

/// Port of llama.cpp `common.cpp::regex_escape`: escape the ECMAScript regex
/// metacharacters so a literal trigger word is matched literally.
fn regex_escape(s: &str) -> String {
    const SPECIAL: &[char] = &[
        '.', '^', '$', '|', '(', ')', '*', '+', '?', '[', ']', '{', '}', '\\',
    ];
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if SPECIAL.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Port of the `PATTERN_FULL` anchoring in llama.cpp `common_sampler_init`:
/// wrap the pattern in `^...$` (empty pattern becomes `^$`).
fn anchor_pattern(pattern: &str) -> String {
    if pattern.is_empty() {
        return "^$".to_string();
    }
    let mut out = String::new();
    if !pattern.starts_with('^') {
        out.push('^');
    }
    out.push_str(pattern);
    if !pattern.ends_with('$') {
        out.push('$');
    }
    out
}

/// Map template grammar triggers into `(regex patterns, trigger tokens)` exactly
/// as llama.cpp's `common_sampler_init` does when building a lazy grammar:
/// literal words are regex-escaped, patterns pass through, full patterns are
/// anchored, and token triggers are collected separately.
fn map_grammar_triggers(triggers: &[GrammarTrigger]) -> (Vec<String>, Vec<LlamaToken>) {
    let mut patterns = Vec::new();
    let mut tokens = Vec::new();
    for trigger in triggers {
        match trigger.trigger_type {
            GrammarTriggerType::Word => patterns.push(regex_escape(&trigger.value)),
            GrammarTriggerType::Pattern => patterns.push(trigger.value.clone()),
            GrammarTriggerType::PatternFull => patterns.push(anchor_pattern(&trigger.value)),
            GrammarTriggerType::Token => {
                if let Some(token) = trigger.token {
                    tokens.push(token);
                }
            }
        }
    }
    (patterns, tokens)
}

/// Build the tool-call grammar sampler from a chat-template result, or `None`
/// when grammar decoding is disabled, not applicable, or fails to initialize.
///
/// On any failure we return `None` (generate unconstrained) rather than
/// erroring: constrained decoding is a reliability boost, never a hard
/// requirement, so it must never take down a generation.
pub(super) fn build_tool_grammar_sampler(
    model: &LlamaModel,
    template_result: &ChatTemplateResult,
) -> Option<LlamaSampler> {
    let grammar = template_result.grammar.as_deref();
    if !should_apply_tool_grammar(
        local_grammar_enabled(),
        grammar,
        template_result.grammar_lazy,
    ) {
        return None;
    }
    // Non-empty per `should_apply_tool_grammar`.
    let grammar = grammar?;
    let (patterns, tokens) = map_grammar_triggers(&template_result.grammar_triggers);

    match LlamaSampler::grammar_lazy_patterns(model, grammar, GRAMMAR_ROOT, &patterns, &tokens) {
        Ok(sampler) => {
            tracing::debug!(
                trigger_patterns = patterns.len(),
                trigger_tokens = tokens.len(),
                "local grammar: constraining tool-call envelope (lazy, reason-then-format)"
            );
            Some(sampler)
        }
        Err(e) => {
            tracing::warn!(
                "local grammar: failed to init grammar sampler ({e}); generating unconstrained"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(value: &str) -> GrammarTrigger {
        GrammarTrigger {
            trigger_type: GrammarTriggerType::Word,
            value: value.to_string(),
            token: None,
        }
    }

    fn pattern(value: &str) -> GrammarTrigger {
        GrammarTrigger {
            trigger_type: GrammarTriggerType::Pattern,
            value: value.to_string(),
            token: None,
        }
    }

    fn pattern_full(value: &str) -> GrammarTrigger {
        GrammarTrigger {
            trigger_type: GrammarTriggerType::PatternFull,
            value: value.to_string(),
            token: None,
        }
    }

    fn token(id: i32) -> GrammarTrigger {
        GrammarTrigger {
            trigger_type: GrammarTriggerType::Token,
            value: String::new(),
            token: Some(LlamaToken(id)),
        }
    }

    // --- env parsing (default ON) -------------------------------------------

    #[test]
    fn test_grammar_enabled_default_on_when_unset() {
        assert!(grammar_enabled_from_env_value(None));
    }

    #[test]
    fn test_grammar_disabled_by_off_values() {
        for v in [
            "off", "OFF", "Off", "0", "false", "FALSE", "no", " off ", "\toff\n",
        ] {
            assert!(
                !grammar_enabled_from_env_value(Some(v)),
                "{v:?} should disable"
            );
        }
    }

    #[test]
    fn test_grammar_enabled_by_on_and_unknown_values() {
        // Only the explicit off-values disable; everything else stays ON.
        for v in ["on", "1", "true", "yes", "", "garbage"] {
            assert!(
                grammar_enabled_from_env_value(Some(v)),
                "{v:?} should stay enabled"
            );
        }
    }

    // --- regex_escape (port parity with llama.cpp) --------------------------

    #[test]
    fn test_regex_escape_leaves_plain_text_untouched() {
        // A common Hermes-style tool-call marker has no regex metacharacters.
        assert_eq!(regex_escape("<tool_call>"), "<tool_call>");
        assert_eq!(regex_escape("plain text 123"), "plain text 123");
    }

    #[test]
    fn test_regex_escape_escapes_metacharacters() {
        assert_eq!(regex_escape("a.b*c"), "a\\.b\\*c");
        assert_eq!(regex_escape("[x]"), "\\[x\\]");
        assert_eq!(regex_escape("(a+b)?"), "\\(a\\+b\\)\\?");
        assert_eq!(regex_escape("a|b^c$"), "a\\|b\\^c\\$");
        assert_eq!(regex_escape("back\\slash"), "back\\\\slash");
        assert_eq!(regex_escape("{n}"), "\\{n\\}");
    }

    // --- anchor_pattern (PATTERN_FULL) --------------------------------------

    #[test]
    fn test_anchor_pattern() {
        assert_eq!(anchor_pattern(""), "^$");
        assert_eq!(anchor_pattern("foo"), "^foo$");
        assert_eq!(anchor_pattern("^foo"), "^foo$");
        assert_eq!(anchor_pattern("foo$"), "^foo$");
        assert_eq!(anchor_pattern("^foo$"), "^foo$");
    }

    // --- trigger mapping ----------------------------------------------------

    #[test]
    fn test_map_word_trigger_is_escaped_pattern() {
        let (patterns, tokens) = map_grammar_triggers(&[word("<tool_call>")]);
        assert_eq!(patterns, vec!["<tool_call>".to_string()]);
        assert!(tokens.is_empty());

        let (patterns, _) = map_grammar_triggers(&[word("a.b")]);
        assert_eq!(patterns, vec!["a\\.b".to_string()]);
    }

    #[test]
    fn test_map_pattern_trigger_passes_through() {
        let (patterns, tokens) = map_grammar_triggers(&[pattern("(?:```(?:json)?\\s*)?\\{")]);
        assert_eq!(patterns, vec!["(?:```(?:json)?\\s*)?\\{".to_string()]);
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_map_pattern_full_trigger_is_anchored() {
        let (patterns, _) = map_grammar_triggers(&[pattern_full("\\{.*")]);
        assert_eq!(patterns, vec!["^\\{.*$".to_string()]);
    }

    #[test]
    fn test_map_token_trigger_collects_token() {
        let (patterns, tokens) = map_grammar_triggers(&[token(128010)]);
        assert!(patterns.is_empty());
        assert_eq!(tokens, vec![LlamaToken(128010)]);
    }

    #[test]
    fn test_map_token_trigger_without_id_is_skipped() {
        let no_id = GrammarTrigger {
            trigger_type: GrammarTriggerType::Token,
            value: String::new(),
            token: None,
        };
        let (patterns, tokens) = map_grammar_triggers(&[no_id]);
        assert!(patterns.is_empty());
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_map_mixed_triggers_partitions_correctly() {
        let (patterns, tokens) = map_grammar_triggers(&[
            word("<tool_call>"),
            pattern("\\{\"name\""),
            pattern_full("call:"),
            token(7),
            token(9),
        ]);
        assert_eq!(
            patterns,
            vec![
                "<tool_call>".to_string(),
                "\\{\"name\"".to_string(),
                "^call:$".to_string(),
            ]
        );
        assert_eq!(tokens, vec![LlamaToken(7), LlamaToken(9)]);
    }

    // --- apply decision: include-when-enabled / omit-when-off / discipline ---

    #[test]
    fn test_apply_when_enabled_lazy_and_grammar_present() {
        assert!(should_apply_tool_grammar(
            true,
            Some("root ::= \"{\" ws \"}\""),
            true
        ));
    }

    #[test]
    fn test_omit_when_disabled() {
        // The off-switch wins even with a valid lazy grammar present.
        assert!(!should_apply_tool_grammar(
            false,
            Some("root ::= \"{\" ws \"}\""),
            true
        ));
    }

    #[test]
    fn test_omit_when_no_grammar() {
        assert!(!should_apply_tool_grammar(true, None, true));
        assert!(!should_apply_tool_grammar(true, Some(""), true));
        assert!(!should_apply_tool_grammar(true, Some("   \n"), true));
    }

    #[test]
    fn test_omit_eager_grammar_to_protect_reasoning() {
        // DISCIPLINE: an eager (whole-output) grammar would constrain the
        // reasoning region, not just the tool-call envelope. We refuse it even
        // when enabled and a grammar is present — only the lazy variety is
        // applied, so free-text reasoning before the call stays unconstrained.
        assert!(!should_apply_tool_grammar(
            true,
            Some("root ::= \"{\" ws \"}\""),
            false
        ));
    }
}
