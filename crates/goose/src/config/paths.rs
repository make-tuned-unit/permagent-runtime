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

    /// Ontology file used by the Spectral Brain.
    pub fn brain_ontology() -> PathBuf {
        Self::brain_dir().join("ontology.toml")
    }
}
