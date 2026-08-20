use std::path::PathBuf;

pub struct Paths;

impl Paths {
    fn base_dir() -> PathBuf {
        if let Ok(test_root) = std::env::var("PERMAGENT_PATH_ROOT") {
            PathBuf::from(test_root)
        } else {
            dirs::home_dir()
                .expect("permagent requires a home dir")
                .join(".permagent")
        }
    }

    pub fn config_dir() -> PathBuf {
        Self::base_dir()
    }

    pub fn data_dir() -> PathBuf {
        Self::base_dir()
    }

    pub fn state_dir() -> PathBuf {
        Self::base_dir()
    }

    pub fn in_state_dir(subpath: &str) -> PathBuf {
        Self::state_dir().join(subpath)
    }

    pub fn in_config_dir(subpath: &str) -> PathBuf {
        Self::config_dir().join(subpath)
    }

    pub fn in_data_dir(subpath: &str) -> PathBuf {
        Self::data_dir().join(subpath)
    }

    pub fn logs_dir() -> PathBuf {
        Self::base_dir().join("logs")
    }

    pub fn spectral_dir() -> PathBuf {
        Self::base_dir().join("spectral")
    }

    pub fn spectral_db() -> PathBuf {
        Self::spectral_dir().join("permagent.db")
    }

    /// Directory for the Spectral Brain (knowledge graph + fingerprint store).
    pub fn brain_dir() -> PathBuf {
        Self::base_dir().join("brain")
    }

    /// Permagent-owned intake directory. Browser downloads (and, later, other
    /// intake flows) land here as files on disk; a metadata row per file lives
    /// in permagent.db (see [`crate::inbox`]). `disk_path` columns are stored
    /// relative to this directory.
    pub fn inbox_dir() -> PathBuf {
        Self::base_dir().join("inbox")
    }

    /// Ontology file used by the Spectral Brain.
    pub fn brain_ontology() -> PathBuf {
        Self::brain_dir().join("ontology.toml")
    }

    /// Directory holding the user's skills as portable `SKILL.md` folders (the
    /// open agentskills.io standard). This is the source-of-truth store the
    /// `skills` platform extension already discovers as a global skills dir, and
    /// the target the auto-skills loop writes learned skills into. Skills here
    /// are portable in and out of Claude Code, Cursor, Codex, and any other
    /// agentskills.io-compatible client.
    pub fn skills_dir() -> PathBuf {
        Self::base_dir().join("skills")
    }

    /// Generated Grow stills and Higgsfield downloads, keyed by project then
    /// card: `grow-media/<project_id>/<card_id>/`. Per-user data dir — never a
    /// repo path, never a hardcoded project.
    pub fn grow_media_dir() -> PathBuf {
        Self::base_dir().join("grow-media")
    }
}
