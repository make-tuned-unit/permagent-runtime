//! **Dev-binary signing guard.** Pins the one property that stops macOS
//! asking the user for their login password after every rebuild.
//!
//! The keychain this crate talks to (`KEYRING_SERVICE` in [`super::base`]) is
//! guarded per-application by an ACL, and macOS decides whether a caller is
//! "the same application" by matching its **designated requirement** — the
//! `codesign -d -r-` string. On Apple silicon, cargo's linker signs every
//! binary ad hoc, and an ad-hoc designated requirement is
//!
//! ```text
//! designated => cdhash H"722bcfac4c429c73c6f594d5feb8eb3d7ee0c1de"
//! ```
//!
//! a hash of the binary's own bytes (measured on this repo's release CLI,
//! 2026-08-19). Recompile anything and it is a different string, so the ACL
//! no longer recognises the caller and the grant the user clicked is dead.
//!
//! `scripts/sign-dev-binaries.sh` re-signs the built binaries with a real
//! Developer ID under a *pinned* `--identifier`, which yields
//!
//! ```text
//! designated => identifier permagent and anchor apple generic and … and
//!               certificate leaf[subject.OU] = <team id>
//! ```
//!
//! — no content hash anywhere, therefore stable across rebuilds. The pin is
//! the whole mechanism: drop `--identifier` and codesign picks something
//! derived from the file, and the drift comes straight back. So the tests
//! below check *that*, not merely that signing succeeded.
//!
//! Two tests, deliberately different in reach:
//!
//! - [`signing_script_pins_identifiers_and_degrades_gracefully`] reads the
//!   script and the npm wiring as text. It runs everywhere, including on the
//!   Linux CI runner that has no certificate and no `codesign`.
//! - [`developer_id_signing_makes_the_designated_requirement_content_free`]
//!   is the real proof, and it can only run on a macOS machine that actually
//!   holds a Developer ID certificate. It builds two *different* tiny Mach-O
//!   binaries, records that their ad-hoc requirements differ, signs both
//!   through the script, and asserts the resulting requirements are
//!   byte-identical. Anywhere else it skips out loud rather than passing
//!   vacuously.

// string_slice: the one slice below starts at a `find()` result (always a
// char boundary) and runs to the end of the string, so it cannot split a
// UTF-8 sequence. Same argument as `identity_name_guard`.
#![allow(clippy::string_slice)]

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/goose; the workspace root is two up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root above crates/goose")
        .to_path_buf()
}

/// Binaries that read the keychain and therefore need a stable requirement.
/// `permagent-app` is not among them: the Tauri shell depends on neither
/// `keyring` nor this crate, and never issues a keychain call.
const KEYCHAIN_READING_BINARIES: &[&str] = &["permagent", "permagentd"];

#[test]
fn signing_script_pins_identifiers_and_degrades_gracefully() {
    let script_path = repo_root().join("scripts/sign-dev-binaries.sh");
    let script = std::fs::read_to_string(&script_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", script_path.display()));

    // 1. The pin itself. Without an explicit --identifier the requirement goes
    //    back to being derived from the file, and the grant breaks again.
    assert!(
        script.contains("--identifier \"$ident\""),
        "sign-dev-binaries.sh must pass an explicit --identifier to codesign"
    );

    // 2. Every keychain-reading binary is covered, with a fixed identifier.
    for name in KEYCHAIN_READING_BINARIES {
        assert!(
            script
                .lines()
                .any(|l| l.starts_with("KNOWN_NAMES=(") && l.contains(name)),
            "sign-dev-binaries.sh does not list `{name}`, which reads the keychain"
        );
        assert!(
            script.contains(&format!("printf '{name}'")),
            "sign-dev-binaries.sh has no pinned identifier for `{name}`"
        );
    }

    // 3. A contributor or CI runner with no certificate must get a no-op, not
    //    a failed build. The absent-identity branch has to end in `exit 0`.
    let marker = "no Developer ID Application certificate in the keychain";
    let idx = script
        .find(marker)
        .expect("sign-dev-binaries.sh must report the missing-certificate case");
    let tail = &script[idx..];
    let exit_zero = tail
        .find("exit 0")
        .expect("missing-certificate branch must exit");
    let exit_nonzero = tail.find("exit 1").unwrap_or(usize::MAX);
    assert!(
        exit_zero < exit_nonzero,
        "the missing-certificate branch must exit 0 (clean no-op), never fail the build"
    );
    assert!(
        script.contains("!= \"Darwin\""),
        "sign-dev-binaries.sh must no-op off macOS rather than erroring"
    );

    // 4. Public repo: the certificate is discovered at runtime, never written
    //    down. A full common name would look like
    //    `Developer ID Application: Some Name (TEAMIDXXXX)`, so neither a name
    //    after the colon nor a ten-character team id in parentheses may appear.
    for (lineno, line) in script.lines().enumerate() {
        if let Some(rest) = line.split("Developer ID Application:").nth(1) {
            let next = rest.chars().next().unwrap_or('"');
            assert!(
                !next.is_ascii_alphabetic(),
                "line {}: a certificate common name is a personal identity string and must not be \
                 committed; discover it at runtime or read PERMAGENT_SIGN_IDENTITY",
                lineno + 1
            );
        }
        assert!(
            !looks_like_team_id_literal(line),
            "line {}: an Apple Team ID must not be hardcoded in this public repo",
            lineno + 1
        );
    }

    // 5. It has to actually run on a normal rebuild, so the npm scripts a
    //    developer really uses must invoke it.
    let pkg_path = repo_root().join("ui/desktop/package.json");
    let pkg = std::fs::read_to_string(&pkg_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", pkg_path.display()));
    for (script_name, bin) in [("build:cli", "permagent"), ("build:daemon", "permagentd")] {
        let line = pkg
            .lines()
            .find(|l| l.contains(&format!("\"{script_name}\":")))
            .unwrap_or_else(|| panic!("ui/desktop/package.json has no `{script_name}` script"));
        assert!(
            line.contains(&format!("sign-dev-binaries.sh {bin}")),
            "`{script_name}` must sign `{bin}` after building it, or a rebuild silently \
             reverts to an ad-hoc signature: {line}"
        );
    }
}

/// True when `line` contains `(XXXXXXXXXX)` — ten upper-case alphanumerics in
/// parentheses, the shape of an Apple Team ID. Kept narrow on purpose: prose
/// in parentheses is lower-case, so this cannot fire on a comment.
fn looks_like_team_id_literal(line: &str) -> bool {
    let b = line.as_bytes();
    b.windows(12).any(|w| {
        w[0] == b'(' && w[11] == b')' && w[1..11].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            // a run of ten digits in parentheses is a number, not a team id
            && w[1..11].iter().any(|c| c.is_ascii_uppercase())
    })
}

/// The property the keychain grant depends on: two builds of *different*
/// source must present the *same* designated requirement once signed.
///
/// Skips (loudly) unless this is macOS with a Developer ID certificate and a
/// working C compiler, because there is no honest way to fake a real
/// certificate chain.
#[test]
fn developer_id_signing_makes_the_designated_requirement_content_free() {
    if !cfg!(target_os = "macos") {
        eprintln!("SKIP: designated-requirement proof needs macOS codesign");
        return;
    }
    let have_identity = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("\"Developer ID Application:"))
        .unwrap_or(false);
    if !have_identity {
        eprintln!(
            "SKIP: no Developer ID Application certificate in this keychain — the \
             designated-requirement proof cannot run here. See \
             docs/operations/DEV_SIGNING.md for the manual procedure."
        );
        return;
    }
    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("SKIP: no C compiler to build signing fixtures with");
        return;
    }

    let dir = std::env::temp_dir().join(format!("permagent-signing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let script = repo_root().join("scripts/sign-dev-binaries.sh");
    // One identifier, two genuinely different programs — the same relationship
    // two consecutive builds of the CLI stand in.
    const IDENTIFIER: &str = "permagent-signing-fixture";
    let mut adhoc = Vec::new();
    let mut signed = Vec::new();

    for (n, body) in [(0u8, "return 7;"), (1u8, "return 8;")] {
        let src = dir.join(format!("f{n}.c"));
        let bin = dir.join(format!("f{n}"));
        std::fs::write(&src, format!("int main(void){{{body}}}\n")).expect("write fixture");
        let cc = Command::new("cc")
            .arg("-o")
            .arg(&bin)
            .arg(&src)
            .output()
            .expect("run cc");
        assert!(
            cc.status.success(),
            "cc failed: {}",
            String::from_utf8_lossy(&cc.stderr)
        );

        adhoc.push(designated_requirement(&bin));

        let out = Command::new("bash")
            .arg(&script)
            .arg("--file")
            .arg(&bin)
            .arg("--identifier")
            .arg(IDENTIFIER)
            .output()
            .expect("run sign-dev-binaries.sh");
        assert!(
            out.status.success(),
            "sign-dev-binaries.sh failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        signed.push(designated_requirement(&bin));
    }

    let _ = std::fs::remove_dir_all(&dir);

    // Baseline: ad-hoc signing is exactly the problem. Two different binaries,
    // two different requirements — so any grant is invalidated by a rebuild.
    assert_ne!(
        adhoc[0], adhoc[1],
        "ad-hoc requirements were expected to differ with content; if they no longer do, \
         the premise of this fix has changed and it should be re-derived"
    );
    for r in &adhoc {
        assert!(
            r.contains("cdhash"),
            "expected a content hash in an ad-hoc requirement: {r}"
        );
    }

    // The fix: identical requirements despite different contents.
    assert_eq!(
        signed[0], signed[1],
        "signed designated requirements must be byte-identical across builds — this is the \
         property the keychain \"Always Allow\" is matched on"
    );
    assert!(
        signed[0].contains(&format!("identifier \"{IDENTIFIER}\""))
            || signed[0].contains(&format!("identifier {IDENTIFIER}")),
        "signed requirement must name the pinned identifier: {}",
        signed[0]
    );
    assert!(
        !signed[0].contains("cdhash"),
        "a signed requirement that still mentions cdhash is not content-free: {}",
        signed[0]
    );
}

fn designated_requirement(path: &std::path::Path) -> String {
    let out = Command::new("codesign")
        .arg("-d")
        .arg("-r-")
        .arg(path)
        .output()
        .expect("run codesign");
    // codesign writes the requirement to stderr on some releases and stdout on
    // others; take whichever carries the line.
    let both = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    both.lines()
        .find(|l| l.trim_start_matches("# ").starts_with("designated =>"))
        .unwrap_or_else(|| panic!("no designated requirement for {}: {both}", path.display()))
        .trim_start_matches("# ")
        .trim()
        .to_string()
}
