pub mod agent_identity;
pub mod base;
pub mod declarative_providers;
pub mod dev_roots;
#[cfg(test)]
mod dev_signing_guard;
mod experiments;
pub mod extensions;
pub mod gmail_oauth;
pub mod goose_mode;
#[cfg(test)]
mod identity_name_guard;
mod migrations;
pub mod paths;
pub mod permission;
pub mod search_path;
pub mod secret_source;
pub mod secure_fs;
pub mod signup_nanogpt;
pub mod signup_openrouter;
pub mod signup_tetrate;
pub mod worker_probe;

pub use crate::agents::ExtensionConfig;
pub use base::{Config, ConfigError};
pub use declarative_providers::DeclarativeProviderConfig;
pub use experiments::ExperimentManager;
pub use extensions::{
    extension_is_grantable, get_all_extension_names, get_all_extensions, get_enabled_extensions,
    get_extension_by_name, get_warnings, is_extension_enabled, name_to_key,
    narrow_extensions_for_agent, remove_extension, resolve_extensions_for_agent,
    resolve_extensions_for_new_session, set_extension, set_extension_enabled, ExtensionEntry,
};
pub use goose_mode::GooseMode;
pub use permission::PermissionManager;
pub use secret_source::{SecretSource, SecretSourceError};
pub use signup_nanogpt::configure_nanogpt;
pub use signup_openrouter::configure_openrouter;
pub use signup_tetrate::configure_tetrate;

pub use extensions::DEFAULT_DISPLAY_NAME;
pub use extensions::DEFAULT_EXTENSION;
pub use extensions::DEFAULT_EXTENSION_DESCRIPTION;
pub use extensions::DEFAULT_EXTENSION_TIMEOUT;

pub use crate::workspace_trust::{
    is_workspace_trusted, list_trusted_workspaces, trust_workspace, untrust_workspace,
    WorkspaceTrustError, WorkspaceTrustStore,
};

/// The Ollama endpoint the local **batch** workers (Librarian describe/annotate/
/// entity passes, Reader summarize) talk to. Defaults to a local Ollama; set
/// `PERMAGENT_OLLAMA_HOST` to offload this heavy, latency-tolerant LLM work to a
/// bigger or pooled machine — the first, verified increment of the batch tier of
/// the mesh-inference epic (#306). Fully reversible: unset ⇒ localhost, i.e.
/// today's behavior exactly. A trailing slash is trimmed so callers can append
/// `/api/generate` uniformly.
/// Resolution order is ENV first, then `~/.permagent/config.yaml` — which is
/// what `Config::get_param` does for the uppercased key. An existing
/// `PERMAGENT_OLLAMA_HOST` export therefore behaves exactly as before; the
/// config file is a new, additional place to set it.
///
/// That second route is the one that actually works on macOS. The daemon is
/// spawned by Permagent.app, which launchd starts without ever reading a shell
/// profile, so an `export` in `.zshrc` can never reach it. The only env route
/// was `launchctl setenv`, which does not survive a reboot — meaning the
/// Librarian would silently fall back to `http://localhost:11434` and retry a
/// port with nothing behind it. A config key is durable.
pub fn ollama_host() -> String {
    resolve_ollama_host(
        Config::global()
            .get_param::<String>("PERMAGENT_OLLAMA_HOST")
            .ok(),
    )
}

/// Optional Librarian-only inference endpoint (`PERMAGENT_LIBRARIAN_ENDPOINT`).
///
/// When set, the Librarian's describe passes go here instead of the mesh
/// pool / `PERMAGENT_OLLAMA_HOST`, so a larger model served by a different
/// engine (today: a two-machine `llama-server` split of Qwen3.8-27B) can do
/// the nightly archiving without touching the app's other Ollama uses. Unset
/// means today's behaviour exactly. Trailing slashes are stripped.
pub fn librarian_endpoint() -> Option<String> {
    Config::global()
        .get_param::<String>("PERMAGENT_LIBRARIAN_ENDPOINT")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

/// Wire protocol spoken by [`librarian_endpoint`]: `"llamacpp"` (default —
/// llama-server's OpenAI-compatible `/v1/chat/completions`) or `"ollama"`
/// (an Ollama `/api/generate` host that is not the app-wide one).
pub fn librarian_backend() -> String {
    Config::global()
        .get_param::<String>("PERMAGENT_LIBRARIAN_BACKEND")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "llamacpp".to_string())
}

/// Is the Apple on-device Foundation Models backend allowed to serve work
/// (`PERMAGENT_APPLE_FM_ENABLED`)?
///
/// Defaults to **on**: it costs nothing per call and keeps the prompt on the
/// machine, and every consumer of it falls back cleanly when it cannot serve.
/// The escape hatch exists for the case the default cannot cover — preferring
/// a larger local model's output quality over a free one's.
///
/// This is a permission, not a promise. It says the backend may be *tried*;
/// whether it can actually serve a given call is probed against the running
/// system at call time.
pub fn apple_fm_enabled() -> bool {
    Config::global()
        .get_param::<String>("PERMAGENT_APPLE_FM_ENABLED")
        .ok()
        .map(|s| {
            !matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

/// Explicit path to the `permagent-applefm` sidecar
/// (`PERMAGENT_APPLE_FM_SIDECAR`), for a layout the default search does not
/// cover. Unset means the ordinary search: next to the running executable,
/// then the source tree.
pub fn apple_fm_sidecar_override() -> Option<String> {
    Config::global()
        .get_param::<String>("PERMAGENT_APPLE_FM_SIDECAR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Pure core of [`ollama_host`], split out so it is unit-testable without
/// touching the process-global environment (env-mutating tests flake under
/// parallel `cargo test`).
fn resolve_ollama_host(raw: Option<String>) -> String {
    raw.map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://localhost:11434".to_string())
}

#[cfg(test)]
mod ollama_host_tests {
    use super::resolve_ollama_host;

    #[test]
    fn defaults_to_localhost_when_unset_or_blank() {
        assert_eq!(resolve_ollama_host(None), "http://localhost:11434");
        assert_eq!(
            resolve_ollama_host(Some("   ".to_string())),
            "http://localhost:11434"
        );
    }

    #[test]
    fn uses_configured_host_and_trims() {
        assert_eq!(
            resolve_ollama_host(Some("http://mini.local:11434/".to_string())),
            "http://mini.local:11434"
        );
        assert_eq!(
            resolve_ollama_host(Some("  http://box:11434 ".to_string())),
            "http://box:11434"
        );
    }
}
