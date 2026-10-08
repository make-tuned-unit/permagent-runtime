# Haiku 5.5 additive registration audit

The initial audit was source-only and performed no daemon requests, model calls, credential reads, configuration writes or restart. The coordinator subsequently applied the additive registration and verified its availability; that separate runtime evidence is recorded at the end. Formatter compatibility below remains source-derived unless explicitly verified.

## REST creation cannot express key-reference + unfiltered models

`POST /config/custom-providers` accepts `UpdateCustomProviderRequest` (`config_management.rs:228,1370`). It does not accept `api_key_env`, `skip_canonical_filtering`, or per-model context metadata. Unknown JSON fields are not the missing feature: creation ignores them. The implementation creates a new generated secret from the supplied literal `api_key` and forces `skip_canonical_filtering=false` (`declarative_providers.rs:221`). Therefore there is no honest REST-only POST payload meeting the requested existing-key-reference and skip-filter requirements. An empty key would create an unusable new credential; `requires_auth:false` does not help because Anthropic's custom constructor still reads its configured key.

## Supported file-plus-PUT registration mechanism

Create a unique additive file under the daemon's `Paths::config_dir()/custom_providers/`, normally `~/.permagent/custom_providers/custom_anthropic_haiku55.json`. Refuse overwrite if that name already exists. No secret value is copied; api_key_env refers to the existing secret. Minimal schema-valid content:

```json
{
  "name": "custom_anthropic_haiku55",
  "engine": "anthropic",
  "display_name": "Anthropic — Haiku 5.5",
  "description": "Additive Haiku 5.5 availability; existing defaults unchanged",
  "api_key_env": "ANTHROPIC_API_KEY",
  "base_url": "https://api.anthropic.com",
  "models": [{"name": "claude-haiku-5-5", "context_limit": 128000}],
  "supports_streaming": true,
  "requires_auth": true,
  "catalog_provider_id": "anthropic",
  "skip_canonical_filtering": true
}
```

The 128,000 context value here is a conservative temporary runtime allowance. The model supports a 1,000,000-token context. The supported PUT below rebuilds model metadata at 128,000 regardless of the file value, so it would not preserve a 1,000,000-token value. The model's output maximum is separately 128,000 tokens; this file has no maximum-output or request-parameters schema.

Then authenticated `PUT /config/custom-providers/custom_anthropic_haiku55` with:

```json
{
  "engine": "anthropic_compatible",
  "display_name": "Anthropic — Haiku 5.5",
  "api_url": "https://api.anthropic.com",
  "api_key": "",
  "models": ["claude-haiku-5-5"],
  "supports_streaming": true,
  "headers": null,
  "requires_auth": true,
  "catalog_provider_id": "anthropic",
  "base_path": null
}
```

`update_custom_provider` loads that file, retains its existing api_key_env and skip_canonical_filtering, does not set_secret when api_key is empty, rewrites only that custom provider file, and refreshes the custom registry (`declarative_providers.rs:276–335`, `config_management.rs:1459`). It changes neither global GOOSE_PROVIDER/GOOSE_MODEL nor any role routes. Existing provider instances are Arcs; the function does not replace the daemon default-provider instance. Do not call `/config/set_provider` or `/config/model-route` merely to make this additive provider appear. Do not use `/config/reload`: it hot-swaps the default provider and is not a custom-registry refresh.

**Shared-daemon caveat:** `providers/init.rs:136` refresh removes all custom registry entries and then reloads them under a separate lock. This is not an atomic additive-only registration. A concurrent new custom-provider request may see a transient absence; malformed/unreadable JSON in any existing custom provider can abort the reload and leave all custom registry entries missing. Read-only validate every current custom JSON and name uniqueness first. If zero disruption to shared custom-provider users is required, defer registration to a coordinated quiet window or updated isolated daemon. No source evidence supports promising this operation is side-effect-free to every concurrent custom-provider lookup.

After registration, GET `/config/providers` can show its static one-model metadata without inference. GET `/config/providers/custom_anthropic_haiku55/models` makes a real provider model-list API request: custom Anthropic first calls `/v1/models` and returns the entire available model list on success. The custom models array is a 404 fallback, **not** an allowlist. skip_canonical_filtering allows newly returned Haiku through the old bundled registry. Account availability still has to be verified; registration alone is not an inference pass.

## Old formatter compatibility boundary

On the old formatter, unknown Haiku 5.5 gets no explicit thinking field by default. The server enables adaptive thinking with its own medium default, which is legal. Existing parsing handles thinking blocks by type and preserves signature-only thinking. With no temperature and no explicit/legacy enabled thinking, a simple user-ending request can be accepted.

Preconditions for any later inference probe: no inherited GOOSE_TEMPERATURE env value (other than legal 1; omission preferred), no explicit `thinking_type=enabled` or global CLAUDE_THINKING_TYPE=enabled, and no CLAUDE_THINKING_ENABLED legacy flag. That combination would cause the old formatter to send rejected budget_tokens. The old unsupported adaptive preference is dropped, which still leaves server adaptive default. Old `disabled` is also omitted and therefore **does not actually disable Haiku 5.5 thinking**. top_p/top_k are not forwarded by this formatter. Verify per-request max_tokens≤128000; legacy default 4096 may exhaust on thinking and leave no text. Do not change global caps/temperature/thinking merely to enable this provider. The declarative schema does not provide per-model request_params; ModelConfig's predefined-model env can, but that requires process environment changes/restart and is not this route.

Registration does not fix Haiku's prefix-bound thinking requirements for rewritten system/tools/history, assistant-prefill rejection, cost-router metadata, or real-account availability. Do not call it fully compatible because it appears in Settings. Existing unknown-price fallback may estimate conservatively; no inexpensive flat pricing is asserted.

## Exact output maximum

Anthropic's model-specific prompting guide explicitly states `max_tokens` can reach **128,000**, confirming the source tests' 128000 value rather than 131072. [Official Haiku 5.5 prompting guide](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/prompting-claude-haiku-5-5)

The same guide confirms thinking can be disabled at low/medium/high effort; xhigh/max with disabled is rejected. The new scoped formatter's disabled branch omits effort and thus uses the legal model default. Existing low/medium/high/max parsing does not yet expose xhigh; that is separate from availability registration.

## Applied and verified by coordinator

The coordinator applied the additive file-plus-PUT route to the running daemon after validating the existing custom-provider JSON. `GET /config/providers` reports `custom_anthropic_haiku55` as configured. Anthropic's actual model inventory returned 14 models, including `claude-haiku-5-5`.

The default-provider identity and SHA-256 of `config.yaml` were unchanged. No key value was read or copied; the declaration references `ANTHROPIC_API_KEY`. The live declaration is `~/.permagent/custom_providers/custom_anthropic_haiku55.json`.

Computer Use verified **Settings → Models → Chat picker → Anthropic — Haiku 5.5**, with selectable **Claude Haiku 5.5** (`claude-haiku-5-5`). No model selection or default was changed.

This verifies registration, account inventory and picker availability. Inference and long-session compatibility remain unverified. The built-in Anthropic catalogue source fix still needs validation against a rebuilt daemon.
