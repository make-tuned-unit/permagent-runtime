//! `manage_extensions` must persist, not just take effect for one session.
//!
//! Its own integration binary, and therefore its own PROCESS: these tests pin
//! `PERMAGENT_PATH_ROOT` and initialise the process-global `Config`, which is a
//! `OnceLock` — sharing a binary with tests that read the global config would
//! make the result depend on which test ran first. `liveness_wire.rs` documents
//! the same constraint for the same reason.

use permagent::events::{self, PermagentEvent, PermagentEventType};

fn drain(rx: &mut tokio::sync::broadcast::Receiver<PermagentEvent>) -> Vec<PermagentEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

/// Does any `config_changed` frame name `key`?
fn names_key(frames: &[PermagentEvent], key: &str) -> bool {
    frames
        .iter()
        .filter(|e| e.event_type == PermagentEventType::ConfigChanged)
        .any(|e| {
            e.payload
                .get("keys")
                .and_then(|v| v.as_array())
                .is_some_and(|a| a.iter().any(|k| k.as_str() == Some(key)))
        })
}

/// FAILS BEFORE: `manage_extensions` mutated only the live in-session
/// `ExtensionManager` and never touched config.yaml, so the change did not
/// survive a restart while Settings — which reads config.yaml — showed the old
/// state the whole time. `persist_and_report` did not exist.
///
/// The "simulated restart" is real: `config::is_extension_enabled` re-reads
/// config.yaml on every call, so reading it back after the write is exactly
/// what a freshly started daemon sees.
///
/// ONE `#[test]` for both cases on purpose. `pin_config_to_temp_root_for_tests`
/// wipes and re-pins a per-PROCESS root; two tests calling it in parallel would
/// each delete the other's fixture, which is the same reason `liveness_wire.rs`
/// keeps one test per integration binary.
///
/// Scope note: this covers the DURABLE half. The live `ExtensionManager`
/// mutation is not exercised — constructing one needs a provider and a session
/// manager — so "`manage_extensions_impl` calls this on both branches" is
/// verified by reading it, not by this test.
#[test]
fn manage_extensions_persists_and_says_so_honestly() {
    permagent::config::base::pin_config_to_temp_root_for_tests();

    let name = "R1 Restart Probe";
    let key = permagent::config::name_to_key(name);
    permagent::config::set_extension(permagent::config::ExtensionEntry {
        enabled: true,
        config: permagent::agents::ExtensionConfig::Builtin {
            name: name.to_string(),
            display_name: Some(name.to_string()),
            description: "R1 test".to_string(),
            timeout: None,
            bundled: None,
            available_tools: vec![],
        },
    });
    assert!(
        permagent::config::is_extension_enabled(&key),
        "fixture did not land"
    );

    let mut rx = events::subscribe();
    let out = permagent::agents::platform_extensions::ext_manager::persist_and_report_for_tests(
        false, name, &key,
    );

    // Simulated restart: re-read config.yaml from disk.
    assert!(
        !permagent::config::is_extension_enabled(&key),
        "the disable did not reach config.yaml — it would be back after a restart"
    );

    // And an open Settings pane is told, via the frame every config write now
    // produces.
    assert!(
        names_key(&drain(&mut rx), "extensions"),
        "persisting the extension flag did not announce config_changed"
    );

    let text = format!("{out:?}");
    assert!(
        text.contains("saved to your configuration"),
        "the tool must tell the model the change is durable; got: {text}"
    );

    // Re-enabling round-trips too — the flag is written, not just cleared.
    let out = permagent::agents::platform_extensions::ext_manager::persist_and_report_for_tests(
        true, name, &key,
    );
    assert!(
        permagent::config::is_extension_enabled(&key),
        "the enable did not reach config.yaml"
    );
    assert!(format!("{out:?}").contains("saved to your configuration"));

    // An extension with no config.yaml entry can be toggled live and CANNOT be
    // persisted. Reporting that as a plain success is what teaches the model to
    // promise durability it never got.
    let out = permagent::agents::platform_extensions::ext_manager::persist_and_report_for_tests(
        false,
        "R1 Not In Config",
        "r1_not_in_config",
    );
    let text = format!("{out:?}");
    assert!(
        text.contains("THIS SESSION ONLY"),
        "a change that could not be saved was reported as if it had been: {text}"
    );
}
