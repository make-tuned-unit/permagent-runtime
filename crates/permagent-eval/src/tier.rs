//! Model-tier profiles — the "cheap vs frontier" axis of the eval.
//!
//! A [`Tier`] pins the harness to a concrete provider + model for one run. The
//! *main loop* is pinned via `permagent run --provider/--model` (CLI flags
//! outrank the `GOOSE_PROVIDER`/`GOOSE_MODEL` environment). When `pin_packs` is
//! set, the cost-router model packs (#720, `PERMAGENT_PACK_*`) are ALSO pinned to
//! the same provider+model so any sub-agent work stays on the tier under test —
//! giving an apples-to-apples measurement of one model rather than the harness's
//! native cross-tier routing. Turn `pin_packs` off (`--native-routing`) to
//! measure the shipped cost-optimizer instead.
//!
//! Provider ids, model strings and key envs are the ones wired on `main`:
//! `ollama` (keyless), `minimax`/`MINIMAX_API_KEY`, `moonshot` (Kimi)/
//! `MOONSHOT_API_KEY`, `anthropic`/`ANTHROPIC_API_KEY`.
//!
//! The `model-defaults-bench` candidate set (`haiku`, `sonnet5`, `glm53`,
//! `glm47`, `minimax27`, `dschat`, `dsreason`, `kimi25`, `gpt54mini`) adds three
//! more provider ids: the declarative providers `custom_deepseek` (DeepSeek) and
//! `moonshot` (reused for the newer Kimi model) plus the built-in `zai`
//! (Z.ai/GLM) and `openai`, alongside `ANTHROPIC_API_KEY`/`ZAI_API_KEY`/
//! `MINIMAX_API_KEY`/`DEEPSEEK_API_KEY`/`MOONSHOT_API_KEY`/`OPENAI_API_KEY`.

/// The four cost-router pack roles (see `permagent`'s `cost_router::packs`).
/// Pinning every role to the tier's model keeps all work on one model.
const PACK_ROLES: [&str; 4] = ["EDIT", "HARD", "MECHANICAL", "LOCAL"];

/// A model tier: the provider + model the harness runs under for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier {
    /// Reporting label (e.g. `local`, `kimi`, `frontier`).
    pub name: String,
    /// Provider id passed to `--provider` (e.g. `ollama`, `anthropic`).
    pub provider: String,
    /// Model string passed to `--model`.
    pub model: String,
    /// When true, pin every cost-router pack to this provider+model.
    pub pin_packs: bool,
    /// The API-key env var this provider needs, if any (`None` = keyless, e.g.
    /// local Ollama). Used only for a pre-flight check.
    pub required_key_env: Option<String>,
    /// Lane K6 / K6-b: this tier's own FLOOR on the harness wall-clock
    /// ceiling (seconds). `None` (most tiers) applies no floor at all —
    /// the task's own `harness_timeout_secs:` (or
    /// [`crate::task::DEFAULT_HARNESS_TIMEOUT_SECS`] if it has none)
    /// applies unchanged. When `Some`, the effective ceiling is
    /// `max(task cap or default, this value)` — see
    /// [`crate::task::TaskSpec::resolved_harness_timeout_secs`] for the
    /// full resolution. It only ever RAISES the ceiling for a slow
    /// provider, never shortens a task's own (possibly looser) cap; a
    /// `--harness-timeout-secs` CLI override still outranks it when an
    /// invocation needs an exact value. The zai tiers (`glm47`, `glm53`)
    /// default to 900s here — evidence: zai glm-4.7 rounds z2/z3 on M19 hit
    /// the driver's flat 600s per-task ceiling on two fixtures while its own
    /// solved-task walls on this suite run 150-440s; K6-b's rationale is
    /// that a fixture's cap encodes "fast enough for a fast model," not a
    /// universal provider-speed claim, so a per-tier floor scales it for a
    /// 3-5x-slower provider without rewriting every fixture.
    pub harness_timeout_secs: Option<u64>,
}

impl Tier {
    /// Resolve a built-in tier by name. Returns `None` for unknown names.
    pub fn builtin(name: &str) -> Option<Tier> {
        let t = |name: &str, provider: &str, model: &str, key: Option<&str>| Tier {
            name: name.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            pin_packs: true,
            required_key_env: key.map(str::to_string),
            harness_timeout_secs: None,
        };
        match name {
            "local" => Some(t("local", "ollama", "qwen3", None)),
            "kimi" => Some(t("kimi", "moonshot", "kimi-k2.5", Some("MOONSHOT_API_KEY"))),
            "minimax" => Some(t(
                "minimax",
                "minimax",
                "MiniMax-M2.5",
                Some("MINIMAX_API_KEY"),
            )),
            "sonnet" => Some(t(
                "sonnet",
                "anthropic",
                "claude-sonnet-5",
                Some("ANTHROPIC_API_KEY"),
            )),
            "frontier" => Some(t(
                "frontier",
                "anthropic",
                "claude-opus-4-8",
                Some("ANTHROPIC_API_KEY"),
            )),
            // model-defaults-bench candidates (#the-nine): reproducible-by-name
            // tiers for the coding-harness default sweep. Provider ids verified
            // live: `custom_deepseek`, `minimax`, `moonshot` are this repo's
            // declarative providers; `zai`, `anthropic`, `openai` are built in.
            "haiku" => Some(t(
                "haiku",
                "anthropic",
                "claude-haiku-4-5-20251001",
                Some("ANTHROPIC_API_KEY"),
            )),
            "sonnet5" => Some(t(
                "sonnet5",
                "anthropic",
                "claude-sonnet-5",
                Some("ANTHROPIC_API_KEY"),
            )),
            // Lane K6: the zai tiers default their own harness ceiling to
            // 900s (see the field doc on `Tier::harness_timeout_secs` and
            // the module-level evidence). "leave dschat at the default"
            // (harness-improvement-loop directive) — DeepSeek's own walls on
            // this suite run 40-200s, well inside the flat 900s default, so
            // `dschat`/`dsreason` below get no override.
            "glm53" => Some(Tier {
                harness_timeout_secs: Some(900),
                ..t("glm53", "zai", "glm-5.3", Some("ZAI_API_KEY"))
            }),
            "glm47" => Some(Tier {
                harness_timeout_secs: Some(900),
                ..t("glm47", "zai", "glm-4.7", Some("ZAI_API_KEY"))
            }),
            "minimax27" => Some(t(
                "minimax27",
                "minimax",
                "MiniMax-M2.7",
                Some("MINIMAX_API_KEY"),
            )),
            "dschat" => Some(t(
                "dschat",
                "custom_deepseek",
                "deepseek-chat",
                Some("DEEPSEEK_API_KEY"),
            )),
            "dsreason" => Some(t(
                "dsreason",
                "custom_deepseek",
                "deepseek-reasoner",
                Some("DEEPSEEK_API_KEY"),
            )),
            "kimi25" => Some(t(
                "kimi25",
                "moonshot",
                "kimi-k2.5",
                Some("MOONSHOT_API_KEY"),
            )),
            "gpt54mini" => Some(t(
                "gpt54mini",
                "openai",
                "gpt-5.4-mini",
                Some("OPENAI_API_KEY"),
            )),
            // Lane MC2 (see `loop-scratch/MINICPM5_2B_PLAN.md`, Lane MC1's
            // research): a cheap-model-usability probe, not a
            // `coding_success` candidate — a 2.52B dense model isn't expected
            // to reliably drive the roadmap/verify/escalate tool-call loop
            // the way `sonnet`/`dschat` do. Keyless like `local`, but on
            // shared 16 GB hardware a 2.5B model is slow, so this tier
            // declares its own 1200s floor (Lane K6's `harness_timeout_secs`
            // mechanism — see the field doc above; it only ever RAISES a
            // task's ceiling, never shortens one).
            //
            // OpenBMB's own docs recommend SGLang for MiniCPM5's native
            // XML-style tool-call format; there is no stated llama.cpp/Ollama
            // parser for it. Two ways to get tool calls working through the
            // `ollama` provider if MiniCPM5's `TEMPLATE` doesn't already
            // speak Ollama's own tool-call JSON: (1) it may just work — the
            // `ollama` provider has no allowlist gating tool calls on a
            // model's listing metadata; (2) if native calls don't parse,
            // fall back to the toolshim: set `GOOSE_TOOLSHIM=true` and
            // `GOOSE_TOOLSHIM_OLLAMA_MODEL=<model>` (see
            // `crates/goose/src/providers/toolshim.rs`) to route raw text
            // output through a second Ollama model that re-derives
            // structured tool calls — format-agnostic, at the cost of a
            // second model load and an extra hop per call. There is no field
            // on `Tier` for this; it's an operator-set env var, not part of
            // the tier's own provider+model pinning.
            "minicpm5" => Some(Tier {
                harness_timeout_secs: Some(1200),
                ..t("minicpm5", "ollama", "openbmb/minicpm5:q8_0", None)
            }),
            // Fallback if `openbmb/minicpm5:q8_0`'s Ollama-library tag turns
            // out to be a smaller/mismatched model than the HF 2.5B GGUF
            // (UNVERIFIED in the plan — the fetched tag page's size/param
            // count didn't match the HF GGUF repo for the same quant level;
            // verify with `ollama show` before trusting it). This name is
            // the local Modelfile built manually from
            // `openbmb/MiniCPM5-2B-GGUF` per the plan's §4a fallback recipe —
            // it does not exist until that Modelfile is created and
            // `ollama create`d under this name.
            "minicpm5-hf" => Some(Tier {
                harness_timeout_secs: Some(1200),
                ..t("minicpm5-hf", "ollama", "minicpm5-2b-q8", None)
            }),
            _ => None,
        }
    }

    /// The names of every built-in tier (for `--help` and the CLI's tier list).
    pub fn builtin_names() -> &'static [&'static str] {
        &[
            "local",
            "kimi",
            "minimax",
            "sonnet",
            "frontier",
            "haiku",
            "sonnet5",
            "glm53",
            "glm47",
            "minimax27",
            "dschat",
            "dsreason",
            "kimi25",
            "gpt54mini",
            "minicpm5",
            "minicpm5-hf",
        ]
    }

    /// A custom, ad-hoc tier from explicit `--provider`/`--model` flags.
    pub fn custom(
        name: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Tier {
        Tier {
            name: name.into(),
            provider: provider.into(),
            model: model.into(),
            pin_packs: true,
            required_key_env: None,
            harness_timeout_secs: None,
        }
    }

    /// Return a copy with pack-pinning toggled (used by `--native-routing`).
    pub fn with_pin_packs(mut self, pin: bool) -> Tier {
        self.pin_packs = pin;
        self
    }

    /// The `PERMAGENT_PACK_*` environment overrides that pin every role to this
    /// tier's provider+model. Empty when `pin_packs` is false.
    pub fn pack_env(&self) -> Vec<(String, String)> {
        if !self.pin_packs {
            return Vec::new();
        }
        let mut env = Vec::with_capacity(PACK_ROLES.len() * 2);
        for role in PACK_ROLES {
            env.push((
                format!("PERMAGENT_PACK_{role}_PROVIDER"),
                self.provider.clone(),
            ));
            env.push((format!("PERMAGENT_PACK_{role}_MODEL"), self.model.clone()));
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_tiers_resolve_with_expected_wiring() {
        let local = Tier::builtin("local").unwrap();
        assert_eq!(local.provider, "ollama");
        assert_eq!(local.required_key_env, None);

        let kimi = Tier::builtin("kimi").unwrap();
        assert_eq!(kimi.provider, "moonshot");
        assert_eq!(kimi.required_key_env.as_deref(), Some("MOONSHOT_API_KEY"));

        let frontier = Tier::builtin("frontier").unwrap();
        assert_eq!(frontier.provider, "anthropic");
        assert_eq!(frontier.model, "claude-opus-4-8");
        assert_eq!(
            frontier.required_key_env.as_deref(),
            Some("ANTHROPIC_API_KEY")
        );
    }

    #[test]
    fn unknown_tier_is_none() {
        assert!(Tier::builtin("nope").is_none());
    }

    #[test]
    fn builtin_names_lists_every_tier_and_each_one_resolves() {
        let names = Tier::builtin_names();
        assert_eq!(names.len(), 16, "{names:?}");
        for &name in names {
            let t = Tier::builtin(name)
                .unwrap_or_else(|| panic!("{name} is in builtin_names() but does not resolve"));
            assert_eq!(t.name, name);
        }
        // The pre-existing five must still be present, unrenamed.
        for name in ["local", "kimi", "minimax", "sonnet", "frontier"] {
            assert!(names.contains(&name), "{name} missing from builtin_names()");
        }
        // The nine model-defaults-bench candidates.
        for name in [
            "haiku",
            "sonnet5",
            "glm53",
            "glm47",
            "minimax27",
            "dschat",
            "dsreason",
            "kimi25",
            "gpt54mini",
        ] {
            assert!(names.contains(&name), "{name} missing from builtin_names()");
        }
        // Lane MC2: the minicpm5 local tier and its HF-Modelfile fallback.
        for name in ["minicpm5", "minicpm5-hf"] {
            assert!(names.contains(&name), "{name} missing from builtin_names()");
        }
    }

    /// The nine model-defaults-bench candidates: provider, model and required
    /// key env, verified against the task's spec.
    #[test]
    fn resolves_the_nine_candidate_tiers_with_expected_wiring() {
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "haiku",
                "anthropic",
                "claude-haiku-4-5-20251001",
                "ANTHROPIC_API_KEY",
            ),
            (
                "sonnet5",
                "anthropic",
                "claude-sonnet-5",
                "ANTHROPIC_API_KEY",
            ),
            ("glm53", "zai", "glm-5.3", "ZAI_API_KEY"),
            ("glm47", "zai", "glm-4.7", "ZAI_API_KEY"),
            ("minimax27", "minimax", "MiniMax-M2.7", "MINIMAX_API_KEY"),
            (
                "dschat",
                "custom_deepseek",
                "deepseek-chat",
                "DEEPSEEK_API_KEY",
            ),
            (
                "dsreason",
                "custom_deepseek",
                "deepseek-reasoner",
                "DEEPSEEK_API_KEY",
            ),
            ("kimi25", "moonshot", "kimi-k2.5", "MOONSHOT_API_KEY"),
            ("gpt54mini", "openai", "gpt-5.4-mini", "OPENAI_API_KEY"),
        ];
        for &(name, provider, model, key) in cases {
            let t = Tier::builtin(name).unwrap_or_else(|| panic!("{name} must resolve"));
            assert_eq!(t.provider, provider, "{name} provider");
            assert_eq!(t.model, model, "{name} model");
            assert_eq!(t.required_key_env.as_deref(), Some(key), "{name} key env");
            assert!(t.pin_packs, "{name} should default to pinned packs");
        }
    }

    /// Lane MC2: `minicpm5` and its `minicpm5-hf` fallback resolve to the
    /// keyless `ollama` provider with the plan's models, and both declare
    /// the 1200s harness-timeout floor (Lane K6's mechanism — a 2.5B model
    /// on shared 16 GB hardware is slow).
    #[test]
    fn minicpm5_tiers_resolve_ollama_with_1200s_floor() {
        let minicpm5 = Tier::builtin("minicpm5").unwrap();
        assert_eq!(minicpm5.provider, "ollama");
        assert_eq!(minicpm5.model, "openbmb/minicpm5:q8_0");
        assert_eq!(minicpm5.required_key_env, None);
        assert_eq!(minicpm5.harness_timeout_secs, Some(1200));
        assert!(minicpm5.pin_packs);

        let minicpm5_hf = Tier::builtin("minicpm5-hf").unwrap();
        assert_eq!(minicpm5_hf.provider, "ollama");
        assert_eq!(minicpm5_hf.model, "minicpm5-2b-q8");
        assert_eq!(minicpm5_hf.required_key_env, None);
        assert_eq!(minicpm5_hf.harness_timeout_secs, Some(1200));
    }

    #[test]
    fn pack_env_pins_all_four_roles_when_enabled() {
        let t = Tier::builtin("frontier").unwrap();
        let env = t.pack_env();
        assert_eq!(env.len(), 8);
        assert!(env
            .iter()
            .any(|(k, v)| k == "PERMAGENT_PACK_HARD_PROVIDER" && v == "anthropic"));
        assert!(env
            .iter()
            .any(|(k, v)| k == "PERMAGENT_PACK_LOCAL_MODEL" && v == "claude-opus-4-8"));
    }

    #[test]
    fn native_routing_leaves_packs_unset() {
        let t = Tier::builtin("local").unwrap().with_pin_packs(false);
        assert!(t.pack_env().is_empty());
    }

    #[test]
    fn custom_tier_from_flags() {
        let t = Tier::custom("adhoc", "openrouter", "moonshotai/kimi-k2");
        assert_eq!(t.provider, "openrouter");
        assert_eq!(t.model, "moonshotai/kimi-k2");
        assert!(t.pin_packs);
        assert_eq!(t.harness_timeout_secs, None);
    }

    /// Lane K6 / K6-b: the zai tiers (`glm47`/`glm53`) declare a 900s
    /// harness ceiling FLOOR, and Lane MC2's `minicpm5`/`minicpm5-hf`
    /// declare their own 1200s floor (see `minicpm5_tiers_resolve_ollama_
    /// with_1200s_floor` above); every other tier — `dschat` explicitly
    /// named, since DeepSeek's own walls on this suite run comfortably under
    /// the flat 900s default — declares no tier-level floor (`None`, so
    /// `TaskSpec::resolved_harness_timeout_secs` applies just the task's
    /// own cap or `DEFAULT_HARNESS_TIMEOUT_SECS`, unchanged by the tier).
    #[test]
    fn zai_tiers_default_harness_timeout_to_900_others_are_unset() {
        for name in ["glm47", "glm53"] {
            let t = Tier::builtin(name).unwrap();
            assert_eq!(t.harness_timeout_secs, Some(900), "{name}");
        }
        for &name in Tier::builtin_names() {
            if name == "glm47" || name == "glm53" || name == "minicpm5" || name == "minicpm5-hf" {
                continue;
            }
            let t = Tier::builtin(name).unwrap();
            assert_eq!(t.harness_timeout_secs, None, "{name} must be unset");
        }
    }
}
