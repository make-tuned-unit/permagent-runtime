//! Keyring-first storage for the Gmail OAuth token.
//!
//! Historically the Gmail integration persisted its OAuth token (access +
//! refresh token, client secret) as plaintext JSON at
//! `~/.permagent/secrets/gmail_token.json` while every other credential in the
//! app was keyring-first. This module closes that gap by storing the token
//! JSON under the [`GMAIL_OAUTH_TOKEN_KEY`] secret via [`Config`] — the same
//! keyring-with-file-fallback machinery used for provider API keys and MCP
//! OAuth credentials (`oauth::persist::GooseCredentialStore`).
//!
//! The `permagent-gmail-mcp` extension subprocess receives the token through
//! the standard `env_keys` secret-injection path (`GMAIL_OAUTH_TOKEN` is
//! resolved from the keyring at spawn time and exported as an environment
//! variable), with the legacy `GMAIL_TOKEN_PATH` file kept only as a
//! transitional fallback.
//!
//! Migration is silent and happens on first read: if the legacy plaintext
//! file exists, its contents are moved into the keyring and the file is
//! removed. If the secret store rejects the write outright, the legacy file
//! is instead rewritten atomically with `0600` permissions so it is never
//! left world-readable.

use std::path::{Path, PathBuf};

use super::base::{Config, ConfigError};
use super::paths::Paths;
use super::secure_fs;

/// Secret key under which the Gmail OAuth token JSON is stored. Doubles as
/// the environment variable name injected into the `permagent-gmail-mcp`
/// extension via `env_keys`.
pub const GMAIL_OAUTH_TOKEN_KEY: &str = "GMAIL_OAUTH_TOKEN";

/// Pre-keyring location of the plaintext token file (also the transitional
/// fallback consumed via `GMAIL_TOKEN_PATH` by older extension configs).
pub fn legacy_token_path() -> PathBuf {
    Paths::data_dir().join("secrets").join("gmail_token.json")
}

/// Store the Gmail OAuth token JSON, keyring-first.
///
/// On success any legacy plaintext file is removed. If the secret store
/// errors, the token is instead written atomically (`0600`) to the legacy
/// path so the integration keeps working without ever exposing a
/// world-readable window.
pub fn store_token(token_json: &str) -> Result<(), ConfigError> {
    store_token_with(Config::global(), &legacy_token_path(), token_json)
}

/// Load the Gmail OAuth token JSON, migrating a legacy plaintext file into
/// the keyring on first read.
pub fn load_token() -> Option<String> {
    load_token_with(Config::global(), &legacy_token_path())
}

/// Whether a Gmail OAuth token is available (keyring or legacy file).
/// Triggers the same silent migration as [`load_token`].
pub fn token_present() -> bool {
    load_token().is_some()
}

/// Delete the Gmail OAuth token from the keyring and remove any legacy file.
pub fn delete_token() -> Result<(), ConfigError> {
    delete_token_with(Config::global(), &legacy_token_path())
}

/// [`store_token`] with injectable config and legacy path (used by tests).
pub fn store_token_with(
    config: &Config,
    legacy_path: &Path,
    token_json: &str,
) -> Result<(), ConfigError> {
    match config.set_secret(GMAIL_OAUTH_TOKEN_KEY, &token_json.to_string()) {
        Ok(()) => {
            if legacy_path.exists() {
                if let Err(e) = std::fs::remove_file(legacy_path) {
                    tracing::warn!(
                        "Gmail token stored to keyring but failed to remove legacy file {}: {e}",
                        legacy_path.display()
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            tracing::warn!(
                "Failed to store Gmail token in secret storage ({e}); \
                 falling back to private file at {}",
                legacy_path.display()
            );
            if let Some(parent) = legacy_path.parent() {
                secure_fs::ensure_private_dir(parent)?;
            }
            secure_fs::write_private_file(legacy_path, token_json.as_bytes())?;
            Ok(())
        }
    }
}

/// [`load_token`] with injectable config and legacy path (used by tests).
pub fn load_token_with(config: &Config, legacy_path: &Path) -> Option<String> {
    if let Ok(token) = config.get_secret::<String>(GMAIL_OAUTH_TOKEN_KEY) {
        return Some(token);
    }
    migrate_legacy_file(config, legacy_path)
}

/// [`delete_token`] with injectable config and legacy path (used by tests).
pub fn delete_token_with(config: &Config, legacy_path: &Path) -> Result<(), ConfigError> {
    let result = config.delete_secret(GMAIL_OAUTH_TOKEN_KEY);
    if legacy_path.exists() {
        if let Err(e) = std::fs::remove_file(legacy_path) {
            tracing::warn!(
                "Failed to remove legacy Gmail token file {}: {e}",
                legacy_path.display()
            );
        }
    }
    result
}

/// Silent one-time migration: move a legacy plaintext token file into the
/// secret store and delete it. If the store rejects the write, rewrite the
/// file atomically with `0600` permissions instead so it is never left
/// world-readable.
fn migrate_legacy_file(config: &Config, legacy_path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(legacy_path).ok()?;

    match config.set_secret(GMAIL_OAUTH_TOKEN_KEY, &raw) {
        Ok(()) => {
            if let Err(e) = std::fs::remove_file(legacy_path) {
                tracing::warn!(
                    "Gmail token migrated to keyring but failed to remove legacy file {}: {e}",
                    legacy_path.display()
                );
            } else {
                tracing::info!(
                    "Migrated Gmail OAuth token from {} into secret storage",
                    legacy_path.display()
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                "Secret storage unavailable for Gmail token migration ({e}); \
                 re-securing legacy file at {}",
                legacy_path.display()
            );
            if let Some(parent) = legacy_path.parent() {
                let _ = secure_fs::ensure_private_dir(parent);
            }
            if let Err(e) = secure_fs::write_private_file(legacy_path, raw.as_bytes()) {
                tracing::warn!(
                    "Failed to re-secure legacy Gmail token file {}: {e}",
                    legacy_path.display()
                );
            }
        }
    }

    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &TempDir) -> Config {
        Config::new_with_file_secrets(
            dir.path().join("config.yaml"),
            dir.path().join("secrets.yaml"),
        )
        .unwrap()
    }

    const TOKEN_JSON: &str =
        r#"{"token":"ya29.a0Af","refresh_token":"1//0gabc","client_id":"id","client_secret":"cs"}"#;

    #[test]
    fn store_then_load_roundtrips_via_secret_store() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let legacy = dir.path().join("secrets").join("gmail_token.json");

        store_token_with(&config, &legacy, TOKEN_JSON).unwrap();

        assert_eq!(
            load_token_with(&config, &legacy).as_deref(),
            Some(TOKEN_JSON)
        );
        // Nothing was written to the legacy plaintext location.
        assert!(!legacy.exists());
    }

    #[test]
    fn store_removes_existing_legacy_plaintext_file() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let legacy_dir = dir.path().join("secrets");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = legacy_dir.join("gmail_token.json");
        std::fs::write(&legacy, "stale-plaintext").unwrap();

        store_token_with(&config, &legacy, TOKEN_JSON).unwrap();

        assert!(!legacy.exists(), "legacy plaintext must be removed");
        assert_eq!(
            load_token_with(&config, &legacy).as_deref(),
            Some(TOKEN_JSON)
        );
    }

    #[test]
    fn load_migrates_legacy_file_into_secret_store_and_removes_it() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let legacy_dir = dir.path().join("secrets");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = legacy_dir.join("gmail_token.json");
        std::fs::write(&legacy, TOKEN_JSON).unwrap();

        // First read returns the token and migrates.
        assert_eq!(
            load_token_with(&config, &legacy).as_deref(),
            Some(TOKEN_JSON)
        );
        assert!(!legacy.exists(), "plaintext file removed after migration");

        // Subsequent reads come from the secret store.
        assert_eq!(
            config
                .get_secret::<String>(GMAIL_OAUTH_TOKEN_KEY)
                .ok()
                .as_deref(),
            Some(TOKEN_JSON)
        );
        assert_eq!(
            load_token_with(&config, &legacy).as_deref(),
            Some(TOKEN_JSON)
        );
    }

    #[test]
    fn load_returns_none_when_nothing_stored() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let legacy = dir.path().join("secrets").join("gmail_token.json");

        assert_eq!(load_token_with(&config, &legacy), None);
        assert!(!token_present_with(&config, &legacy));
    }

    #[test]
    fn delete_removes_secret_and_legacy_file() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let legacy_dir = dir.path().join("secrets");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = legacy_dir.join("gmail_token.json");
        std::fs::write(&legacy, "leftover").unwrap();

        store_token_with(&config, &legacy, TOKEN_JSON).unwrap();
        std::fs::write(&legacy, "leftover-again").unwrap();

        delete_token_with(&config, &legacy).unwrap();

        assert!(!legacy.exists());
        assert_eq!(load_token_with(&config, &legacy), None);
    }

    /// Test-only presence helper mirroring [`token_present`].
    fn token_present_with(config: &Config, legacy_path: &Path) -> bool {
        load_token_with(config, legacy_path).is_some()
    }
}
