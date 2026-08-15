#[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
compile_error!("At least one of `rustls-tls` or `native-tls` features must be enabled");

#[cfg(all(feature = "rustls-tls", feature = "native-tls"))]
compile_error!("Features `rustls-tls` and `native-tls` are mutually exclusive");

/// Pin this test binary's config to a temp root before any test body runs.
///
/// `permagent` arms the same pin for its own tests, but `cfg(test)` is set only
/// while compiling that crate's test binary — this one links `permagent` as an
/// ordinary dependency, so its initialiser never fired here and daemon tests
/// read the developer's real config and system keyring.
///
/// That was not theoretical: `agents_surface::tests::
/// secret_response_never_serializes_value` hung indefinitely in
/// `Config::get_secret -> all_secrets`, blocked on a macOS keychain
/// authorisation prompt that a headless run cannot answer.
#[cfg(test)]
#[ctor::ctor(unsafe)]
fn pin_config_for_daemon_tests() {
    permagent::config::base::pin_config_to_temp_root_for_tests();
}

pub mod agent_state_tick;
pub mod analytics;
pub mod analytics_drain;
pub mod app_catalog;
pub mod auth;
pub mod automation;
pub mod backup;
pub mod brain_ops;
pub mod configuration;
pub mod device_registry;
pub mod error;
pub mod event_at_backfill;
pub mod federation;
pub mod growth_sweep;
pub mod middleware;
pub mod notification_router;
pub mod openapi;
pub mod routes;
pub mod session_event_bus;
pub mod state;
#[cfg(test)]
mod test_support;
#[cfg(any(feature = "rustls-tls", feature = "native-tls"))]
pub mod tls;
pub mod tunnel;
pub mod verification;
pub mod voice;
pub mod wal_checkpoint;

// Re-export commonly used items
pub use openapi::*;
pub use state::*;
