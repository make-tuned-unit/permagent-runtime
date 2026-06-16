//! Evidence harness for #299 (local crash capture).
//!
//! Proves the headline behavior end-to-end: the *installed global panic hook*
//! fires on a real panic and lands a crash report on disk (message + location +
//! backtrace). Runs in its own test binary so the `Once`-guarded hook install
//! is fresh. Consent gating + prune/collect are covered by the unit tests in
//! `crash_capture`; sharing has no network path (upload deferred to #327).

use permagent::session::crash_capture;

#[test]
fn real_panic_fires_hook_and_writes_crash_log() {
    let dir = tempfile::tempdir().unwrap();
    crash_capture::install_panic_hook_to(dir.path().to_path_buf());

    // A genuine panic — the hook runs before unwinding; catch_unwind keeps the
    // test process alive so we can inspect what the hook wrote.
    let outcome = std::panic::catch_unwind(|| panic!("evidence boom 42"));
    assert!(outcome.is_err(), "the panic should have unwound");

    let logs = crash_capture::collect_crash_logs(dir.path());
    assert_eq!(
        logs.len(),
        1,
        "the installed hook should have written exactly one crash report"
    );
    let text = String::from_utf8(logs[0].1.clone()).unwrap();
    assert!(
        text.contains("evidence boom 42"),
        "report carries the panic message"
    );
    assert!(
        text.contains("Location:"),
        "report carries the panic location"
    );
    assert!(text.contains("Backtrace:"), "report carries a backtrace");
}
