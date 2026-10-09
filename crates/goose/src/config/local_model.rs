//! The local (Ollama) model Permagent's cheap background workers use when the
//! user has not chosen one.
//!
//! One config key, one default, one place. Everything that used to hard-code
//! a local model id (Reader summaries, Concierge drafts, the Librarian
//! availability check, the role-default resolver) reads [`local_default_model`].
//!
//! Resolution order:
//! 1. `local_default_model` in `~/.permagent/config.yaml` (or the
//!    `LOCAL_DEFAULT_MODEL` environment variable).
//! 2. [`DEFAULT_LOCAL_MODEL`], a small widely-available instruct model. It is
//!    a default, not a requirement: pull it, or set the key to a model you have.

/// Config key holding the local model id (an Ollama tag such as `qwen2.5:7b`).
pub const LOCAL_DEFAULT_MODEL_KEY: &str = "local_default_model";

/// The default local model id. The only place this literal lives.
pub const DEFAULT_LOCAL_MODEL: &str = "qwen2.5:7b";

/// Pure: resolve the local model from a key reader.
pub fn resolve_local_default_model(read: impl Fn(&str) -> Option<String>) -> String {
    read(LOCAL_DEFAULT_MODEL_KEY)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_LOCAL_MODEL.to_string())
}

/// The local default model, read from the process-global config.
pub fn local_default_model() -> String {
    let config = crate::config::Config::global();
    resolve_local_default_model(|key| config.get_param::<String>(key).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_blank_uses_the_default() {
        assert_eq!(resolve_local_default_model(|_| None), DEFAULT_LOCAL_MODEL);
        assert_eq!(
            resolve_local_default_model(|_| Some("  ".into())),
            DEFAULT_LOCAL_MODEL
        );
    }

    #[test]
    fn a_configured_value_wins_and_is_trimmed() {
        assert_eq!(
            resolve_local_default_model(
                |k| (k == LOCAL_DEFAULT_MODEL_KEY).then(|| " llama3.2:3b ".to_string())
            ),
            "llama3.2:3b"
        );
    }
}
