//! Deterministic credential / secret scanner — #508.
//!
//! Henry's *judgment* on whether a file is safe to commit is inconsistent (he
//! refused a `.env` one day, committed a connection-string file the next). A
//! prompt cannot be a reliable gate for secret leakage — a 14B (or any) model
//! will sometimes let a secret through. This module is the deterministic,
//! fail-closed backstop: a pure function over `(filename, content)` that
//! recognises credential-*shaped* artefacts the same way every time, with no
//! model in the loop.
//!
//! It lives in the Steward safety core (alongside the protected-branch guard)
//! rather than in a parallel mechanism, and is wired into the worker
//! git-commit/dispatch seam by the goal engine — see
//! `crates/goose/src/agents/platform_extensions/goal_engine.rs`.
//!
//! ## What it matches
//!
//! **Filenames** (catch even binary / unreadable files):
//! `.env` / `.env.<env>` (placeholder `.env.example|sample|template|dist` are
//! exempt), `*.pem`, `*.key` (not `*.pub`), the SSH private keys
//! `id_rsa|id_dsa|id_ecdsa|id_ed25519` (not their `.pub`), any name containing
//! `credentials`, and `*.json` whose name hints at a service account.
//!
//! **Content**: PEM `-----BEGIN ... PRIVATE KEY-----` blocks, DB connection
//! strings carrying a password (`scheme://user:pass@…`), `Authorization: Bearer
//! <token>`, AWS access-key ids (`AKIA…`) and 40-char secret-access-keys,
//! well-known token prefixes (`ghp_`, `github_pat_`, `xox[bp]-`, `sk-`, `AIza`),
//! GCP service-account JSON, and generic `*_KEY|*_SECRET|*_TOKEN|PASSWORD`
//! assignments whose value is a *real* secret (placeholders, env-var references,
//! and short/low-entropy values are deliberately not flagged — the
//! false-positive guard).
//!
//! ## Deliberate non-goals (false-positive tradeoffs)
//!
//! Pure high-entropy-only detection is intentionally NOT done: git SHAs, UUIDs,
//! lockfile checksums and base64 blobs are high-entropy but not secrets, and
//! flagging them would block legitimate commits and erode trust in the guard.
//! Secrets are instead recognised by *structure* (known prefixes, connection
//! shapes, PEM headers) or by a credential-named assignment with a real value.

use lazy_static::lazy_static;
use regex::Regex;

/// A credential-shaped match. Carries enough for a human-readable block message
/// and a stable machine id — never the secret value itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    /// Stable id of the rule that fired (for logs / tests).
    pub rule: &'static str,
    /// Reviewer-facing explanation. Does NOT echo the matched secret.
    pub detail: String,
}

impl SecretFinding {
    fn new(rule: &'static str, detail: impl Into<String>) -> Self {
        Self {
            rule,
            detail: detail.into(),
        }
    }
}

/// Scan a single file by name and content. Filename rules run first (they catch
/// even binary or unreadable files); content rules run second. Returns the first
/// match, or `None` when nothing credential-shaped is found.
pub fn scan_file(path: &str, content: &str) -> Option<SecretFinding> {
    if let Some(f) = scan_path(path) {
        return Some(f);
    }
    scan_content(content)
}

/// Filename-only scan. The basename is matched case-insensitively.
pub fn scan_path(path: &str) -> Option<SecretFinding> {
    let base = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();

    // dotenv files, excluding the conventional committed placeholders.
    if base == ".env" || base.starts_with(".env.") {
        const ENV_PLACEHOLDER_SUFFIXES: &[&str] =
            &[".example", ".sample", ".template", ".dist", ".defaults"];
        if !ENV_PLACEHOLDER_SUFFIXES.iter().any(|s| base.ends_with(s)) {
            return Some(SecretFinding::new(
                "filename_dotenv",
                format!("`{path}` is a dotenv file, which routinely holds secrets"),
            ));
        }
    }

    // SSH / TLS private-key material by extension or well-known basename.
    if base.ends_with(".pem") {
        return Some(SecretFinding::new(
            "filename_pem",
            format!("`{path}` is a PEM file (commonly a private key or certificate bundle)"),
        ));
    }
    if base.ends_with(".key") {
        return Some(SecretFinding::new(
            "filename_key",
            format!("`{path}` is a `.key` file (commonly private key material)"),
        ));
    }
    if matches!(
        base.as_str(),
        "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519"
    ) {
        return Some(SecretFinding::new(
            "filename_ssh_private_key",
            format!("`{path}` is an SSH private key"),
        ));
    }

    // Anything explicitly named for credentials.
    if base.contains("credentials") {
        return Some(SecretFinding::new(
            "filename_credentials",
            format!("`{path}` is named as a credentials file"),
        ));
    }

    // GCP/cloud service-account key files are almost always JSON named for it.
    if base.ends_with(".json")
        && (base.contains("service-account")
            || base.contains("service_account")
            || base.contains("serviceaccount")
            || base.contains("gcp-key")
            || base.contains("gcloud-key"))
    {
        return Some(SecretFinding::new(
            "filename_service_account",
            format!("`{path}` looks like a cloud service-account key file"),
        ));
    }

    None
}

lazy_static! {
    /// PEM private-key block header (RSA / EC / OPENSSH / generic).
    static ref RE_PRIVATE_KEY: Regex =
        Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----").unwrap();
    /// DB/broker connection string carrying an inline password: `scheme://user:pass@`.
    static ref RE_CONN_STRING: Regex = Regex::new(
        r"(?i)\b(postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis|rediss|amqps?)://[^\s:/@]+:[^\s:/@]+@"
    )
    .unwrap();
    /// `Authorization: Bearer <token>` with a non-trivial token.
    static ref RE_BEARER: Regex =
        Regex::new(r"(?i)authorization\s*:\s*bearer\s+[A-Za-z0-9._\-]{16,}").unwrap();
    /// AWS access key id.
    static ref RE_AWS_AKID: Regex = Regex::new(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b").unwrap();
    /// AWS secret access key, as a `aws_secret_access_key = <40 b64 chars>` assignment.
    static ref RE_AWS_SECRET: Regex =
        Regex::new(r#"(?i)aws_secret_access_key\s*[:=]\s*["']?[A-Za-z0-9/+=]{40}"#).unwrap();
    /// Well-known provider token prefixes (GitHub, Slack, OpenAI/Stripe `sk-`, Google).
    static ref RE_TOKEN_PREFIX: Regex = Regex::new(
        r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9]{20,}|AIza[0-9A-Za-z\-_]{30,})\b"
    )
    .unwrap();
    /// GCP service-account JSON body (private_key + service_account type).
    static ref RE_GCP_SA: Regex =
        Regex::new(r#"(?s)"type"\s*:\s*"service_account".*"private_key"\s*:"#).unwrap();
    /// A credential-named assignment: captures (name, value).
    static ref RE_SECRET_ASSIGN: Regex = Regex::new(
        r#"(?i)([A-Za-z0-9_.\-]*(?:api[_-]?key|secret|token|password|passwd|access[_-]?key|private[_-]?key|client[_-]?secret|auth[_-]?token))\s*[:=]\s*(.+)"#
    )
    .unwrap();
}

/// Content-only scan. Returns the first credential-shaped match.
pub fn scan_content(content: &str) -> Option<SecretFinding> {
    if RE_PRIVATE_KEY.is_match(content) {
        return Some(SecretFinding::new(
            "content_private_key",
            "contains a PEM `-----BEGIN ... PRIVATE KEY-----` block",
        ));
    }
    if RE_GCP_SA.is_match(content) {
        return Some(SecretFinding::new(
            "content_service_account",
            "contains a GCP service-account key body (`\"type\": \"service_account\"` + `private_key`)",
        ));
    }
    if RE_CONN_STRING.is_match(content) {
        return Some(SecretFinding::new(
            "content_connection_string",
            "contains a database connection string with an inline password (`scheme://user:pass@…`)",
        ));
    }
    if RE_BEARER.is_match(content) {
        return Some(SecretFinding::new(
            "content_bearer_token",
            "contains an `Authorization: Bearer <token>` header",
        ));
    }
    if RE_AWS_AKID.is_match(content) {
        return Some(SecretFinding::new(
            "content_aws_access_key_id",
            "contains an AWS access key id (`AKIA…`/`ASIA…`)",
        ));
    }
    if RE_AWS_SECRET.is_match(content) {
        return Some(SecretFinding::new(
            "content_aws_secret_key",
            "contains an `aws_secret_access_key` assignment",
        ));
    }
    if RE_TOKEN_PREFIX.is_match(content) {
        return Some(SecretFinding::new(
            "content_token_prefix",
            "contains a token with a well-known secret prefix (e.g. `ghp_`, `xoxb-`, `sk-`, `AIza`)",
        ));
    }

    // Generic credential-named assignments — only when the *value* is a real
    // secret, not a placeholder / env-var reference / short token. Line-scoped so
    // one match doesn't depend on the whole file.
    for line in content.lines() {
        if let Some(caps) = RE_SECRET_ASSIGN.captures(line) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if looks_like_real_secret(value) {
                return Some(SecretFinding::new(
                    "content_secret_assignment",
                    format!("contains a `{name}` assignment with a credential-shaped value"),
                ));
            }
        }
    }

    None
}

lazy_static! {
    /// Values that read as placeholders / instructions / env references, not real
    /// secrets — the core false-positive guard for credential-named assignments.
    static ref RE_PLACEHOLDER: Regex = Regex::new(
        r"(?ix)
        ^(
            x{3,} | \*{3,} | \.{3,} | -+ | _+ |
            change[_-]?me | your[_-]?\w* | my[_-]?\w* |
            none | null | nil | true | false | undefined |
            example | placeholder | redacted | dummy | sample | fixme | todo |
            replace[_-]?me | insert[_-]?\w* | enter[_-]?\w* |
            secret | password | passwd | token | apikey | api[_-]?key | key |
            test | testing | localhost
        )$"
    )
    .unwrap();
}

/// Whether an assignment's right-hand side looks like a *real* secret rather
/// than a placeholder, an env-var reference, or a value too short/uniform to be
/// a credential. Conservative on the "not a secret" side to avoid false blocks.
fn looks_like_real_secret(raw: &str) -> bool {
    // Strip a trailing comment, surrounding quotes, and trailing punctuation.
    let mut v = raw.trim();
    if let Some((head, _)) = v.split_once(" #") {
        v = head.trim_end();
    }
    if let Some((head, _)) = v.split_once(" //") {
        v = head.trim_end();
    }
    v = v.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    v = v.trim_end_matches([',', ';']);
    v = v.trim();

    if v.len() < 8 {
        return false;
    }
    // A real credential is a single token. A value with internal whitespace is
    // prose (e.g. an instructional comment "set API_KEY=your_key in the env"),
    // not a secret.
    if v.split_whitespace().count() != 1 {
        return false;
    }
    // Env-var / template references: `$VAR`, `${VAR}`, `{{ var }}`, `%VAR%`,
    // `process.env.X`, `os.environ[...]`, `<...>`.
    if v.starts_with('$')
        || v.starts_with("{{")
        || (v.starts_with('<') && v.ends_with('>'))
        || (v.starts_with('%') && v.ends_with('%'))
        || v.contains("process.env")
        || v.contains("os.environ")
        || v.contains("ENV[")
        || v.contains("getenv")
    {
        return false;
    }
    if RE_PLACEHOLDER.is_match(v) {
        return false;
    }
    // All-identical characters (e.g. "xxxxxxxx", "00000000") are not secrets.
    if let Some(first) = v.chars().next() {
        if v.chars().all(|c| c == first) {
            return false;
        }
    }
    true
}

// ── Self-knowledge descriptor (#508 / #353) ──────────────────────────────────

/// The credential-commit guard as a capability Henry is *subject to* and must be
/// able to describe. Surfaced in the `<permagent_self>` brief under "Guardrails".
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "credential_commit_guard",
        display_name: "Credential commit guard",
        category: crate::agents::self_knowledge::FeatureCategory::Guard,
        what_it_does:
            "A deterministic scanner inspects every file a dispatched worker commits and \
             blocks the goal from advancing if it finds credential-shaped content — \
             dotenv/key/PEM files, connection strings with passwords, API keys, tokens, or \
             private keys",
        why_it_matters:
            "It is a code backstop, not a judgment call: even if you (or a worker) decide a \
             secret-looking file is fine to commit, this guard stops it and surfaces the \
             reason. Never rely on prompt judgment alone to keep credentials out of git",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: &str, content: &str) -> Option<&'static str> {
        scan_file(path, content).map(|f| f.rule)
    }

    // ── Filename rules: positives ──
    #[test]
    fn flags_credential_filenames() {
        for path in [
            ".env",
            ".env.production",
            "config/.env.local",
            "server.pem",
            "tls/private.key",
            "deploy/id_rsa",
            "secrets/id_ed25519",
            "aws-credentials",
            "my_credentials.txt",
            "gcp-service-account.json",
            "infra/service_account_key.json",
        ] {
            assert!(
                scan_path(path).is_some(),
                "{path} should be flagged by filename"
            );
        }
    }

    // ── Filename rules: false-positive guards ──
    #[test]
    fn allows_safe_filenames() {
        for path in [
            ".env.example",
            ".env.sample",
            ".env.template",
            "config/.env.dist",
            "id_rsa.pub",
            "id_ed25519.pub",
            "README.md",
            "src/main.rs",
            "package.json",
            "tsconfig.json",
            "keyboard.rs", // contains "key" but not a .key file
            "monkey.txt",  // substring "key" must not trip
        ] {
            assert!(
                scan_path(path).is_none(),
                "{path} should NOT be flagged by filename"
            );
        }
    }

    // ── Content rules: positives ──
    #[test]
    fn flags_private_key_block() {
        let body =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
        assert_eq!(rule("notes.txt", body), Some("content_private_key"));
    }

    #[test]
    fn flags_openssh_private_key_block() {
        let body = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk...\n";
        assert_eq!(rule("k", body), Some("content_private_key"));
    }

    #[test]
    fn flags_connection_strings_with_password() {
        for s in [
            "DATABASE_URL=postgres://admin:s3cr3tpw@db.example.com:5432/app",
            "uri: mongodb+srv://user:Pa55word@cluster0.mongodb.net/test",
            "redis://default:longpasswordhere@redis:6379",
            "mysql://root:hunter2pass@localhost/db",
        ] {
            assert_eq!(
                rule("config.txt", s),
                Some("content_connection_string"),
                "should flag: {s}"
            );
        }
    }

    #[test]
    fn flags_bearer_aws_and_token_prefixes() {
        assert_eq!(
            rule("h", "Authorization: Bearer abcdef0123456789ghijkl"),
            Some("content_bearer_token")
        );
        assert_eq!(
            rule("c", "key id AKIAIOSFODNN7EXAMPLE here"),
            Some("content_aws_access_key_id")
        );
        assert_eq!(
            rule("t", "token=ghp_0123456789abcdefABCDEF0123456789abcd"),
            Some("content_token_prefix")
        );
        assert_eq!(
            rule("t", "OPENAI=sk-0123456789abcdefABCDEFghij"),
            Some("content_token_prefix")
        );
    }

    #[test]
    fn flags_real_secret_assignment() {
        for line in [
            r#"API_KEY = "8f3a9c2b7d1e4f6a0b5c9d2e1f7a3b6c""#,
            "client_secret: 9aXk2Lp7QzRm4Vn8Bf1Hc3Jd5Tg6Yw",
            "DB_PASSWORD=Sup3rS3cretP@ssw0rd!",
        ] {
            assert_eq!(
                rule("settings.yaml", line),
                Some("content_secret_assignment"),
                "should flag: {line}"
            );
        }
    }

    #[test]
    fn flags_gcp_service_account_body() {
        let body =
            r#"{ "type": "service_account", "project_id": "x", "private_key": "-----BEGIN..." }"#;
        // Body matches the SA rule before the generic private-key rule on the same string.
        assert!(scan_content(body).is_some());
    }

    // ── Content rules: false-positive guards ──
    #[test]
    fn ignores_placeholder_and_reference_assignments() {
        for line in [
            "API_KEY=your_api_key_here",
            "API_KEY=changeme",
            "SECRET=${APP_SECRET}",
            "TOKEN=$GITHUB_TOKEN",
            "password: <your-password>",
            "DB_PASSWORD={{ db_password }}",
            "client_secret = process.env.CLIENT_SECRET",
            "api_key = ''",
            "PASSWORD=xxx",
            "# To configure, set API_KEY=your_key in the environment",
            "key: example",
            "ACCESS_KEY=",
        ] {
            assert_eq!(
                scan_content(line),
                None,
                "placeholder/reference should NOT be flagged: {line}"
            );
        }
    }

    #[test]
    fn ignores_connection_string_without_password() {
        for s in [
            "postgres://localhost:5432/app",
            "mongodb://db:27017/test",
            "redis://cache:6379",
        ] {
            assert_eq!(scan_content(s), None, "no inline password: {s}");
        }
    }

    #[test]
    fn ignores_high_entropy_non_secrets() {
        // git SHAs, UUIDs and lockfile hashes are high-entropy but not secrets,
        // and carry no credential-named key or known prefix.
        for s in [
            "commit a1b2c3d4e5f60718293a4b5c6d7e8f9012345678",
            "id = 550e8400-e29b-41d4-a716-446655440000",
            "sha256 = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ] {
            assert_eq!(scan_content(s), None, "non-secret high entropy: {s}");
        }
    }

    #[test]
    fn scan_file_prefers_filename_then_content() {
        // Filename rule wins even with benign content.
        assert_eq!(rule(".env", "PORT=8080"), Some("filename_dotenv"));
        // Clean filename falls through to content.
        assert_eq!(
            rule("app.rs", "let url = \"postgres://u:p@h/db\";"),
            Some("content_connection_string")
        );
        // Clean both → no finding.
        assert_eq!(rule("app.rs", "let port = 8080;"), None);
    }
}
