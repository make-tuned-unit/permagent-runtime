//! Where a given secret actually comes from.
//!
//! Until now secrets were keychain-or-nothing. That is fine right up until the
//! keychain says no — and on macOS it says no for reasons that have nothing to
//! do with the user: an ad-hoc-signed rebuild gets a new `cdhash`, which is not
//! on the keychain item's partition list, which produces `Unable to obtain
//! authorization`; a sleeping display produces `In dark wake, no UI possible`.
//! Both took several wrong diagnoses to find (see `base.rs`
//! `is_authorization_refusal`). A password-manager reference resolved through
//! the manager's own CLI never touches that ACL at all.
//!
//! **The keychain stays the default.** This module ADDS sources. A key with no
//! configured source behaves exactly as it did before, byte for byte.
//!
//! Two rules govern everything here, and both exist because this repo has been
//! burned by their absence:
//!
//! 1. **No silent fallback.** If a key is configured to come from 1Password and
//!    1Password cannot answer, the key is UNAVAILABLE and the error says so.
//!    Quietly reading the keychain instead would leave the user believing a
//!    reference works when it does not — the same shape as the incident where
//!    an unreadable keychain was reported as an empty one and 25 live keys read
//!    as "not configured".
//! 2. **No secret value in any log, error, or trace.** Enforced structurally:
//!    a CLI's stdout is wrapped in [`SecretValue`] the instant it is read, and
//!    that type has no `Display` and a redacting `Debug`. Its stderr — the only
//!    external text we ever surface — goes through [`sanitize_cli_stderr`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Config key holding the per-key source map, e.g.
/// `secret_sources: { OPENAI_API_KEY: "op://Personal/OpenAI/credential" }`.
pub const SECRET_SOURCES_KEY: &str = "secret_sources";

/// Config key holding the source used for keys absent from the map above.
/// Only the built-in stores are meaningful here — see [`SecretSource::parse_default`].
pub const SECRET_SOURCE_DEFAULT_KEY: &str = "secret_source_default";

/// How long a single secret READ may take before we give up on it.
///
/// Generous, because `op read` on the first use of a session can involve a
/// Touch ID prompt, and killing that at 2s would make the feature unusable.
/// Bounded, because the alternative is the failure mode this whole file is
/// written against: a credential read that never returns.
pub const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// How long an availability PROBE may take. Much shorter than a read: `op
/// whoami` / `bw status` are local and must never make a Settings page hang.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a successfully resolved value stays usable without re-running the
/// CLI.
///
/// Not an optimisation for its own sake. `check_provider_configured` calls
/// `get_secret` for every provider every time Settings lists them; without a
/// cache that is one subprocess — and potentially one biometric prompt — per
/// provider per page load. Only SUCCESSES are cached and only briefly, so a
/// manager that goes away mid-session starts failing honestly within
/// `CACHE_TTL` rather than being papered over indefinitely.
const CACHE_TTL: Duration = Duration::from_secs(30);

/// A secret, wrapped so it cannot be printed by accident.
///
/// The point is not politeness, it is that `{:?}` on a struct, a `tracing`
/// field, or an error context chain will otherwise happily serialise a live API
/// key into a log file. Making the redaction part of the TYPE means every
/// future call site inherits it without having to remember.
#[derive(Clone)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        SecretValue(value.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Deliberately verbose: unwrapping a secret should be visible in review.
    pub fn expose(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Length is withheld too — it narrows the search space for short values
        // and is never worth what it costs.
        f.write_str("SecretValue(<redacted>)")
    }
}

/// Which external manager a source talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    OnePassword,
    Bitwarden,
}

impl Backend {
    pub const ALL: [Backend; 2] = [Backend::OnePassword, Backend::Bitwarden];

    pub fn id(self) -> &'static str {
        match self {
            Backend::OnePassword => "onepassword",
            Backend::Bitwarden => "bitwarden",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Backend::OnePassword => "1Password",
            Backend::Bitwarden => "Bitwarden",
        }
    }

    pub fn binary(self) -> &'static str {
        match self {
            Backend::OnePassword => "op",
            Backend::Bitwarden => "bw",
        }
    }

    /// Escape hatch for installs where the CLI is not on any path we search,
    /// and the hook the unit tests use to stand in a stub binary rather than
    /// requiring a real 1Password on the machine running `cargo test`.
    fn binary_override_env(self) -> &'static str {
        match self {
            Backend::OnePassword => "PERMAGENT_OP_BIN",
            Backend::Bitwarden => "PERMAGENT_BW_BIN",
        }
    }
}

/// Everything that can go wrong resolving a secret, kept as distinct variants
/// because the remedies are completely different: install the CLI, sign in,
/// fix the reference, or look at why it hung.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretSourceError {
    #[error(
        "'{spec}' is not a valid secret source. Use \"keychain\", \"file\", an \
         \"op://Vault/Item/field\" reference, or \"bw://Item/field\". {detail}"
    )]
    Malformed { spec: String, detail: String },

    #[error(
        "{} is not installed (no '{}' found), so this key cannot be read.",
        backend.display_name(),
        backend.binary()
    )]
    NotInstalled { backend: Backend },

    #[error(
        "{} is installed but not signed in, so this key cannot be read. {detail}",
        backend.display_name()
    )]
    NotSignedIn { backend: Backend, detail: String },

    #[error("{} has no item matching '{locator}'.", backend.display_name())]
    NotFound { backend: Backend, locator: String },

    #[error(
        "{} did not answer within {}s. If an approval prompt is waiting on screen, \
         allow it and try again.",
        backend.display_name(),
        timeout.as_secs()
    )]
    Timeout { backend: Backend, timeout: Duration },

    #[error("{} returned an empty value for '{locator}'.", backend.display_name())]
    Empty { backend: Backend, locator: String },

    #[error("{} could not read '{locator}'. {detail}", backend.display_name())]
    Failed {
        backend: Backend,
        locator: String,
        detail: String,
    },
}

impl SecretSourceError {
    /// The backend involved, when there is one. `None` for a malformed spec,
    /// which fails before any CLI is chosen.
    pub fn backend(&self) -> Option<Backend> {
        match self {
            SecretSourceError::Malformed { .. } => None,
            SecretSourceError::NotInstalled { backend }
            | SecretSourceError::NotSignedIn { backend, .. }
            | SecretSourceError::NotFound { backend, .. }
            | SecretSourceError::Timeout { backend, .. }
            | SecretSourceError::Empty { backend, .. }
            | SecretSourceError::Failed { backend, .. } => Some(*backend),
        }
    }
}

/// Where one key's value comes from.
///
/// `Keychain` and `File` are the BUILT-IN stores and resolve inside `Config`;
/// the other two shell out to a password manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretSource {
    Keychain,
    OnePassword { reference: String },
    Bitwarden { item: String, field: String },
    File,
}

/// Field used when a `bw://` spec names an item but no field. Bitwarden's own
/// CLI shorthand (`bw get password <item>`) makes this the obvious default.
const BITWARDEN_DEFAULT_FIELD: &str = "password";

/// Fields `bw get <field> <item>` understands directly. Anything else is a
/// custom field and has to come out of the full item JSON.
const BITWARDEN_BUILTIN_FIELDS: [&str; 5] = ["password", "username", "totp", "notes", "uri"];

impl SecretSource {
    /// Parse a per-key spec string as written in `config.yaml`.
    ///
    /// Unknown text is an ERROR, never a shrug back to the keychain: a typo in
    /// `op://Personal/OpenAI/credentail` that silently resolved from the
    /// keychain would look exactly like a working reference, which is the
    /// failure this feature exists to remove.
    pub fn parse(spec: &str) -> Result<Self, SecretSourceError> {
        let trimmed = spec.trim();
        let malformed = |detail: &str| SecretSourceError::Malformed {
            spec: trimmed.to_string(),
            detail: detail.to_string(),
        };

        match trimmed.to_ascii_lowercase().as_str() {
            "" => return Err(malformed("The value is empty.")),
            "keychain" | "keyring" => return Ok(SecretSource::Keychain),
            "file" => return Ok(SecretSource::File),
            _ => {}
        }

        if let Some(rest) = trimmed.strip_prefix("op://") {
            // op://<vault>/<item>/[section/]<field> — at least three segments,
            // none empty. `op` itself rejects fewer, but only after a
            // subprocess round trip; catching it here keeps a typo out of the
            // CLI path entirely and lets Settings validate as the user types.
            let segments: Vec<&str> = rest.split('/').collect();
            if segments.len() < 3 || segments.iter().any(|s| s.is_empty()) {
                return Err(malformed(
                    "A 1Password reference needs at least vault, item and field: \
                     op://Vault/Item/field.",
                ));
            }
            return Ok(SecretSource::OnePassword {
                reference: trimmed.to_string(),
            });
        }

        if let Some(rest) = trimmed.strip_prefix("bw://") {
            let mut segments = rest.splitn(2, '/');
            let item = segments.next().unwrap_or_default().trim();
            let field = segments.next().unwrap_or(BITWARDEN_DEFAULT_FIELD).trim();
            if item.is_empty() {
                return Err(malformed("A Bitwarden reference needs an item: bw://Item."));
            }
            if field.is_empty() {
                return Err(malformed(
                    "The field after the item is empty. Drop the trailing '/' to use \
                     the password field.",
                ));
            }
            return Ok(SecretSource::Bitwarden {
                item: item.to_string(),
                field: field.to_string(),
            });
        }

        Err(malformed("Unrecognised prefix."))
    }

    /// Parse the process-wide DEFAULT source.
    ///
    /// Only the built-in stores are accepted. A manager reference is per-key by
    /// construction — one `op://` URL cannot be the default for every key, it
    /// would hand every provider the same credential — so accepting one here
    /// would be accepting a configuration that cannot work.
    pub fn parse_default(spec: &str) -> Result<Self, SecretSourceError> {
        match Self::parse(spec)? {
            source @ (SecretSource::Keychain | SecretSource::File) => Ok(source),
            _ => Err(SecretSourceError::Malformed {
                spec: spec.trim().to_string(),
                detail: format!(
                    "Only \"keychain\" or \"file\" can be the default. A password-manager \
                     reference names one specific item, so it only makes sense under a \
                     single key in {SECRET_SOURCES_KEY}."
                ),
            }),
        }
    }

    /// Round-trips through [`SecretSource::parse`]. This is what the UI writes back.
    pub fn to_spec(&self) -> String {
        match self {
            SecretSource::Keychain => "keychain".to_string(),
            SecretSource::File => "file".to_string(),
            SecretSource::OnePassword { reference } => reference.clone(),
            SecretSource::Bitwarden { item, field } => format!("bw://{item}/{field}"),
        }
    }

    /// Human label for Settings: "macOS Keychain", "1Password", …
    pub fn label(&self) -> String {
        match self {
            SecretSource::Keychain => {
                if cfg!(target_os = "macos") {
                    "macOS Keychain".to_string()
                } else {
                    "System keyring".to_string()
                }
            }
            SecretSource::File => "Local secrets file".to_string(),
            SecretSource::OnePassword { .. } => Backend::OnePassword.display_name().to_string(),
            SecretSource::Bitwarden { .. } => Backend::Bitwarden.display_name().to_string(),
        }
    }

    pub fn backend(&self) -> Option<Backend> {
        match self {
            SecretSource::Keychain | SecretSource::File => None,
            SecretSource::OnePassword { .. } => Some(Backend::OnePassword),
            SecretSource::Bitwarden { .. } => Some(Backend::Bitwarden),
        }
    }

    /// What this source points at, for messages. Never secret: a vault path is
    /// not a credential, and the user needs to see it to fix a typo.
    pub fn locator(&self) -> String {
        match self {
            SecretSource::Keychain | SecretSource::File => String::new(),
            SecretSource::OnePassword { reference } => reference.clone(),
            SecretSource::Bitwarden { item, field } => format!("{item}/{field}"),
        }
    }

    /// Read the value from an external manager.
    ///
    /// Returns `Ok(None)` for the built-in stores — they are `Config`'s job,
    /// and this module deliberately does not grow a second way to read the
    /// keychain.
    pub fn resolve(&self, timeout: Duration) -> Result<Option<SecretValue>, SecretSourceError> {
        let Some(backend) = self.backend() else {
            return Ok(None);
        };

        let cache_key = self.to_spec();
        if let Some(hit) = cache_get(&cache_key) {
            return Ok(Some(hit));
        }

        let value = match self {
            SecretSource::OnePassword { reference } => read_one_password(reference, timeout)?,
            SecretSource::Bitwarden { item, field } => read_bitwarden(item, field, timeout)?,
            SecretSource::Keychain | SecretSource::File => unreachable!("guarded by backend()"),
        };

        if value.is_empty() {
            return Err(SecretSourceError::Empty {
                backend,
                locator: self.locator(),
            });
        }

        cache_put(&cache_key, &value);
        Ok(Some(value))
    }
}

// ── The per-key source map ───────────────────────────────────────────────

/// Resolve the source for `key` from raw config values.
///
/// Split out from `Config` so it is testable without a keychain, a config file
/// or a process environment. Precedence is exactly the one in the proposal:
/// explicit per-key source → configured default → keychain.
pub fn source_for_key(
    key: &str,
    sources: &HashMap<String, String>,
    default_spec: Option<&str>,
) -> Result<SecretSource, SecretSourceError> {
    if let Some(spec) = lookup_case_insensitive(sources, key) {
        return SecretSource::parse(spec);
    }
    match default_spec {
        Some(spec) => SecretSource::parse_default(spec),
        None => Ok(SecretSource::Keychain),
    }
}

/// Keys reach `get_secret` in both cases (`openai_api_key` from `config_value!`,
/// `OPENAI_API_KEY` from providers), and a source that only matched one of them
/// would apply on some call paths and not others — a half-applied source is
/// worse than none.
fn lookup_case_insensitive<'a>(map: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    if let Some(v) = map.get(key) {
        return Some(v.as_str());
    }
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

// ── Availability ─────────────────────────────────────────────────────────

/// What Settings and onboarding need to know about one manager.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub id: String,
    pub display_name: String,
    /// The CLI exists somewhere we look.
    pub installed: bool,
    /// The CLI exists AND answered that it has an unlocked session. Both halves
    /// matter: an installed-but-locked `bw` fails every read, and offering it
    /// in onboarding as if it were ready is exactly the false green light this
    /// repo keeps paying for.
    pub signed_in: bool,
    /// Why not, in the CLI's own words, sanitised. `None` when all is well.
    pub detail: Option<String>,
}

/// Probe every supported manager. Never fails: an absent manager is a RESULT.
pub fn probe_backends(timeout: Duration) -> Vec<BackendStatus> {
    Backend::ALL
        .iter()
        .map(|b| probe_backend(*b, timeout))
        .collect()
}

pub fn probe_backend(backend: Backend, timeout: Duration) -> BackendStatus {
    let status = |installed: bool, signed_in: bool, detail: Option<String>| BackendStatus {
        id: backend.id().to_string(),
        display_name: backend.display_name().to_string(),
        installed,
        signed_in,
        detail,
    };

    let Some(program) = resolve_binary(backend) else {
        return status(
            false,
            false,
            Some(format!("No '{}' found on PATH.", backend.binary())),
        );
    };

    // `op whoami` and `bw status` are the cheapest calls that distinguish
    // "installed" from "usable". Neither touches secret material — and their
    // output still goes through the sanitiser, because "safe by construction"
    // is a claim about the CLI as it behaves today.
    let args: &[&str] = match backend {
        Backend::OnePassword => &["whoami"],
        Backend::Bitwarden => &["status"],
    };

    match run_cli(backend, &program, args, timeout) {
        Err(e) => status(true, false, Some(e.to_string())),
        Ok(outcome) => {
            if !outcome.success {
                let detail = if outcome.stderr.is_empty() {
                    format!("'{} {}' failed.", backend.binary(), args.join(" "))
                } else {
                    outcome.stderr
                };
                return status(true, false, Some(detail));
            }
            match backend {
                Backend::OnePassword => status(true, true, None),
                // `bw status` exits 0 whether the vault is unlocked, locked or
                // unauthenticated; only the JSON body says which. Treating exit
                // 0 as "signed in" would advertise a locked vault as ready.
                Backend::Bitwarden => match bitwarden_status(&outcome.stdout) {
                    Some(s) if s == "unlocked" => status(true, true, None),
                    Some(s) => status(
                        true,
                        false,
                        Some(format!(
                            "Vault is {s}. Run 'bw unlock' and export BW_SESSION where \
                             the daemon can see it."
                        )),
                    ),
                    None => status(
                        true,
                        false,
                        Some("Could not read 'bw status' output.".to_string()),
                    ),
                },
            }
        }
    }
}

/// `bw status` prints `{"status":"unlocked", …}`. Parsed rather than
/// substring-matched so "unauthenticated" cannot be mistaken for "unlocked".
fn bitwarden_status(stdout: &SecretValue) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(&stdout.0).ok()?;
    parsed.get("status")?.as_str().map(str::to_string)
}

// ── CLI plumbing ─────────────────────────────────────────────────────────

fn resolve_binary(backend: Backend) -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(backend.binary_override_env()) {
        let path = PathBuf::from(explicit);
        // An override that points at nothing is a configuration mistake, not a
        // reason to silently search elsewhere and report a different binary's
        // answer as if it were the configured one.
        return path.exists().then_some(path);
    }
    // The daemon is started by launchd, which never reads a shell profile, so
    // the inherited PATH routinely lacks /opt/homebrew/bin — where `op` and
    // `bw` actually live on most Macs. SearchPaths is the same widening every
    // other CLI-backed feature here uses.
    crate::config::search_path::SearchPaths::builder()
        .resolve(backend.binary())
        .ok()
}

struct CliOutcome {
    success: bool,
    stdout: SecretValue,
    /// Already sanitised. There is no path to raw stderr from outside this module.
    stderr: String,
}

/// Run a manager CLI with a hard wall-clock bound.
///
/// The shape is lifted from `config_management.rs::PROBE_TIMEOUT`, for the same
/// reason: you cannot time out a blocking call by wrapping the call. There the
/// fix was to move the work onto its own task and time out the JOIN HANDLE;
/// here the equivalent is to keep `&mut Child` on this thread, move only the
/// PIPES onto reader threads, and poll `try_wait` against a deadline — so when
/// the deadline passes we still own the handle and can actually kill the
/// process. Handing the whole `Child` to a worker thread and timing out a
/// channel would report the timeout and then leak a process parked on a
/// biometric prompt for the rest of the daemon's life.
///
/// Draining stdout and stderr concurrently is not optional either: a CLI that
/// fills a 64 KiB pipe buffer while nobody is reading blocks forever, and the
/// timeout would then be measuring our own deadlock rather than the CLI's.
fn run_cli(
    backend: Backend,
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<CliOutcome, SecretSourceError> {
    use crate::subprocess::SubprocessExt;
    use std::io::Read;

    let mut command = Command::new(program);
    command
        .args(args)
        // No stdin. `op` and `bw` prompt when they have a tty; with stdin
        // closed they fail fast and say why instead of blocking to the deadline.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.set_no_window();
    // Own process group, so the deadline below can kill the whole tree. `op`
    // and `bw` are frequently shell shims that exec the real binary as a child.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command.spawn().map_err(|e| SecretSourceError::Failed {
        backend,
        locator: program.display().to_string(),
        detail: sanitize_cli_stderr(&e.to_string()),
    })?;

    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(e) => {
                return Err(SecretSourceError::Failed {
                    backend,
                    locator: program.display().to_string(),
                    detail: sanitize_cli_stderr(&e.to_string()),
                })
            }
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let Some(status) = exit else {
        // Kill the process GROUP, not just the child. A shim that exec'd the
        // real CLI as its own child leaves that grandchild holding these pipes,
        // so killing only the shim lets the reader joins below block until the
        // grandchild exits on its own — the deadline gets reported but never
        // enforced, which is the bug this replaced.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        // Kill, then reap, then let the readers finish — in that order, or the
        // joins block on pipes the dead child still owns.
        let _ = child.kill();
        let _ = child.wait();
        let _ = out_reader.join();
        let _ = err_reader.join();
        return Err(SecretSourceError::Timeout { backend, timeout });
    };

    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();

    Ok(CliOutcome {
        success: status.success(),
        stdout: SecretValue(String::from_utf8_lossy(&stdout).trim_end().to_string()),
        stderr: sanitize_cli_stderr(&String::from_utf8_lossy(&stderr)),
    })
}

/// Chosen below the shortest credential anyone actually issues (a bare 32-hex
/// key) and above the longest word in a CLI diagnostic.
const REDACT_TOKEN_LEN: usize = 24;

/// Reduce a CLI's stderr to something safe to put in an error message.
///
/// stderr has to be surfaced: without it, "1Password could not read the key" is
/// unactionable, and `op`'s own "You are not currently signed in" is exactly
/// what the user needs. But it is text we do not control, so it is treated as
/// hostile:
///
/// * control characters are dropped (no terminal escapes into our logs),
/// * any token of [`REDACT_TOKEN_LEN`] characters or more becomes `[redacted]`,
/// * the result is capped at two lines and 300 characters.
///
/// The length rule is a shape heuristic, deliberately. On a FAILED read there
/// is no resolved value to diff against, so redaction cannot be exact — but
/// every credential worth protecting is a long opaque run of characters, while
/// genuine diagnostics are made of ordinary short words. It over-redacts long
/// paths and URLs; that is the correct side to err on.
pub fn sanitize_cli_stderr(raw: &str) -> String {
    const MAX_LEN: usize = 300;

    let cleaned = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(2)
        .map(|line| {
            line.split_whitespace()
                .map(|token| {
                    if token.chars().count() >= REDACT_TOKEN_LEN {
                        "[redacted]".to_string()
                    } else {
                        token.chars().filter(|c| !c.is_control()).collect()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.chars().count() > MAX_LEN {
        let mut truncated: String = cleaned.chars().take(MAX_LEN).collect();
        truncated.push('…');
        truncated
    } else {
        cleaned
    }
}

fn read_one_password(reference: &str, timeout: Duration) -> Result<SecretValue, SecretSourceError> {
    let backend = Backend::OnePassword;
    let program = resolve_binary(backend).ok_or(SecretSourceError::NotInstalled { backend })?;

    let outcome = run_cli(
        backend,
        &program,
        &["read", "--no-newline", reference],
        timeout,
    )?;
    if outcome.success {
        return Ok(outcome.stdout);
    }
    Err(classify_failure(
        backend,
        reference.to_string(),
        &outcome.stderr,
    ))
}

fn read_bitwarden(
    item: &str,
    field: &str,
    timeout: Duration,
) -> Result<SecretValue, SecretSourceError> {
    let backend = Backend::Bitwarden;
    let program = resolve_binary(backend).ok_or(SecretSourceError::NotInstalled { backend })?;
    let locator = format!("{item}/{field}");

    if BITWARDEN_BUILTIN_FIELDS.contains(&field) {
        let outcome = run_cli(backend, &program, &["get", field, item], timeout)?;
        return if outcome.success {
            Ok(outcome.stdout)
        } else {
            Err(classify_failure(backend, locator, &outcome.stderr))
        };
    }

    // Custom fields have no `bw get <field>` shorthand; they only exist inside
    // the item document.
    let outcome = run_cli(backend, &program, &["get", "item", item], timeout)?;
    if !outcome.success {
        return Err(classify_failure(backend, locator, &outcome.stderr));
    }
    extract_bitwarden_custom_field(&outcome.stdout, field)
        .ok_or(SecretSourceError::NotFound { backend, locator })
}

/// Pull a named custom field out of `bw get item` JSON.
///
/// Kept separate from the subprocess call so the JSON shape is unit-testable
/// without a Bitwarden install.
pub(crate) fn extract_bitwarden_custom_field(
    item_json: &SecretValue,
    field: &str,
) -> Option<SecretValue> {
    let parsed: serde_json::Value = serde_json::from_str(&item_json.0).ok()?;
    parsed
        .get("fields")?
        .as_array()?
        .iter()
        .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(field))
        .and_then(|f| f.get("value"))
        .and_then(|v| v.as_str())
        .map(SecretValue::new)
}

/// Turn a non-zero exit into the most specific error we can justify.
///
/// Substring matching on someone else's CLI output is fragile, so it only ever
/// REFINES the message — an unrecognised failure still surfaces verbatim
/// (sanitised) rather than being flattened into a generic error. A generic
/// error is what sent three separate keychain investigations down the wrong
/// path; the same mistake is not worth repeating one layer out.
fn classify_failure(backend: Backend, locator: String, stderr: &str) -> SecretSourceError {
    let lower = stderr.to_ascii_lowercase();

    if lower.contains("not currently signed in")
        || lower.contains("not signed in")
        || lower.contains("you are not logged in")
        || lower.contains("vault is locked")
        || lower.contains("not unlocked")
        || lower.contains("session key")
        || lower.contains("authentication required")
    {
        return SecretSourceError::NotSignedIn {
            backend,
            detail: stderr.to_string(),
        };
    }

    if lower.contains("isn't an item")
        || lower.contains("not found")
        || lower.contains("no item")
        || lower.contains("doesn't seem to be a vault")
        || lower.contains("could not find")
    {
        return SecretSourceError::NotFound { backend, locator };
    }

    SecretSourceError::Failed {
        backend,
        locator,
        detail: if stderr.is_empty() {
            "The command failed with no output.".to_string()
        } else {
            stderr.to_string()
        },
    }
}

// ── Short-lived success cache ────────────────────────────────────────────

type CacheMap = HashMap<String, (Instant, SecretValue)>;

fn cache() -> &'static Mutex<CacheMap> {
    static CACHE: OnceLock<Mutex<CacheMap>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_get(spec: &str) -> Option<SecretValue> {
    let map = cache().lock().ok()?;
    map.get(spec)
        .filter(|(at, _)| at.elapsed() < CACHE_TTL)
        .map(|(_, v)| v.clone())
}

fn cache_put(spec: &str, value: &SecretValue) {
    if let Ok(mut map) = cache().lock() {
        map.insert(spec.to_string(), (Instant::now(), value.clone()));
    }
}

/// Drop every cached resolution. Called when a source changes in Settings so
/// the next read reflects the new configuration immediately rather than up to
/// `CACHE_TTL` later — a stale "it works" after an edit is the same lie as a
/// silent fallback.
pub fn clear_cache() {
    if let Ok(mut map) = cache().lock() {
        map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_builtin_stores() {
        assert_eq!(
            SecretSource::parse("keychain").unwrap(),
            SecretSource::Keychain
        );
        assert_eq!(
            SecretSource::parse("  Keyring ").unwrap(),
            SecretSource::Keychain
        );
        assert_eq!(SecretSource::parse("file").unwrap(), SecretSource::File);
    }

    #[test]
    fn parses_one_password_reference() {
        let parsed = SecretSource::parse("op://Personal/OpenAI/credential").unwrap();
        assert_eq!(
            parsed,
            SecretSource::OnePassword {
                reference: "op://Personal/OpenAI/credential".to_string()
            }
        );
        assert_eq!(parsed.to_spec(), "op://Personal/OpenAI/credential");
        assert_eq!(parsed.label(), "1Password");
    }

    #[test]
    fn parses_bitwarden_with_and_without_field() {
        assert_eq!(
            SecretSource::parse("bw://OpenAI").unwrap(),
            SecretSource::Bitwarden {
                item: "OpenAI".into(),
                field: "password".into()
            }
        );
        assert_eq!(
            SecretSource::parse("bw://OpenAI/api-key").unwrap(),
            SecretSource::Bitwarden {
                item: "OpenAI".into(),
                field: "api-key".into()
            }
        );
    }

    /// A typo must be an error. If `op://Personal/OpenAI` (two segments) parsed
    /// as anything usable, the user would get a keychain read wearing a
    /// 1Password label — indistinguishable from success, and wrong.
    #[test]
    fn malformed_specs_are_errors_not_fallbacks() {
        for spec in [
            "",
            "1password",
            "op://Personal/OpenAI",
            "op://Personal//credential",
            "bw://",
            "bw://Item/",
            "https://example.com/secret",
        ] {
            let err = SecretSource::parse(spec).unwrap_err();
            assert!(
                matches!(err, SecretSourceError::Malformed { .. }),
                "{spec:?} should be rejected, got {err:?}"
            );
        }
    }

    #[test]
    fn default_source_rejects_manager_references() {
        assert_eq!(
            SecretSource::parse_default("keychain").unwrap(),
            SecretSource::Keychain
        );
        assert_eq!(
            SecretSource::parse_default("file").unwrap(),
            SecretSource::File
        );
        let err = SecretSource::parse_default("op://Personal/OpenAI/credential").unwrap_err();
        assert!(err.to_string().contains("Only \"keychain\" or \"file\""));
    }

    #[test]
    fn precedence_is_per_key_then_default_then_keychain() {
        let mut sources = HashMap::new();
        sources.insert(
            "OPENAI_API_KEY".to_string(),
            "op://Personal/OpenAI/credential".to_string(),
        );

        assert_eq!(
            source_for_key("OPENAI_API_KEY", &sources, Some("file")).unwrap(),
            SecretSource::OnePassword {
                reference: "op://Personal/OpenAI/credential".into()
            },
            "an explicit per-key source outranks the default"
        );
        assert_eq!(
            source_for_key("ANTHROPIC_API_KEY", &sources, Some("file")).unwrap(),
            SecretSource::File,
            "keys with no entry take the configured default"
        );
        assert_eq!(
            source_for_key("ANTHROPIC_API_KEY", &sources, None).unwrap(),
            SecretSource::Keychain,
            "with nothing configured the keychain stays the default"
        );
    }

    /// `config_value!` reads lowercase keys while providers read uppercase
    /// ones. A source matching only the spelling the user happened to type
    /// would apply on some call paths and not others.
    #[test]
    fn key_lookup_is_case_insensitive() {
        let mut sources = HashMap::new();
        sources.insert("openai_api_key".to_string(), "file".to_string());
        assert_eq!(
            source_for_key("OPENAI_API_KEY", &sources, None).unwrap(),
            SecretSource::File
        );
    }

    #[test]
    fn secret_value_debug_never_prints_the_secret() {
        let v = SecretValue::new("sk-proj-abcdefghijklmnopqrstuvwxyz0123456789");
        assert_eq!(format!("{v:?}"), "SecretValue(<redacted>)");
        assert!(!format!("{v:#?}").contains("sk-proj"));
    }

    /// The "not allowed" list is explicit that stderr passthrough counts as a
    /// leak channel. We cannot diff against the value on a failure path — there
    /// is no value — so redaction is by shape, and this pins the shape.
    #[test]
    fn sanitizer_redacts_credential_shaped_tokens_and_keeps_diagnostics() {
        let leaked = "error: sk-proj-abcdefghijklmnopqrstuvwxyz0123456789 rejected";
        let clean = sanitize_cli_stderr(leaked);
        assert!(!clean.contains("sk-proj"), "got: {clean}");
        assert!(clean.contains("[redacted]"));
        assert!(
            clean.contains("rejected"),
            "diagnostics must survive: {clean}"
        );

        let diagnostic = "[ERROR] you are not currently signed in. Run 'op signin'.";
        let kept = sanitize_cli_stderr(diagnostic);
        assert!(kept.contains("not currently signed in"), "got: {kept}");
    }

    #[test]
    fn sanitizer_bounds_length_and_lines() {
        let noisy = (0..50)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let clean = sanitize_cli_stderr(&noisy);
        assert!(clean.contains("line0") && clean.contains("line1"));
        assert!(
            !clean.contains("line3"),
            "only the first two lines: {clean}"
        );

        let long = "a ".repeat(1000);
        assert!(sanitize_cli_stderr(&long).chars().count() <= 301);
    }

    #[test]
    fn sanitizer_strips_control_characters() {
        let clean = sanitize_cli_stderr("plain\u{1b}[31mred\u{7} text");
        assert!(!clean.contains('\u{1b}'));
        assert!(!clean.contains('\u{7}'));
    }

    #[test]
    fn classifies_not_signed_in() {
        let err = classify_failure(
            Backend::OnePassword,
            "op://V/I/f".into(),
            "You are not currently signed in. Please run 'op signin'.",
        );
        assert!(matches!(err, SecretSourceError::NotSignedIn { .. }));
        assert!(err.to_string().contains("not signed in"));
    }

    #[test]
    fn classifies_missing_item() {
        let err = classify_failure(
            Backend::OnePassword,
            "op://V/Missing/f".into(),
            "\"Missing\" isn't an item. Specify the item with its UUID, name, or domain.",
        );
        assert!(matches!(err, SecretSourceError::NotFound { .. }));
        assert!(err.to_string().contains("op://V/Missing/f"));
    }

    /// An unrecognised failure must NOT be flattened into a generic message.
    /// The keychain investigations that cost three wrong diagnoses all started
    /// with an error that had thrown away the underlying cause.
    #[test]
    fn unrecognised_failures_keep_the_cli_message() {
        let err = classify_failure(
            Backend::Bitwarden,
            "Item/password".into(),
            "connect ECONNREFUSED 127.0.0.1:8087",
        );
        assert!(err.to_string().contains("ECONNREFUSED"), "{err}");
    }

    #[test]
    fn extracts_bitwarden_custom_field() {
        let json = SecretValue::new(
            r#"{"name":"OpenAI","fields":[{"name":"api-key","value":"top-secret"}]}"#,
        );
        assert_eq!(
            extract_bitwarden_custom_field(&json, "api-key")
                .unwrap()
                .expose(),
            "top-secret"
        );
        assert!(extract_bitwarden_custom_field(&json, "absent").is_none());
    }

    #[test]
    fn bitwarden_locked_vault_is_not_signed_in() {
        assert_eq!(
            bitwarden_status(&SecretValue::new(r#"{"status":"locked"}"#)).as_deref(),
            Some("locked")
        );
        assert_eq!(
            bitwarden_status(&SecretValue::new(r#"{"status":"unlocked"}"#)).as_deref(),
            Some("unlocked")
        );
        assert!(bitwarden_status(&SecretValue::new("not json")).is_none());
    }

    // ── Stub-CLI tests ───────────────────────────────────────────────────
    //
    // These run a real subprocess, so the timeout and pipe handling are
    // exercised rather than asserted about. The stub stands in for `op` via
    // PERMAGENT_OP_BIN, which is also a genuine escape hatch for installs where
    // the CLI lives somewhere we do not search.

    #[cfg(unix)]
    fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    #[cfg(unix)]
    fn reads_a_value_through_the_cli() {
        clear_cache();
        let dir = tempfile::tempdir().unwrap();
        let bin = stub(dir.path(), "op", "printf 'stub-value'");
        let _guard = env_lock::lock_env([("PERMAGENT_OP_BIN", Some(bin.to_str().unwrap()))]);

        let value = SecretSource::parse("op://V/Read/f")
            .unwrap()
            .resolve(PROBE_TIMEOUT)
            .unwrap()
            .unwrap();
        assert_eq!(value.expose(), "stub-value");
        clear_cache();
    }

    /// The leak test the proposal asks for. The stub fails, and shouts the
    /// secret on BOTH channels on its way out. Neither may reach the error.
    #[test]
    #[cfg(unix)]
    fn a_failed_resolution_never_carries_the_value() {
        clear_cache();
        const SECRET: &str = "sk-live-9f3b2c1d8e7a6b5c4d3e2f1a0b9c8d7e";
        let dir = tempfile::tempdir().unwrap();
        let bin = stub(
            dir.path(),
            "op",
            &format!("printf '{SECRET}'\nprintf 'boom {SECRET} boom' >&2\nexit 1"),
        );
        let _guard = env_lock::lock_env([("PERMAGENT_OP_BIN", Some(bin.to_str().unwrap()))]);

        let err = SecretSource::parse("op://V/Fail/f")
            .unwrap()
            .resolve(PROBE_TIMEOUT)
            .unwrap_err();

        let rendered = format!("{err} {err:?}");
        assert!(!rendered.contains(SECRET), "secret leaked into: {rendered}");
        assert!(
            !rendered.contains("sk-live"),
            "secret leaked into: {rendered}"
        );
        assert!(
            rendered.contains("[redacted]"),
            "expected redaction: {rendered}"
        );
    }

    /// An empty answer is a failure, not an empty key. Storing "" would make
    /// every downstream provider look configured and then 401.
    #[test]
    #[cfg(unix)]
    fn empty_output_is_reported_not_accepted() {
        clear_cache();
        let dir = tempfile::tempdir().unwrap();
        let bin = stub(dir.path(), "op", "printf ''");
        let _guard = env_lock::lock_env([("PERMAGENT_OP_BIN", Some(bin.to_str().unwrap()))]);

        let err = SecretSource::parse("op://V/Empty/f")
            .unwrap()
            .resolve(PROBE_TIMEOUT)
            .unwrap_err();
        assert!(matches!(err, SecretSourceError::Empty { .. }), "{err:?}");
    }

    /// A CLI parked on an approval prompt must not park the daemon with it.
    /// This is the whole reason the runner polls `try_wait` while keeping the
    /// child handle instead of awaiting the read inline.
    #[test]
    #[cfg(unix)]
    fn a_hanging_cli_is_killed_at_the_deadline() {
        clear_cache();
        let dir = tempfile::tempdir().unwrap();
        let bin = stub(dir.path(), "op", "sleep 30");
        let _guard = env_lock::lock_env([("PERMAGENT_OP_BIN", Some(bin.to_str().unwrap()))]);

        let started = Instant::now();
        let err = SecretSource::parse("op://V/Hang/f")
            .unwrap()
            .resolve(Duration::from_millis(200))
            .unwrap_err();

        assert!(matches!(err, SecretSourceError::Timeout { .. }), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the read waited for the hung CLI: {:?}",
            started.elapsed()
        );
    }

    /// A CLI that writes more than a pipe buffer must not deadlock the runner.
    #[test]
    #[cfg(unix)]
    fn large_stderr_does_not_deadlock() {
        clear_cache();
        let dir = tempfile::tempdir().unwrap();
        // ~256 KiB on stderr, well past the 64 KiB pipe buffer.
        let bin = stub(
            dir.path(),
            "op",
            "i=0; while [ $i -lt 4096 ]; do \
             printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' >&2; \
             i=$((i+1)); done; printf 'ok'",
        );
        let _guard = env_lock::lock_env([("PERMAGENT_OP_BIN", Some(bin.to_str().unwrap()))]);

        let value = SecretSource::parse("op://V/Big/f")
            .unwrap()
            .resolve(Duration::from_secs(20))
            .unwrap()
            .unwrap();
        assert_eq!(value.expose(), "ok");
        clear_cache();
    }

    /// A binary-override that points at nothing must report absence, not fall
    /// through to whatever `op` happens to be installed on the machine.
    #[test]
    #[cfg(unix)]
    fn a_dangling_binary_override_reports_not_installed() {
        let _guard = env_lock::lock_env([(
            "PERMAGENT_OP_BIN",
            Some("/nonexistent/definitely/not/here/op"),
        )]);
        assert!(resolve_binary(Backend::OnePassword).is_none());

        let status = probe_backend(Backend::OnePassword, PROBE_TIMEOUT);
        assert!(!status.installed);
        assert!(!status.signed_in);
    }

    #[test]
    #[cfg(unix)]
    fn probe_reports_installed_but_not_signed_in() {
        let dir = tempfile::tempdir().unwrap();
        let bin = stub(
            dir.path(),
            "op",
            "echo 'You are not currently signed in.' >&2\nexit 1",
        );
        let _guard = env_lock::lock_env([("PERMAGENT_OP_BIN", Some(bin.to_str().unwrap()))]);

        let status = probe_backend(Backend::OnePassword, PROBE_TIMEOUT);
        assert!(status.installed);
        assert!(!status.signed_in);
        assert!(status.detail.unwrap().contains("not currently signed in"));
    }

    #[test]
    #[cfg(unix)]
    fn probe_reports_a_locked_bitwarden_vault_as_not_signed_in() {
        let dir = tempfile::tempdir().unwrap();
        let bin = stub(dir.path(), "bw", "printf '{\"status\":\"locked\"}'");
        let _guard = env_lock::lock_env([("PERMAGENT_BW_BIN", Some(bin.to_str().unwrap()))]);

        let status = probe_backend(Backend::Bitwarden, PROBE_TIMEOUT);
        assert!(status.installed, "the CLI is right there");
        assert!(
            !status.signed_in,
            "exit 0 from `bw status` does not mean the vault is usable"
        );
        assert!(status.detail.unwrap().contains("locked"));
    }
}
