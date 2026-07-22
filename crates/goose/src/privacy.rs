//! Shared privacy redaction — the single source of truth for scrubbing
//! known-sensitive substrings out of any text before it is exported to the user
//! or (ever) transmitted off this machine.
//!
//! Reused by:
//! - the telemetry sanitizer (`crate::posthog::sanitize_string`), and
//! - the local, user-triggered redacted crash-report export
//!   (`crate::session::crash_capture::export_redacted_bundle`, #327).
//!
//! This module is deliberately **not** feature-gated: the crash-report export
//! must redact even in builds without the `telemetry` feature. Redaction is
//! mitigation, not physics — a regex allowlist reduces leakage of known shapes
//! (home paths, key/token prefixes, bearer creds, emails, creds-in-URL, UUIDs)
//! but cannot prove a backtrace is free of user content. `replace_all` runs over
//! the whole string, so multi-line backtraces are covered.

use regex::Regex;
use std::sync::LazyLock;

/// Ordered list of sensitive substring patterns replaced with `[REDACTED]`.
static SENSITIVE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // Home directories (leak the OS username).
        Regex::new(r"/Users/[^/\s]+").unwrap(),
        Regex::new(r"/home/[^/\s]+").unwrap(),
        Regex::new(r"(?i)C:\\Users\\[^\\\s]+").unwrap(),
        // API keys / tokens.
        Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap(),
        Regex::new(r"pk-[a-zA-Z0-9]{20,}").unwrap(),
        Regex::new(r"(?i)key[_-]?[a-zA-Z0-9]{16,}").unwrap(),
        Regex::new(r"(?i)token[_-]?[a-zA-Z0-9]{16,}").unwrap(),
        Regex::new(r"(?i)bearer\s+[a-zA-Z0-9._-]+").unwrap(),
        // Emails.
        Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap(),
        // Credentials embedded in a URL (`https://user:pass@host`).
        Regex::new(r"https?://[^:]+:[^@]+@").unwrap(),
        // UUIDs (installation ids, session ids, …).
        Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
            .unwrap(),
    ]
});

/// Redact known-sensitive substrings from `s`, replacing each with `[REDACTED]`.
/// Idempotent and multi-line safe. This is the one place the patterns live.
pub fn redact(s: &str) -> String {
    let mut result = s.to_string();
    for pattern in SENSITIVE_PATTERNS.iter() {
        result = pattern.replace_all(&result, "[REDACTED]").to_string();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_home_paths_keys_and_tokens_in_a_multiline_backtrace() {
        let backtrace = "thread 'main' panicked at src/foo.rs:1:1\n\
             0: at /Users/jesse/dev/permagent/crate.rs\n\
             1: token sk-abcdefghijklmnopqrstuvwxyz012345\n\
             2: contact jesse@example.com";
        let out = redact(backtrace);
        assert!(out.contains("[REDACTED]"), "must redact");
        assert!(!out.contains("/Users/jesse"), "home path must not leak");
        assert!(!out.contains("jesse@example.com"), "email must not leak");
        assert!(
            !out.contains("sk-abcdefghijklmnopqrstuvwxyz012345"),
            "api key must not leak"
        );
        // Non-sensitive structure survives so the report is still useful.
        assert!(out.contains("src/foo.rs:1:1"));
        assert!(out.contains("panicked"));
    }

    #[test]
    fn redacts_linux_home_windows_home_bearer_and_uuid() {
        let s = "/home/alice /C:\\Users\\Bob bearer abc.def-123 \
             550e8400-e29b-41d4-a716-446655440000";
        let out = redact(s);
        assert!(!out.contains("/home/alice"));
        assert!(!out.to_lowercase().contains("users\\bob"));
        assert!(!out.contains("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!out.to_lowercase().contains("bearer abc.def-123"));
    }

    #[test]
    fn leaves_clean_text_untouched() {
        let s = "a normal panic message with no secrets";
        assert_eq!(redact(s), s);
    }
}
