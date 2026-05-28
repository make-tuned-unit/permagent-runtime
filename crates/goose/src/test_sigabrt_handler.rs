//! SIGABRT handler for test diagnostics (issue #190).
//!
//! Installs a signal handler that captures an in-process backtrace when
//! SIGABRT fires during test teardown on Linux. The backtrace is printed to
//! stderr with resolved symbols, sidestepping the unreadable core dump problem.
//!
//! This module is `#[cfg(test)]` only — it does not exist in production builds.
//! The handler is Linux-only; on macOS `install()` is a no-op.

use std::sync::Once;

static INIT: Once = Once::new();

/// Install the SIGABRT handler. No-op on non-Linux platforms.
pub fn install() {
    #[cfg(target_os = "linux")]
    INIT.call_once(|| {
        unsafe {
            libc::signal(libc::SIGABRT, sigabrt_handler as libc::sighandler_t);
        }
        eprintln!("[test_sigabrt_handler] SIGABRT handler installed for #190 diagnostics");
    });

    #[cfg(not(target_os = "linux"))]
    let _ = &INIT; // suppress unused warning
}

#[cfg(target_os = "linux")]
extern "C" fn sigabrt_handler(_sig: libc::c_int) {
    // NOTE: backtrace::Backtrace::new() allocates and is not async-signal-safe.
    // For process-exit aborts this usually works. If it deadlocks, CI times out.

    let bt = backtrace::Backtrace::new();

    let header = b"\n=== SIGABRT caught (#190 diagnostic) - in-process backtrace ===\n";
    unsafe {
        libc::write(2, header.as_ptr() as *const libc::c_void, header.len());
    }

    let formatted = format!("{:?}\n", bt);
    let bytes = formatted.as_bytes();
    unsafe {
        libc::write(2, bytes.as_ptr() as *const libc::c_void, bytes.len());
    }

    let footer = b"=== end SIGABRT backtrace ===\n";
    unsafe {
        libc::write(2, footer.as_ptr() as *const libc::c_void, footer.len());
    }

    // Re-raise with default handler
    unsafe {
        libc::signal(libc::SIGABRT, libc::SIG_DFL);
        libc::raise(libc::SIGABRT);
    }
}
