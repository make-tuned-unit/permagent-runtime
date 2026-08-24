//! Venv + pip for user checkouts we start (Picker, Polybot).
//!
//! Homebrew Python on this machine returns an empty `platform.mac_ver()`,
//! which breaks `ensurepip` and `uv`'s interpreter probe. Prefer `uv` and
//! let it pick a managed CPython. Fall back to `python3 -m venv` only when
//! `uv` is not on PATH.

use std::path::{Path, PathBuf};

/// Create `venv` if it does not already have a `bin/python`.
pub async fn ensure_venv(venv: &Path) -> Result<PathBuf, String> {
    let python = venv.join("bin/python");
    if python.is_file() {
        return Ok(python);
    }
    if which("uv").is_some() {
        run("uv", &["venv", &venv.to_string_lossy()]).await?;
        if python.is_file() {
            return Ok(python);
        }
    }
    let interpreter = which("python3.12")
        .or_else(|| which("python3"))
        .ok_or("python3 is not on PATH")?;
    run(
        &interpreter.to_string_lossy(),
        &["-m", "venv", &venv.to_string_lossy()],
    )
    .await?;
    if python.is_file() {
        Ok(python)
    } else {
        Err(format!(
            "created {} but bin/python is missing",
            venv.display()
        ))
    }
}

/// `pip install` into an existing venv. `extra` is appended after
/// `pip install` (package names, or `-r requirements.txt`).
pub async fn pip_install(venv: &Path, extra: &[&str]) -> Result<(), String> {
    let python = venv.join("bin/python");
    if !python.is_file() {
        return Err(format!("no python in {}", venv.display()));
    }
    if which("uv").is_some() {
        let mut args = vec![
            "pip".into(),
            "install".into(),
            "--python".into(),
            python.display().to_string(),
        ];
        args.extend(extra.iter().map(|s| (*s).to_string()));
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        return run("uv", &refs).await.map(|_| ());
    }
    let mut args = vec!["-m".into(), "pip".into(), "install".into()];
    args.extend(extra.iter().map(|s| (*s).to_string()));
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(&python.to_string_lossy(), &refs).await.map(|_| ())
}

fn which(bin: &str) -> Option<PathBuf> {
    let Ok(out) = std::process::Command::new("which").arg(bin).output() else {
        return None;
    };
    if !out.status.success() {
        return None;
    }
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    p.is_file().then_some(p)
}

async fn run(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new(bin)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("{bin} could not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{bin} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
