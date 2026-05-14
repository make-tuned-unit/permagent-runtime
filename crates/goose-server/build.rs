use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/");

    // Git SHA
    let sha = git(&["rev-parse", "--short=9", "HEAD"]);
    println!("cargo:rustc-env=PERMAGENT_GIT_SHA={}", sha);

    // Git branch
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    println!("cargo:rustc-env=PERMAGENT_GIT_BRANCH={}", branch);

    // Build timestamp (UTC, ISO 8601)
    let ts = chrono_now_utc();
    println!("cargo:rustc-env=PERMAGENT_BUILD_TIMESTAMP={}", ts);

    // Rust version
    let rustc = rustc_version();
    println!("cargo:rustc-env=PERMAGENT_RUST_VERSION={}", rustc);

    // Spectral pin — extract rev from Cargo.lock
    let spectral_pin = spectral_rev();
    println!("cargo:rustc-env=PERMAGENT_SPECTRAL_PIN={}", spectral_pin);
}

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn chrono_now_utc() -> String {
    // Use `date` command to avoid adding a build dependency
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| {
            // "rustc 1.85.0 (..." -> "1.85.0"
            s.trim()
                .strip_prefix("rustc ")
                .unwrap_or(s.trim())
                .split_whitespace()
                .next()
                .unwrap_or("unknown")
                .to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn spectral_rev() -> String {
    // Parse Cargo.lock for spectral's git rev
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../Cargo.lock");
    if let Ok(contents) = std::fs::read_to_string(lock_path) {
        // Look for: name = "spectral"\nversion = ...\nsource = "git+...?rev=XXXX#full_hash"
        let mut in_spectral = false;
        for line in contents.lines() {
            if line.trim() == "name = \"spectral\"" {
                in_spectral = true;
                continue;
            }
            if in_spectral && line.starts_with("source = ") {
                // Extract short rev from the ?rev=XXX part
                if let Some(rev_start) = line.find("?rev=") {
                    let after = &line[rev_start + 5..];
                    let rev = after.split('#').next().unwrap_or(after);
                    let rev = rev.trim_end_matches('"');
                    return rev.to_string();
                }
                break;
            }
            if in_spectral && line.starts_with("[[") {
                break; // next package block
            }
        }
    }
    "unknown".to_string()
}
