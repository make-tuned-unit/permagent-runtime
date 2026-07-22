#[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
compile_error!("At least one of `rustls-tls` or `native-tls` features must be enabled");

#[cfg(all(feature = "rustls-tls", feature = "native-tls"))]
compile_error!("Features `rustls-tls` and `native-tls` are mutually exclusive");

pub mod agent_state_tick;
pub mod analytics;
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
