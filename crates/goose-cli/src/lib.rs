#[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
compile_error!("At least one of `rustls-tls` or `native-tls` features must be enabled");

#[cfg(all(feature = "rustls-tls", feature = "native-tls"))]
compile_error!("Features `rustls-tls` and `native-tls` are mutually exclusive");

/// Pin `permagent`'s config and data root to a temp dir for this crate's test
/// binary. The library pins itself only while compiling ITS OWN tests; here it
/// is an ordinary dependency, so without this CLI tests wrote the developer's
/// real ~/.permagent/config.yaml (seen 2026-09-24: `learn_next_last_offered`
/// written by a `permagent-cli --lib` run). Same fix as permagent-daemon's lib.
#[cfg(test)]
#[ctor::ctor(unsafe)]
fn pin_config_for_cli_tests() {
    permagent::config::base::pin_config_to_temp_root_for_tests();
}

pub mod cli;
pub mod commands;
pub mod logging;
pub mod project_tracker;
pub mod recipes;
pub mod scenario_tests;
pub mod session;
pub mod signal;

// Re-export commonly used types
pub use cli::Cli;
pub use session::CliSession;
