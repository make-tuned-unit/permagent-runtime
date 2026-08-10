# Completion-check PATH builder location

- **File defining `check_path`:** `crates/goose-server/src/verification/checks.rs`
- **Async function that invokes it when running a command check:** `run_command_check` (sets `.env("PATH", check_path())` on the spawned shell command)
- **Why `check_path` exists:** The daemon is launched by launchd, whose PATH omits developer toolchains (so e.g. `cargo check` exited 127 and condemned finished goal work), and `check_path` prepends the standard tool homes (`~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin`) to the inherited PATH so completion checks can find those tools.
