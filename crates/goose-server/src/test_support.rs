//! Shared test support for the goose-server lib test binary.
//!
//! Every `#[cfg(test)]` module in this crate compiles into a *single* test
//! binary, so they all share one process. The session-manager pool behind
//! `AppState` is a process-global `LazyLock` (`SESSION_STORAGE` in
//! `crates/goose/src/session/session_manager.rs`): on first access it captures
//! `db_path = Paths::spectral_db()` (which reads `PERMAGENT_PATH_ROOT`) and
//! `create_dir_all`s that path's parent. That pin happens **once per process**,
//! at whichever test builds an `AppState` first.
//!
//! Historically each AppState-building test minted its own `tempfile::tempdir()`
//! and pointed `PERMAGENT_PATH_ROOT` at it. The first such test's tempdir was
//! deleted the moment that test returned (its `TempDir` dropped), while the
//! process-global pool kept pointing at it, so the next test's very first query
//! opened a sqlite connection under a **vanished parent directory** →
//! `"unable to open database file"` (#840/#843/#856/#858). It *flaked* rather
//! than always failing because per-platform run order and lazy-connection reuse
//! decided whether a fresh connection actually had to be opened after the drop.
//!
//! The fix (the #843 pattern, hoisted to a shared helper): one tempdir owned by
//! a `static`, so it lives for the whole process and is **never dropped between
//! tests**, with the db/brain/inbox/logs parents created up front. Every
//! AppState-building test calls [`test_root`] *before* building an `AppState`,
//! so whichever runs first pins the global pool to a directory that outlives
//! every test. `HOME` is pointed at the same root because the project-documents
//! write path resolves under `~/.permagent/…` via `dirs::home_dir()` (an
//! inherited #677 wart, not `PERMAGENT_PATH_ROOT`) — doing it once here keeps
//! stray artifacts out of the real home. Tests that need an *isolated* HOME
//! (e.g. the findings ledger / identity `agent.yaml`, both keyed off HOME)
//! still override HOME per-test via their own `env_lock` guard; only
//! `PERMAGENT_PATH_ROOT` — the global-pool DB root — must resolve to this
//! shared, process-lifetime directory.

use std::path::PathBuf;
use std::sync::LazyLock;

/// Process-lifetime `PERMAGENT_PATH_ROOT` shared by every AppState-building
/// test in this binary. Owned by this `static` so the tempdir lives for the
/// whole process and is never dropped between tests.
static TEST_ROOT: LazyLock<tempfile::TempDir> = LazyLock::new(|| {
    let tmp = tempfile::tempdir().expect("create test PERMAGENT_PATH_ROOT");
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());
    std::env::set_var("HOME", tmp.path());
    // Pre-create every parent a lazily-opened db/brain/inbox/logs handle might
    // touch, so no connection can ever race a missing directory.
    for dir in [
        permagent::config::paths::Paths::spectral_dir(),
        permagent::config::paths::Paths::brain_dir(),
        permagent::config::paths::Paths::inbox_dir(),
        permagent::config::paths::Paths::logs_dir(),
    ] {
        std::fs::create_dir_all(&dir).expect("create test root subdir");
    }
    tmp
});

/// Force the shared, process-lifetime `PERMAGENT_PATH_ROOT` and return its path.
///
/// Call at the top of any test that builds an `AppState`, **before**
/// `AppState::new`. Tests that also set env via `env_lock` should pass the
/// returned path as their `PERMAGENT_PATH_ROOT` value (rather than a per-test
/// tempdir) so the global pool pins to this never-dropped directory.
pub fn test_root() -> PathBuf {
    TEST_ROOT.path().to_path_buf()
}

#[cfg(test)]
mod tests {
    /// No test binary may resolve the session database to the developer's real
    /// `~/.permagent`.
    ///
    /// This module is compiled into BOTH crate roots — the lib and the
    /// `permagentd` bin — because `main.rs` re-declares `mod routes`, so every
    /// route test exists twice, once per binary. `lib.rs` arms a `#[ctor]` that
    /// pins config to a temp root before any test body runs; `main.rs` did not,
    /// so in the bin binary nothing pinned the root before the process-global
    /// `SESSION_STORAGE` `LazyLock` captured `Paths::spectral_db()`. It captured
    /// the real one, and route tests wrote fixture projects into the user's own
    /// database — 40 of them, found in the Projects tab.
    ///
    /// `test_root()` is not enough on its own: it pins correctly, but only from
    /// the first test that calls it. Anything touching the global pool earlier
    /// has already fixed the path for the whole process. The `#[ctor]` is what
    /// makes the pin unconditional, and this test is what makes a missing
    /// `#[ctor]` fail loudly instead of silently writing to real data.
    #[test]
    fn the_session_db_never_resolves_to_the_real_permagent_dir() {
        let db = permagent::config::paths::Paths::spectral_db();
        let real = dirs::home_dir()
            .expect("home dir")
            .join(".permagent")
            .join("spectral");
        assert!(
            !db.starts_with(&real),
            "this test binary resolves the session database to {} — the REAL user \
             database. Whatever runs first here will write test fixtures into it. \
             Every crate root that compiles these test modules needs the \
             pin_config_to_temp_root_for_tests() ctor that lib.rs arms.",
            db.display()
        );
    }
}
