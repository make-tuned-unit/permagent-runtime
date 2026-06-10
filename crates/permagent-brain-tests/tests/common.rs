//! Shared test helpers for Brain-dependent integration tests.
//! This crate runs WITHOUT V8 (no code-mode feature) to avoid
//! the libc++abi symbol collision on Linux. See issue #190.
//!
//! Sanctioned raw `spectral::Brain` usage — test crate owns its runtime.
#![allow(dead_code)]

use spectral::Brain;
use std::sync::{Arc, OnceLock};

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

/// Shared Brain instance for context_builder tests.
/// Uses OnceLock to create once per test binary.
pub fn shared_context_builder_brain() -> Arc<Brain> {
    static BRAIN: OnceLock<Arc<Brain>> = OnceLock::new();
    BRAIN
        .get_or_init(|| {
            let temp = tempfile::tempdir().expect("tempdir");
            let brain_path = temp.path().join("brain");
            let ontology_path = temp.path().join("ontology.toml");
            std::fs::write(&ontology_path, ONTOLOGY_TOML).unwrap();
            let _ = Box::leak(Box::new(temp));
            Arc::new(
                Brain::builder()
                    .data_dir(&brain_path)
                    .ontology_path(&ontology_path)
                    .device_id(spectral::DeviceId::from_descriptor("test-context-builder"))
                    .build()
                    .expect("test brain"),
            )
        })
        .clone()
}

/// Shared Brain instance for ingestion tests.
pub fn shared_ingestion_brain() -> Arc<Brain> {
    static BRAIN: OnceLock<Arc<Brain>> = OnceLock::new();
    BRAIN
        .get_or_init(|| {
            let temp = tempfile::tempdir().expect("tempdir");
            let brain_path = temp.path().join("brain");
            let ontology_path = temp.path().join("ontology.toml");
            std::fs::write(&ontology_path, ONTOLOGY_TOML).unwrap();
            let _ = Box::leak(Box::new(temp));
            Arc::new(
                Brain::builder()
                    .data_dir(&brain_path)
                    .ontology_path(&ontology_path)
                    .device_id(spectral::DeviceId::from_descriptor("test-ingestion"))
                    .build()
                    .expect("test brain"),
            )
        })
        .clone()
}
