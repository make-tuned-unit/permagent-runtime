//! Integration coverage for the runaway-loop `ProgressMonitor` public API
//! (non-negotiable B). The exhaustive per-signal/threshold tests live inline in
//! `tool_monitor.rs`; this file proves the pure detection surface is usable from
//! outside the crate and re-checks the load-bearing invariants: the exact
//! escalation threshold and the critical false-positive guard.

use permagent::tool_monitor::{assess_tool_call, is_wait_poll, LoopAction, Signal, ToolEvent};
use serde_json::{json, Value};

fn ev(name: &str, args: Value, result_hash: u64, is_error: bool) -> ToolEvent {
    ToolEvent {
        name: name.to_string(),
        args,
        result_hash,
        is_error,
        is_mutating: permagent::tool_monitor::is_mutating(name),
    }
}

/// S1: a 3rd identical call+result escalates; the 2nd only nudges (fires at 3,
/// not 2).
#[test]
fn repeated_identical_call_escalates_at_three_not_two() {
    let args = json!({"id": 123});

    let after_one = vec![ev("fetch_user", args.clone(), 7, false)];
    assert_eq!(
        assess_tool_call(&after_one, "fetch_user", &args),
        LoopAction::Nudge(Signal::RepeatedCall),
        "the 2nd identical call must nudge, not escalate"
    );

    let after_two = vec![
        ev("fetch_user", args.clone(), 7, false),
        ev("fetch_user", args.clone(), 7, false),
    ];
    assert_eq!(
        assess_tool_call(&after_two, "fetch_user", &args),
        LoopAction::Escalate(Signal::RepeatedCall),
        "the 3rd identical call must escalate"
    );
}

/// The critical false-positive guard: varying args → progress → NO trigger.
#[test]
fn varying_args_pass() {
    let win = vec![
        ev("fetch_user", json!({"id": 1}), 1, false),
        ev("fetch_user", json!({"id": 2}), 2, false),
        ev("fetch_user", json!({"id": 3}), 3, false),
    ];
    assert_eq!(
        assess_tool_call(&win, "fetch_user", &json!({"id": 4})),
        LoopAction::Pass,
        "changing arguments is progress and must pass"
    );
}

/// The critical false-positive guard: a changing result_hash → progress → never
/// escalates (a legit retry that is getting somewhere is preserved).
#[test]
fn changing_result_does_not_escalate() {
    let args = json!({"cmd": "poll"});
    let win = vec![
        ev("check", args.clone(), 1, false),
        ev("check", args.clone(), 2, false),
        ev("check", args.clone(), 3, false),
    ];
    assert_ne!(
        assess_tool_call(&win, "check", &args),
        LoopAction::Escalate(Signal::RepeatedCall),
        "a changing result must never escalate"
    );
}

/// S2: three identical *failures* escalate as `SameFailure` (Stuck).
#[test]
fn same_failure_escalates_as_stuck() {
    let args = json!({"cmd": "cargo build"});
    let win = vec![
        ev("developer__shell", args.clone(), 9, true),
        ev("developer__shell", args.clone(), 9, true),
    ];
    assert_eq!(
        assess_tool_call(&win, "developer__shell", &args),
        LoopAction::Escalate(Signal::SameFailure)
    );
}

/// wait/poll/watch tools are exempt from the S1–S3 stall signals.
#[test]
fn wait_poll_tools_are_exempt() {
    assert!(is_wait_poll("developer__wait_for_process"));
    let args = json!({"pid": 1});
    let win = vec![
        ev("developer__wait_for_process", args.clone(), 5, false),
        ev("developer__wait_for_process", args.clone(), 5, false),
        ev("developer__wait_for_process", args.clone(), 5, false),
    ];
    assert_eq!(
        assess_tool_call(&win, "developer__wait_for_process", &args),
        LoopAction::Pass,
        "a legit long-poll must never be blocked"
    );
}
