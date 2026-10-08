//! A compile-time, debug-only target for an independently managed review daemon.
//! Runtime environment never redirects a shipped app's authentication or writes.
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const REVIEW_IDENTIFIER: &str = "ai.permagent.record-review-20261007";

#[derive(Debug, Clone)]
pub struct RuntimeTarget {
    pub root: PathBuf,
    pub port: u16,
    pub review: bool,
}

impl RuntimeTarget {
    pub fn parse(
        root: Option<&str>,
        port: Option<&str>,
        debug: bool,
        home: &Path,
    ) -> Result<Self, String> {
        match (root, port) {
            (None, None) => Ok(Self {
                root: home.join(".permagent"),
                port: 3001,
                review: false,
            }),
            (Some(root), Some(port)) => {
                if !debug {
                    return Err("review target overrides are forbidden in release builds".into());
                }
                let root = PathBuf::from(root);
                if !root.is_absolute()
                    || root
                        .components()
                        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
                {
                    return Err(
                        "review root must be an absolute path without dot components".into(),
                    );
                }
                let production = home.join(".permagent");
                if root == home
                    || production.starts_with(&root)
                    || root.starts_with(&production)
                    || root.components().any(|c| c.as_os_str() == ".permagent")
                {
                    return Err(
                        "review root must be separate from the production data directory".into(),
                    );
                }
                // This purpose-built artifact must not redirect writes to a broad
                // workspace/home folder accidentally supplied as the root.
                if root.file_name().and_then(|s| s.to_str()) != Some("record-qa-runtime") {
                    return Err(
                        "review root must be the dedicated record-qa-runtime directory".into(),
                    );
                }
                let port = port.parse::<u16>().map_err(|_| "invalid review port")?;
                if port < 1024 || port == 3001 {
                    return Err("review port must be an unprivileged non-production port".into());
                }
                Ok(Self {
                    root,
                    port,
                    review: true,
                })
            }
            _ => Err(
                "PERMAGENT_REVIEW_ROOT and PERMAGENT_REVIEW_PORT must be compiled together".into(),
            ),
        }
    }

    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address())
    }
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url())
    }
    pub fn token_path(&self) -> PathBuf {
        self.root.join("secrets/daemon_token.json")
    }

    pub fn validate_review_root(&self) -> Result<(), String> {
        if !self.review {
            return Ok(());
        }
        let canonical = self
            .root
            .canonicalize()
            .map_err(|e| format!("review root must already exist: {e}"))?;
        if canonical != self.root || !canonical.is_dir() {
            return Err("review root must be a canonical directory, not a symlink".into());
        }
        Ok(())
    }
}

pub fn target() -> &'static RuntimeTarget {
    static TARGET: OnceLock<RuntimeTarget> = OnceLock::new();
    TARGET.get_or_init(|| {
        let home = std::env::var("HOME").expect("HOME is required for native runtime paths");
        RuntimeTarget::parse(
            option_env!("PERMAGENT_REVIEW_ROOT"),
            option_env!("PERMAGENT_REVIEW_PORT"),
            cfg!(debug_assertions),
            Path::new(&home),
        )
        .expect("invalid compiled native review target")
    })
}

pub fn initialize(identifier: &str) -> Result<(), String> {
    let target = target();
    if target.review && identifier != REVIEW_IDENTIFIER {
        return Err("isolated review target requires its dedicated app bundle identifier".into());
    }
    target.validate_review_root()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(root: Option<&str>, port: Option<&str>, debug: bool) -> Result<RuntimeTarget, String> {
        RuntimeTarget::parse(root, port, debug, Path::new("/Users/fixture"))
    }
    #[test]
    fn production_defaults_are_unchanged() {
        let target = parse(None, None, false).unwrap();
        assert_eq!(target.root, Path::new("/Users/fixture/.permagent"));
        assert_eq!(target.base_url(), "http://127.0.0.1:3001");
        assert!(!target.review);
    }
    #[test]
    fn review_target_is_coherent_and_loopback_only() {
        let target = parse(Some("/tmp/record-qa-runtime"), Some("3002"), true).unwrap();
        assert_eq!(
            target.token_path(),
            Path::new("/tmp/record-qa-runtime/secrets/daemon_token.json")
        );
        assert_eq!(target.url("/api/inbox"), "http://127.0.0.1:3002/api/inbox");
        assert!(target.review);
    }
    #[test]
    fn review_overrides_fail_closed() {
        for root in [
            "relative/record-qa-runtime",
            "/",
            "/Users/fixture",
            "/Users/fixture/.permagent/record-qa-runtime",
            "/tmp/../record-qa-runtime",
            "/tmp/not-a-dedicated-root",
        ] {
            assert!(
                parse(Some(root), Some("3002"), true).is_err(),
                "accepted {root}"
            );
        }
        for port in ["0", "80", "3001", "65536", "localhost:3002", ""] {
            assert!(parse(Some("/tmp/record-qa-runtime"), Some(port), true).is_err());
        }
        assert!(parse(Some("/tmp/record-qa-runtime"), None, true).is_err());
        assert!(parse(None, Some("3002"), true).is_err());
        assert!(parse(Some("/tmp/record-qa-runtime"), Some("3002"), false).is_err());
    }
}
