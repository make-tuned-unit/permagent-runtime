//! First-user Brain setup for the serving daemon, before AppState mounts it.
//! Existing directories (including interrupted/partial setups) are never repaired
//! implicitly: their ontology and databases may be user-owned.
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

pub const BASELINE_ONTOLOGY: &str = include_str!("brain_default_ontology.toml");
#[derive(Debug, PartialEq, Eq)]
pub enum BootstrapOutcome {
    Created,
    ExistingOntology,
    ExistingUninitialized,
}

pub fn initialize_fresh_brain(data_dir: &Path) -> io::Result<BootstrapOutcome> {
    let brain_dir = data_dir.join("brain");
    let ontology = brain_dir.join("ontology.toml");
    // create_dir is exclusive: never follow a pre-existing brain symlink or
    // write into an existing directory, even when it looks empty.
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&brain_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Ok(if ontology.is_file() {
                BootstrapOutcome::ExistingOntology
            } else {
                BootstrapOutcome::ExistingUninitialized
            });
        }
        Err(error) => return Err(error),
    }
    let temporary = brain_dir.join(".ontology.bootstrap.tmp");
    let mut owns_temporary = false;
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        owns_temporary = true;
        file.write_all(BASELINE_ONTOLOGY.as_bytes())?;
        file.sync_all()?;
        // Publish complete bytes atomically without ever replacing a file that
        // another process may have created. Temp and final are on one volume.
        fs::hard_link(&temporary, &ontology)?;
        Ok(BootstrapOutcome::Created)
    })();
    if owns_temporary {
        let _ = fs::remove_file(&temporary);
    }
    if result.is_err() {
        // Only remove our directory if still empty; never remove other files.
        let _ = fs::remove_dir(&brain_dir);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "permagent-brain-bootstrap-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn fresh_setup_is_complete_private_and_idempotent() {
        let fixture = Fixture::new();
        assert_eq!(
            initialize_fresh_brain(&fixture.0).unwrap(),
            BootstrapOutcome::Created
        );
        let brain = fixture.0.join("brain");
        assert_eq!(
            fs::read_to_string(brain.join("ontology.toml")).unwrap(),
            BASELINE_ONTOLOGY
        );
        assert_eq!(fs::read_dir(&brain).unwrap().count(), 1);
        assert_eq!(
            initialize_fresh_brain(&fixture.0).unwrap(),
            BootstrapOutcome::ExistingOntology
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&brain).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(brain.join("ontology.toml"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn existing_ontology_and_database_are_untouched_even_when_invalid() {
        let fixture = Fixture::new();
        let brain = fixture.0.join("brain");
        fs::create_dir(&brain).unwrap();
        fs::write(brain.join("ontology.toml"), b"user data, even invalid").unwrap();
        fs::write(brain.join("memory.db"), b"existing database").unwrap();
        assert_eq!(
            initialize_fresh_brain(&fixture.0).unwrap(),
            BootstrapOutcome::ExistingOntology
        );
        assert_eq!(
            fs::read(brain.join("ontology.toml")).unwrap(),
            b"user data, even invalid"
        );
        assert_eq!(
            fs::read(brain.join("memory.db")).unwrap(),
            b"existing database"
        );
    }
    #[test]
    fn interrupted_or_existing_uninitialized_directories_require_explicit_repair() {
        let fixture = Fixture::new();
        let brain = fixture.0.join("brain");
        fs::create_dir(&brain).unwrap();
        assert_eq!(
            initialize_fresh_brain(&fixture.0).unwrap(),
            BootstrapOutcome::ExistingUninitialized
        );
        fs::write(brain.join(".ontology.bootstrap.tmp"), b"partial").unwrap();
        fs::write(brain.join("memory.db"), b"keep").unwrap();
        assert_eq!(
            initialize_fresh_brain(&fixture.0).unwrap(),
            BootstrapOutcome::ExistingUninitialized
        );
        assert!(!brain.join("ontology.toml").exists());
        assert_eq!(
            fs::read(brain.join(".ontology.bootstrap.tmp")).unwrap(),
            b"partial"
        );
        assert_eq!(fs::read(brain.join("memory.db")).unwrap(), b"keep");
    }
    #[test]
    fn existing_file_is_not_replaced_with_directory() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("brain"), b"keep").unwrap();
        assert_eq!(
            initialize_fresh_brain(&fixture.0).unwrap(),
            BootstrapOutcome::ExistingUninitialized
        );
        assert_eq!(fs::read(fixture.0.join("brain")).unwrap(), b"keep");
    }
    #[cfg(unix)]
    #[test]
    fn existing_symlink_never_receives_an_ontology() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        std::os::unix::fs::symlink(&outside.0, fixture.0.join("brain")).unwrap();
        assert_eq!(
            initialize_fresh_brain(&fixture.0).unwrap(),
            BootstrapOutcome::ExistingUninitialized
        );
        assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
    }
    #[test]
    fn missing_data_parent_is_reported_not_guessed() {
        let fixture = Fixture::new();
        assert!(initialize_fresh_brain(&fixture.0.join("missing")).is_err());
        assert!(!fixture.0.join("missing").exists());
    }
}
