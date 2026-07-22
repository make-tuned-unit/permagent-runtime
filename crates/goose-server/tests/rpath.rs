//! Regression test for issue #295: the daemon binaries must carry an
//! `@executable_path` rpath on macOS so they can locate the sherpa-onnx /
//! onnxruntime dylibs that a plain `cargo build` drops beside them in
//! `target/<profile>/`. Without it, in-place debug builds fail to load the
//! STT dylibs at runtime unless DYLD_FALLBACK_LIBRARY_PATH is set by hand.
//!
//! The rpath is emitted by `build.rs` via `cargo:rustc-link-arg-bins`.

/// On macOS, assert the built `permagentd` binary has an `@executable_path`
/// LC_RPATH load command. On other platforms this is a no-op (the rpath is
/// macOS-specific and only relevant to the macOS-only sherpa dylib layout).
#[test]
fn permagentd_has_executable_path_rpath() {
    if !cfg!(target_os = "macos") {
        return;
    }

    // Cargo exposes the path to the built binary to integration tests.
    let bin = env!("CARGO_BIN_EXE_permagentd");

    let output = std::process::Command::new("otool")
        .args(["-l", bin])
        .output()
        .expect("failed to run `otool -l` on the permagentd binary");
    assert!(output.status.success(), "otool -l exited non-zero on {bin}");

    let text = String::from_utf8_lossy(&output.stdout);

    // otool prints LC_RPATH load commands as a `cmd LC_RPATH` block whose
    // `path` line names the rpath, e.g. `path @executable_path (offset 12)`.
    let has_rpath = text
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("path @executable_path "));

    assert!(
        has_rpath,
        "permagentd is missing the `@executable_path` rpath (issue #295); \
         otool -l output:\n{text}"
    );
}
