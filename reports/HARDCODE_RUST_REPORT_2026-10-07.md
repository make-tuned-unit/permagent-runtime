# Hardcode sweep (Rust) report, 2026-10-07, branch wq/hardcode-rust

STATUS: every change is UNCOMPILED and UNTESTED. Cargo was never run to completion (one `cargo check -p permagent` was queued behind another lane's build and killed when the coordinator paused cargo for the disk floor). Run `cargo check -p permagent`, `-p permagent-daemon`, `-p permagent-cli`, then the focused `--lib` tests below before merging.

## Commits (oldest first)
- 9c54ef62 L3 provider-aware defaults
- 71bb6e8a L4 local_default_model
- 4d1fe3ae L5 finance locale
- 585c871b L6 scanner/Polybot gating
- 77c9c761 L7 qwen38_split explicit-only
- bc783e21 L10 Ollama resolver
- 295ba301 L9 PostHog config
- fddfe9c6 L2 ontology fixture
- c7c5f980 L8 release config
- 76ac5251 small follow-up (General failover skips detection)

## Config keys added (default)
- trusted_default_models: list of provider/model (openai/gpt-5.4-mini, custom_deepseek/deepseek-chat)
- local_default_model (qwen2.5:7b, defined once in config/local_model.rs)
- finance_scanner_url (unset; picker_url still honoured); picker_root only (no checkout guessing)
- telemetry_posthog_key (empty = no sink), telemetry_posthog_host (default https://us.i.posthog.com, used only with a key)
- ~/.permagent/finance_rules.yaml (optional file: rules: [{match, category}])
- QWEN38_SPLIT_HOST now has no default (unset = not probed)

## Tests added/changed (permagent crate, run with `--lib -- <filter>`)
model_roles::tests (no providers, anthropic only, ollama only, order, explicit config authoritative, trusted list, gate), voice_model::tests, local_model::tests, finance_statements::tests, picker::tests (message), posthog::tests (sink), fallback tests (coord_ctx helper), agent_identity roster assertion. Server tests touched: config_management harness projection, voice default (use `_in` variants with all providers usable).

## Not done / left
- permagent-harvester config.rs keeps its own qwen2.5:7b and 127.0.0.1:11434 defaults (separate crate, own config file).
- mesh/mod.rs LOCAL_ENDPOINT stays (default of PERMAGENT_OLLAMA_HOST, a different knob).
- qwen38_split declarative JSON and knowledge row are kept (tests in goose-cli depend on the provider being declared); only its probe default and description changed.
- ios/PermagentMobile project.yml and pbxproj still carry DEVELOPMENT_TEAM Z58XSBM78S (outside this lane).
- spectral dependency: untouched. `git ls-remote https://github.com/make-tuned-unit/spectral HEAD` succeeds anonymously (credential helper disabled), so the repo looks public.
- identity_name_guard not run (needs cargo). Grep of the added diff lines for Jesse/Henry/Hank: no hits.
- Local signed builds now need APPLE_SIGNING_IDENTITY exported (documented in RELEASE.md), else ad-hoc signing.
- No guard hits belonging to the UI lane were seen.
