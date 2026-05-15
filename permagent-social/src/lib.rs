//! Permagent Social: social media scheduling for Permagent.
//!
//! Architecture:
//! - `models/` defines domain types
//! - `db/` handles SQLite + migrations
//! - `adapters/` provides per-platform implementations (Bluesky, day 2-3)
//! - `scheduler/` runs the posting worker (day 4)
//! - `http/` exposes the HTTP API (day 5)

pub mod db;
pub mod error;
pub mod models;

pub use error::{Error, Result};
