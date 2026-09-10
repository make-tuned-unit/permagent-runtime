//! Tier 0 — the $0 offline harness-loop metrics.
//!
//! `docs/design/HARNESS_BENCH_RESEARCH.md` §5 calls this "the highest-ROI tier
//! and it should exist regardless": mock the model and assert the *loop*
//! behaves, deterministically, for zero spend, on every commit. Tiers 1 and 2
//! (a held-constant Haiku column, then the bigger suites) only earn their money
//! once this one is green.
//!
//! ## How it is $0 and still real
//!
//! It runs the REAL `permagent` binary — same reply loop, same tool execution,
//! same context management — and replaces only the model. A scripted stub HTTP
//! server ([`StubServer`]) speaks the OpenAI chat-completions shape on
//! `127.0.0.1:<ephemeral>`, handing back a fixed sequence of assistant turns.
//! The harness reaches it through a **declarative provider** written into the
//! run's own `$PERMAGENT_PATH_ROOT/custom_providers/` (see [`provider_json`]),
//! so nothing on the machine is configured and no key is needed
//! (`requires_auth: false` → the OpenAI provider selects `NoAuth`).
//!
//! The measurement is not the model's answer — the model is a puppet. It is the
//! **sequence of requests the harness made**: how many turns it took, what it
//! put in the context, what it dropped. That is exactly the harness behaviour
//! §3 of the research doc says matters, and it is observable from outside.
//!
//! ## Why `supports_streaming: false`
//!
//! The built-in `openai` provider hard-codes streaming, but a declarative
//! provider may turn it off, which lets the stub answer with one plain JSON
//! body instead of an SSE frame sequence. Fewer moving parts in the measuring
//! instrument is worth more here than fidelity to the wire format, because the
//! thing under test is the loop, not the transport.
//!
//! ## What each scenario measures
//!
//! | scenario | the behaviour, and why it is the metric |
//! |---|---|
//! | `retry-on-malformed-tool-call` | a tool call with unparseable arguments must be *repaired*, not fatal. Aider's 52%→88% came from exactly this repair pass. |
//! | `tool-result-pruning` | an enormous tool result must be truncated in the next prompt, or context is spent on spew. |
//! | `premature-done-guard` | a turn that claims done without verifying must be held. Measured as a **differential**: the same script with the guard on vs off. |
//! | `compaction-preserve-list` | when the context is compacted, the working state (the file under edit, the test command) must survive and the spew must not. |
//!
//! Scenario checks are written against *structural* observations — request
//! counts, message counts, sentinel strings the scenario itself planted —
//! rather than against harness prose, so they do not go stale every time a
//! prompt is reworded.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The provider id (and file stem) the stub registers itself under.
pub const STUB_PROVIDER: &str = "permagent_eval_stub";
/// The model name the stub answers to. Deliberately neutral: the OpenAI
/// provider routes `gpt-5*`-shaped names to a different endpoint with a
/// different payload, which would silently take the harness off the path this
/// tier is measuring.
pub const STUB_MODEL: &str = "stub-model";

// ---------------------------------------------------------------------------
// The scripted model
// ---------------------------------------------------------------------------

/// One scripted assistant turn.
#[derive(Debug, Clone, PartialEq)]
pub enum StubTurn {
    /// A plain text answer — the model "finishing".
    Text(String),
    /// A tool call. `arguments` is the raw string the harness will be asked to
    /// parse, so a scenario can hand over deliberately malformed JSON.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
}

impl StubTurn {
    pub fn text(s: impl Into<String>) -> Self {
        StubTurn::Text(s.into())
    }

    pub fn shell(id: &str, command: &str) -> Self {
        StubTurn::ToolCall {
            id: id.to_string(),
            name: "shell".to_string(),
            arguments: json!({ "command": command }).to_string(),
        }
    }

    /// A MUTATING tool call. The premature-done guard is gated on "did this turn
    /// actually change something?" -- by design, so that wiring a verify-hold
    /// into every chat turn does not hold every chat turn -- so a scenario that
    /// only talks can never exercise it.
    pub fn write(id: &str, path: &str, content: &str) -> Self {
        StubTurn::ToolCall {
            id: id.to_string(),
            name: "write".to_string(),
            arguments: json!({ "path": path, "content": content }).to_string(),
        }
    }

    /// A tool call whose arguments are not valid JSON at all.
    pub fn malformed_shell(id: &str) -> Self {
        StubTurn::ToolCall {
            id: id.to_string(),
            name: "shell".to_string(),
            arguments: "{\"command\": ".to_string(),
        }
    }

    /// Render as an OpenAI non-streaming chat-completion body.
    ///
    /// Required by `crates/goose/src/providers/formats/openai.rs`
    /// `response_to_message`: `choices[0].message` must exist; a tool call
    /// needs `id`, `function.name` and `function.arguments` as a
    /// JSON-*encoded* string.
    pub fn to_completion(&self, model: &str, prompt_tokens: usize) -> Value {
        let message = match self {
            StubTurn::Text(t) => json!({ "role": "assistant", "content": t }),
            StubTurn::ToolCall {
                id,
                name,
                arguments,
            } => json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments },
                }],
            }),
        };
        let finish = match self {
            StubTurn::Text(_) => "stop",
            StubTurn::ToolCall { .. } => "tool_calls",
        };
        json!({
            "id": "stub-completion",
            "object": "chat.completion",
            "created": 0,
            "model": model,
            "choices": [{ "index": 0, "message": message, "finish_reason": finish }],
            // Usage must be PLAUSIBLE, not merely present. The harness trusts a
            // provider's reported total over its own local estimate
            // (`check_if_compaction_needed` reads `session.total_tokens` first),
            // so a hardcoded `total_tokens: 2` silently told it the context was
            // empty and auto-compaction could never trigger no matter how large
            // the prompt actually grew. These are still never billed: the tier
            // reports $0 and asserts the stub was the only model called.
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": 1,
                "total_tokens": prompt_tokens + 1,
            },
        })
    }
}

/// One request the harness made, as the stub saw it. This is the measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub model: String,
    /// Every message in the prompt, flattened to text (tool results included).
    pub message_texts: Vec<String>,
    /// Number of messages in the prompt — the compaction signal.
    pub message_count: usize,
    /// Tool names offered to the model.
    pub tool_names: Vec<String>,
}

impl RecordedRequest {
    fn from_body(body: &Value) -> Self {
        let messages = body
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        RecordedRequest {
            model: body
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message_count: messages.len(),
            message_texts: messages.iter().map(flatten_text).collect(),
            tool_names: body
                .get("tools")
                .and_then(Value::as_array)
                .map(|ts| {
                    ts.iter()
                        .filter_map(|t| {
                            t.pointer("/function/name")
                                .or_else(|| t.get("name"))
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Whether any message in this prompt contains `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.message_texts.iter().any(|m| m.contains(needle))
    }

    /// Whether any message contains `needle`, ignoring case — for matching
    /// harness prose (an error explanation) without pinning its wording.
    pub fn contains_ignore_case(&self, needle: &str) -> bool {
        any_contains_ignore_case(&self.message_texts, needle)
    }

    /// The messages this prompt has that `prev` did not — i.e. what the turn
    /// actually added.
    ///
    /// Searching the WHOLE prompt for a word like "error" is a trap: the system
    /// prompt is in every request and is full of such words, so a check written
    /// that way passes whatever the harness does. Only the appended turn is
    /// evidence about the turn. Returns empty when the prompt did not grow
    /// (e.g. it was compacted), which is correctly treated as "no evidence".
    pub fn appended_since(&self, prev: &RecordedRequest) -> &[String] {
        self.message_texts.get(prev.message_count..).unwrap_or(&[])
    }

    /// Total characters of prompt — the crude context-size signal.
    pub fn total_chars(&self) -> usize {
        self.message_texts.iter().map(String::len).sum()
    }
}

/// Did this request GAIN a signal the previous one did not carry?
///
/// `appended_since` only sees messages a turn appended, which is the wrong lens
/// for feedback the harness delivers by AMENDING a message that already exists.
/// The malformed-tool-call guidance is exactly that: the harness rewrites the
/// per-turn status block rather than appending a message, so the message count
/// is unchanged and an append-only check reported "no sign the call failed"
/// about a prompt that carried a precise parse error and instructions to
/// re-issue the call. Comparing presence across the two requests catches both
/// shapes.
pub fn gained_signal(prev: &RecordedRequest, cur: &RecordedRequest, needles: &[&str]) -> bool {
    needles.iter().any(|n| {
        any_contains_ignore_case(&cur.message_texts, n)
            && !any_contains_ignore_case(&prev.message_texts, n)
    })
}

/// Case-insensitive substring search across a set of messages.
pub fn any_contains_ignore_case(texts: &[String], needle: &str) -> bool {
    let needle = needle.to_lowercase();
    texts.iter().any(|m| m.to_lowercase().contains(&needle))
}

/// Flatten a message value to searchable text, whatever shape it takes
/// (string content, a content-part array, or a tool result).
fn flatten_text(msg: &Value) -> String {
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::String(s) => {
                out.push_str(s);
                out.push('\n');
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            Value::Object(map) => map.values().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(msg, &mut out);
    out
}

// ---------------------------------------------------------------------------
// The stub server
// ---------------------------------------------------------------------------

/// Shared state between the serving thread and the caller.
#[derive(Default)]
struct StubState {
    script: Vec<StubTurn>,
    next: usize,
    requests: Vec<RecordedRequest>,
}

/// A single-threaded, dependency-free HTTP server that answers
/// `POST /v1/chat/completions` from a script and records what it was asked.
///
/// Hand-rolled rather than pulled from a crate on purpose: this crate's
/// dependency list is deliberately tiny (it must not drag the agent runtime
/// into a CI eval build), and the protocol surface needed here is one route,
/// one method, `Content-Length`-framed.
pub struct StubServer {
    port: u16,
    state: Arc<Mutex<StubState>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl StubServer {
    /// Bind an ephemeral loopback port and start serving `script`.
    ///
    /// Once the script is exhausted every further request gets a terminal
    /// "done" text turn, so a harness that loops more than expected finishes
    /// instead of hanging — and the extra requests are still recorded, which is
    /// itself a finding.
    pub fn start(script: Vec<StubTurn>) -> Result<Self> {
        let listener =
            TcpListener::bind("127.0.0.1:0").context("binding the tier-0 stub server")?;
        let port = listener.local_addr()?.port();
        listener
            .set_nonblocking(true)
            .context("setting the stub listener non-blocking")?;

        let state = Arc::new(Mutex::new(StubState {
            script,
            next: 0,
            requests: Vec::new(),
        }));
        let shutdown = Arc::new(AtomicBool::new(false));

        let thread_state = Arc::clone(&state);
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = std::thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // The listener is non-blocking so the accept loop can
                        // poll for shutdown, and on macOS an ACCEPTED socket
                        // inherits that flag. `serve_one` then reads the request
                        // headers, gets `WouldBlock` before the client's bytes
                        // have arrived, bails, and drops the connection without
                        // writing anything — the caller sees EOF and an empty
                        // response. That made the stub's own tests fail roughly
                        // one run in three, and would silently abort a real
                        // scenario the same way. Requests are served blocking.
                        if let Err(error) = stream.set_nonblocking(false) {
                            eprintln!(
                                "tier0 stub: could not make the connection blocking: {error}"
                            );
                            continue;
                        }
                        let _ = serve_one(stream, &thread_state);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(StubServer {
            port,
            state,
            shutdown,
            handle: Some(handle),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", self.port)
    }

    /// Every request received so far, oldest first.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state.lock().expect("stub state").requests.clone()
    }

    /// Stop serving and return the recorded requests.
    pub fn stop(mut self) -> Vec<RecordedRequest> {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.state.lock().expect("stub state").requests.clone()
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Read one HTTP request, record it, and answer with the next scripted turn.
fn serve_one(stream: TcpStream, state: &Arc<Mutex<StubState>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    // Head.
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }

    // Body.
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

    // Only a body carrying `messages` is a chat completion. Anything else that
    // reaches this port -- a provider probe, a warmup, a models lookup -- must
    // be answered without touching the script.
    //
    // Recording those shifted every check by one: `reqs.get(1)`, meant to be
    // the follow-up that carries the tool result, was really the FIRST prompt,
    // before any tool had run. Worse, advancing the cursor spent the scripted
    // tool call on the probe, so the first real request got the NEXT turn and
    // no tool was ever dispatched. That single fault produced three failures
    // that looked independent: nothing to prune, nothing large enough to
    // compact, and a guard control that counted the probe as an extra turn.
    let is_completion = parsed.get("messages").and_then(Value::as_array).is_some();

    let mut prompt_tokens = 1usize;
    let turn = {
        let mut st = state.lock().expect("stub state");
        if !is_completion {
            StubTurn::text("ok")
        } else {
            let recorded = RecordedRequest::from_body(&parsed);
            // Roughly four characters per token: enough for a threshold test,
            // and it moves with the prompt instead of being a constant lie.
            prompt_tokens = recorded.total_chars() / 4;
            st.requests.push(recorded);
            let turn = st.script.get(st.next).cloned();
            st.next += 1;
            // An exhausted script means "stop", not "hang".
            turn.unwrap_or_else(|| StubTurn::text("done"))
        }
    };

    let payload = serde_json::to_vec(&turn.to_completion(STUB_MODEL, prompt_tokens))?;
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )?;
    stream.write_all(&payload)?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

// ---------------------------------------------------------------------------
// Provider wiring
// ---------------------------------------------------------------------------

/// The declarative-provider JSON that points the harness at the stub.
///
/// Written to `<data_root>/custom_providers/<STUB_PROVIDER>.json`, which the
/// harness loads from `Paths::config_dir()` — and `Paths::base_dir()` honours
/// `PERMAGENT_PATH_ROOT`, which the eval already sets per run. So the stub is
/// registered for exactly one run and leaves nothing on the machine.
pub fn provider_json(base_url: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "name": STUB_PROVIDER,
        "engine": "openai",
        "display_name": "permagent-eval tier-0 stub",
        "description": "Scripted local model for $0 harness-loop metrics. Never leaves loopback.",
        "api_key_env": "",
        "base_url": base_url,
        "models": [{ "name": STUB_MODEL, "context_limit": 128000 }],
        // Non-streaming keeps the stub a single JSON body instead of an SSE
        // frame sequence; only a declarative provider may ask for this.
        "supports_streaming": false,
        "requires_auth": false,
        "skip_canonical_filtering": true,
    }))
    .expect("provider json is a literal")
}

/// Install the stub provider under `data_root` (the run's `PERMAGENT_PATH_ROOT`).
pub fn install_provider(data_root: &Path, base_url: &str) -> Result<()> {
    let dir = data_root.join("custom_providers");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{STUB_PROVIDER}.json"));
    std::fs::write(&path, provider_json(base_url))
        .with_context(|| format!("writing {}", path.display()))
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// Sentinels a scenario plants so its check can be structural rather than
/// dependent on harness prose.
pub const HEAD_SENTINEL: &str = "PERMAGENT_EVAL_HEAD_SENTINEL";
pub const MID_SENTINEL: &str = "PERMAGENT_EVAL_MID_SENTINEL";
pub const TAIL_SENTINEL: &str = "PERMAGENT_EVAL_TAIL_SENTINEL";
pub const KEEP_SENTINEL: &str = "PERMAGENT_EVAL_KEEP_SENTINEL";

/// A shell command that prints a very large output with sentinels at the head,
/// middle and tail — enough to cross the harness's large-result offload
/// threshold (20k tokens; the offload keeps ~2k chars of head and tail).
fn huge_output_command() -> String {
    // The sentinels are ASSEMBLED at runtime from halves, so no complete
    // sentinel string ever appears in the command itself.
    //
    // This is load-bearing. The assistant's tool call is echoed back in the
    // next prompt, command and all, so a command containing the literal
    // sentinels put every sentinel into the prompt regardless of what the tool
    // returned. The check then reported "the middle reached the prompt
    // verbatim — no pruning happened" while matching its own command text: it
    // could not distinguish surviving OUTPUT from a mentioned command.
    let split = |s: &str| {
        let (a, b) = s.split_at(s.len() / 2);
        format!("printf '%s%s\\n' '{a}' '{b}'")
    };
    format!(
        "{}; \
         for i in $(seq 1 2000); do printf 'filler line %s xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' \"$i\"; done; \
         {}; \
         for i in $(seq 1 2000); do printf 'filler line %s yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy\\n' \"$i\"; done; \
         {}",
        split(HEAD_SENTINEL),
        split(MID_SENTINEL),
        split(TAIL_SENTINEL)
    )
}

/// The verdict of one scenario, with the observation that produced it — a bare
/// pass/fail would make a red CI run un-debuggable without a rerun.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub passed: bool,
    pub detail: String,
}

impl CheckOutcome {
    pub fn pass(detail: impl Into<String>) -> Self {
        CheckOutcome {
            passed: true,
            detail: detail.into(),
        }
    }
    pub fn fail(detail: impl Into<String>) -> Self {
        CheckOutcome {
            passed: false,
            detail: detail.into(),
        }
    }
}

/// One $0 loop metric: a script for the puppet model, the environment to run
/// the harness under, and a structural check over what the harness asked for.
#[derive(Debug, Clone)]
pub struct Tier0Scenario {
    pub id: &'static str,
    /// What loop behaviour this measures, and why it is worth measuring.
    pub measures: &'static str,
    /// The user prompt handed to the harness.
    pub prompt: &'static str,
    /// Environment overrides for the harness child.
    pub env: &'static [(&'static str, &'static str)],
    /// The scripted assistant turns.
    pub script: fn() -> Vec<StubTurn>,
    /// The measurement, over the requests the harness made.
    pub check: fn(&[RecordedRequest]) -> CheckOutcome,
}

/// Every Tier-0 scenario.
pub fn scenarios() -> Vec<Tier0Scenario> {
    vec![
        Tier0Scenario {
            id: "retry-on-malformed-tool-call",
            measures: "a tool call whose arguments are not valid JSON must be repaired and the \
                       loop must continue, not abort. Aider's 52%→88% jump came from exactly \
                       this repair pass, so a regression here is expensive.",
            prompt: "Print the contents of the current directory, then say done.",
            env: &[],
            script: || {
                vec![
                    StubTurn::malformed_shell("call_bad"),
                    StubTurn::text("done"),
                ]
            },
            check: |reqs| {
                if reqs.len() < 2 {
                    return CheckOutcome::fail(format!(
                        "the harness stopped after {} request(s): a malformed tool call ended \
                         the run instead of being repaired",
                        reqs.len()
                    ));
                }
                // Only what the turn ADDED counts: the system prompt is in
                // every request and already contains words like "error".
                let told = gained_signal(
                    &reqs[0],
                    &reqs[1],
                    &["could not be parsed", "did not run", "nothing was executed"],
                );
                if told {
                    CheckOutcome::pass(format!(
                        "the loop continued for {} request(s) and the model was told the call \
                         failed, so it could repair it",
                        reqs.len()
                    ))
                } else {
                    CheckOutcome::fail(
                        "the loop continued, but the follow-up prompt carries no sign the tool \
                         call failed — the model is being asked to retry blind"
                            .to_string(),
                    )
                }
            },
        },
        Tier0Scenario {
            id: "tool-result-pruning",
            measures: "an enormous tool result must be reduced before it is fed back, or a single \
                       `cat` spends the whole context window. The shell tool's contract is a \
                       TAIL preview plus a saved path, not a head: command output matters most \
                       at the end, and the full text stays recoverable.",
            prompt: "Run the command you are given and then say done.",
            env: &[],
            script: || {
                vec![
                    StubTurn::shell("call_big", &huge_output_command()),
                    StubTurn::text("done"),
                ]
            },
            check: |reqs| {
                let Some(follow_up) = reqs.get(1) else {
                    return CheckOutcome::fail(
                        "no second request: the tool result was never fed back".to_string(),
                    );
                };
                let added = follow_up.appended_since(&reqs[0]);
                let bytes: usize = added.iter().map(String::len).sum();
                let tail = any_contains_ignore_case(added, TAIL_SENTINEL);
                let mid = any_contains_ignore_case(added, MID_SENTINEL);
                let told = any_contains_ignore_case(added, "exceeded")
                    && any_contains_ignore_case(added, "limit");

                if bytes >= 40_000 {
                    return CheckOutcome::fail(format!(
                        "the result was not bounded: {bytes} chars of a ~200KB result reached the \
                         prompt"
                    ));
                }
                if mid {
                    return CheckOutcome::fail(format!(
                        "the middle of a ~200KB result survived in {bytes} chars, so this was not \
                         reduced to a preview"
                    ));
                }
                if !tail {
                    return CheckOutcome::fail(format!(
                        "the result was reduced to {bytes} chars but the END of the output did not \
                         survive — the model cannot see how the command finished, which is the \
                         part that says whether it worked"
                    ));
                }
                if !told {
                    return CheckOutcome::fail(format!(
                        "the result was reduced to {bytes} chars and the tail survived, but the \
                         model was not told it was truncated — it cannot know output is missing \
                         or where the full text was saved"
                    ));
                }
                CheckOutcome::pass(format!(
                    "a ~200KB result became {bytes} chars: the tail survived, the middle did not, \
                     and the model was told it was truncated"
                ))
            },
        },
        Tier0Scenario {
            id: "premature-done-guard-on",
            measures: "a turn that claims completion without having run any verification must be \
                       held. Devin's 13.86%→23% came from test-driven self-verification; this is \
                       the harness half of it.",
            prompt: "Fix the bug in the project, verify it, and tell me when you are done.",
            env: &[("PERMAGENT_AFTER_TURN_HOLD", "1")],
            script: || {
                vec![
                    // Change code first: the guard judges only a turn that
                    // mutated something.
                    StubTurn::write("call_edit", "fix.py", "def fixed():\n    return 1\n"),
                    StubTurn::text("All done — the bug is fixed."),
                    StubTurn::text("Verified and done."),
                ]
            },
            check: |reqs| {
                // 1: opening ask -> write. 2: after the edit -> unverified
                // 'done'. A hold produces a 3rd.
                if reqs.len() >= 3 {
                    CheckOutcome::pass(format!(
                        "the unverified 'done' was held after a real edit: the harness asked the \
                         model again ({} requests total)",
                        reqs.len()
                    ))
                } else {
                    CheckOutcome::fail(
                        "the harness accepted 'done' after an unverified edit — the premature-done \
                         guard did not fire"
                            .to_string(),
                    )
                }
            },
        },
        Tier0Scenario {
            id: "premature-done-guard-off",
            measures: "the control for the scenario above: with the guard disabled the same \
                       script must NOT be held. Without this half, a harness that loops for \
                       unrelated reasons would score a false pass on the guard.",
            prompt: "Fix the bug in the project, verify it, and tell me when you are done.",
            env: &[("PERMAGENT_AFTER_TURN_HOLD", "0")],
            script: || {
                vec![
                    // The SAME script as the guard-on arm, edit included, so the
                    // only difference between the two runs is the guard.
                    StubTurn::write("call_edit", "fix.py", "def fixed():\n    return 1\n"),
                    StubTurn::text("All done — the bug is fixed."),
                    StubTurn::text("Verified and done."),
                ]
            },
            check: |reqs| {
                if reqs.len() == 2 {
                    CheckOutcome::pass(
                        "with the guard off the unverified 'done' ended the run, so the held turn \
                         in the paired scenario is attributable to the guard"
                            .to_string(),
                    )
                } else {
                    CheckOutcome::fail(format!(
                        "the guard is off yet the harness made {} requests — the extra turns in \
                         the guard-on scenario cannot be attributed to the guard",
                        reqs.len()
                    ))
                }
            },
        },
        Tier0Scenario {
            id: "compaction-preserve-list",
            measures: "when the context is compacted, the working state must survive and the \
                       spew must not. A compaction that forgets which file was being edited \
                       costs more turns than it saves.",
            prompt: "Work through the task, running the commands you are given.",
            // A small window plus an eager threshold forces compaction after a
            // couple of large tool results instead of after a real session.
            env: &[
                ("GOOSE_CONTEXT_LIMIT", "8000"),
                ("GOOSE_AUTO_COMPACT_THRESHOLD", "0.5"),
            ],
            script: || {
                vec![
                    StubTurn::shell("keep", &format!("printf '%s\\n' {KEEP_SENTINEL}")),
                    StubTurn::shell("big1", &huge_output_command()),
                    StubTurn::shell("big2", &huge_output_command()),
                    StubTurn::text("done"),
                ]
            },
            check: |reqs| {
                if reqs.len() < 3 {
                    return CheckOutcome::fail(format!(
                        "only {} request(s): the run never got far enough to compact",
                        reqs.len()
                    ));
                }
                // Compaction shows up as a prompt that got SHORTER than an
                // earlier one despite more turns having happened.
                let peak = reqs.iter().map(|r| r.total_chars()).max().unwrap_or(0);
                let last = reqs.last().expect("checked non-empty");
                let compacted = last.total_chars() < peak;
                let kept = last.contains(KEEP_SENTINEL);
                let spew = last.contains(MID_SENTINEL);
                if !compacted {
                    return CheckOutcome::fail(format!(
                        "the prompt never shrank (peak {peak} chars, final {} chars) — no \
                         compaction happened, so the preserve-list was not exercised",
                        last.total_chars()
                    ));
                }
                match (kept, spew) {
                    (true, false) => CheckOutcome::pass(format!(
                        "compaction shrank the prompt from {peak} to {} chars, kept the working \
                         state and dropped the spew",
                        last.total_chars()
                    )),
                    (false, _) => CheckOutcome::fail(
                        "compaction dropped the working state: the earliest, still-relevant \
                         result is gone from the prompt"
                            .to_string(),
                    ),
                    (true, true) => CheckOutcome::fail(
                        "compaction kept the working state but also kept the spew — the prompt \
                         shrank for some other reason"
                            .to_string(),
                    ),
                }
            },
        },
    ]
}

/// Look up one scenario by id.
pub fn scenario(id: &str) -> Option<Tier0Scenario> {
    scenarios().into_iter().find(|s| s.id == id)
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// One scenario's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tier0Result {
    pub id: String,
    pub measures: String,
    pub passed: bool,
    pub detail: String,
    /// How many times the harness called the model.
    pub model_requests: usize,
    /// Prompt size (characters) per request, in order — the context-growth
    /// curve, which is what makes a compaction regression legible.
    pub prompt_chars: Vec<usize>,
    pub duration_secs: f64,
    /// Set when the scenario could not run at all (spawn failure, timeout).
    pub error: Option<String>,
}

/// The whole Tier-0 run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tier0Report {
    pub schema: String,
    /// Always zero. Stated, not implied: this tier's whole claim is that it
    /// costs nothing, and a reader should be able to check that claim in the
    /// artifact rather than trust it.
    pub measured_cost_usd: f64,
    pub scenarios: Vec<Tier0Result>,
}

/// Schema id for a Tier-0 result file.
pub const TIER0_SCHEMA: &str = "permagent-eval/tier0-result@1";

impl Tier0Report {
    pub fn new(scenarios: Vec<Tier0Result>) -> Self {
        Tier0Report {
            schema: TIER0_SCHEMA.to_string(),
            measured_cost_usd: 0.0,
            scenarios,
        }
    }

    pub fn passed(&self) -> usize {
        self.scenarios.iter().filter(|s| s.passed).count()
    }

    pub fn all_passed(&self) -> bool {
        !self.scenarios.is_empty() && self.scenarios.iter().all(|s| s.passed)
    }
}

/// Render a Tier-0 report as Markdown.
pub fn render_markdown(r: &Tier0Report) -> String {
    let mut out = String::from("# Tier 0 — offline harness-loop metrics ($0)\n\n");
    out.push_str(&format!(
        "{}/{} scenario(s) passed · measured spend ${:.4} (mock model, loopback only)\n\n",
        r.passed(),
        r.scenarios.len(),
        r.measured_cost_usd
    ));
    out.push_str("| scenario | result | model calls | prompt chars | detail |\n");
    out.push_str("|---|---|---:|---|---|\n");
    for s in &r.scenarios {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            s.id,
            if s.passed { "PASS" } else { "FAIL" },
            s.model_requests,
            s.prompt_chars
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" → "),
            s.error.clone().unwrap_or_else(|| s.detail.clone())
        ));
    }
    out.push_str("\n## What each scenario measures\n\n");
    for s in &r.scenarios {
        out.push_str(&format!("- **`{}`** — {}\n", s.id, s.measures));
    }
    out
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// How long a single Tier-0 scenario may run before it is killed. Generous for
/// a local stub (no network), tight enough that a hung loop fails CI in
/// minutes rather than never.
pub const SCENARIO_TIMEOUT_SECS: u64 = 120;

/// Run one scenario end to end against the real `permagent` binary.
///
/// `work_root` gets a fresh workspace + data root per scenario, so nothing —
/// config, session db, provider registration — leaks between scenarios.
pub fn run_scenario(
    scenario: &Tier0Scenario,
    permagent_bin: &str,
    work_root: &Path,
) -> Result<Tier0Result> {
    let started = Instant::now();
    let dir = work_root.join(scenario.id);
    let workspace = dir.join("workspace");
    let data_root = dir.join("data");
    std::fs::create_dir_all(&workspace)?;
    std::fs::create_dir_all(&data_root)?;
    // Same reason as the real runner (`crate::runner::seed_git_repo`): the
    // harness's internal goal dispatch requires a git baseline commit to
    // branch a worker off of, and a bare scratch directory has none. This
    // tier's scripted scenarios don't dispatch goals today, but it shares the
    // real `permagent` binary and recipe path with the runner, so a scenario
    // that starts doing so must not immediately dead-end on a missing repo.
    crate::runner::seed_git_repo(&workspace, scenario.id)
        .with_context(|| format!("seeding git baseline for tier0 scenario {}", scenario.id))?;

    let server = StubServer::start((scenario.script)())?;
    install_provider(&data_root, &server.base_url())?;

    let recipe = dir.join("recipe.yaml");
    std::fs::write(&recipe, tier0_recipe(scenario.prompt))?;

    let log_path = dir.join("harness.log");
    let log = std::fs::File::create(&log_path)?;
    let err_log = log.try_clone()?;

    let recipe_arg = recipe.to_string_lossy().into_owned();
    let mut cmd = std::process::Command::new(permagent_bin);
    cmd.args([
        "run",
        "--recipe",
        recipe_arg.as_str(),
        "--provider",
        STUB_PROVIDER,
        "--model",
        STUB_MODEL,
        "--output-format",
        "text",
        // A runaway loop against an instant local stub would spin thousands of
        // turns before the wall-clock timeout noticed. No scenario here needs
        // more than four, so this bounds the damage without constraining the
        // behaviour being measured.
        "--max-turns",
        "12",
    ])
    .current_dir(&workspace)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::from(log))
    .stderr(std::process::Stdio::from(err_log))
    .env("PERMAGENT_PATH_ROOT", &data_root)
    .env("GOOSE_MODE", "auto")
    .env("PERMAGENT_DISABLE_KEYRING", "1")
    // A different provider for planning would be real network spend, and this
    // tier's entire promise is that there is none.
    .env_remove("GOOSE_PLANNER_PROVIDER")
    .env_remove("GOOSE_PLANNER_MODEL")
    .env_remove("OPENAI_HOST")
    .env_remove("OPENAI_BASE_PATH");
    for (k, v) in scenario.env {
        cmd.env(k, v);
    }

    let mut error = None;
    match cmd.spawn() {
        Ok(mut child) => {
            let deadline = Instant::now() + Duration::from_secs(SCENARIO_TIMEOUT_SECS);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        error = Some(format!(
                            "scenario timed out after {SCENARIO_TIMEOUT_SECS}s (log: {})",
                            log_path.display()
                        ));
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(e) => {
                        error = Some(format!("waiting for the harness failed: {e}"));
                        break;
                    }
                }
            }
        }
        Err(e) => error = Some(format!("spawning `{permagent_bin}` failed: {e}")),
    }

    let requests = server.stop();
    // The requests ARE the measurement, so keep them next to the scratch. A
    // failing check otherwise reports a verdict with no way to see what the
    // harness actually sent, which cost a whole diagnosis cycle: three
    // scenarios failed for one shared reason that was only visible here.
    if let Err(error) = dump_requests(&dir, &requests) {
        eprintln!("    (could not write requests.txt: {error})");
    }
    let outcome = if error.is_some() {
        CheckOutcome::fail("the scenario did not run to completion")
    } else {
        (scenario.check)(&requests)
    };

    Ok(Tier0Result {
        id: scenario.id.to_string(),
        measures: scenario.measures.to_string(),
        passed: outcome.passed && error.is_none(),
        detail: outcome.detail,
        model_requests: requests.len(),
        prompt_chars: requests.iter().map(RecordedRequest::total_chars).collect(),
        duration_secs: started.elapsed().as_secs_f64(),
        error,
    })
}

/// The recipe every Tier-0 scenario runs under.
///
/// The title deliberately does NOT match the coding-harness recipe title:
/// `is_coding_harness_recipe` gates repo-map injection on an exact title match,
/// and a repo map built from a scratch directory would add nondeterministic
/// content to the very prompts this tier is asserting over.
pub fn tier0_recipe(prompt: &str) -> String {
    let mut out = String::from(
        "title: Permagent Eval Tier 0\n\
         description: Scripted offline harness-loop metric\n\
         extensions:\n  \
         - type: builtin\n    \
           name: developer\n\
         instructions: |\n  \
           Follow the user's instruction using the tools available.\n\
         prompt: |-\n",
    );
    for line in prompt.trim_end_matches('\n').split('\n') {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Write every recorded request to `requests.txt` in the scenario's scratch
/// directory: one block per request, each message flattened, plus the delta a
/// turn added. This is what makes a failed check diagnosable instead of merely
/// true.
fn dump_requests(dir: &Path, requests: &[RecordedRequest]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut out = std::fs::File::create(dir.join("requests.txt"))?;
    writeln!(out, "{} request(s) recorded\n", requests.len())?;
    for (i, r) in requests.iter().enumerate() {
        writeln!(
            out,
            "=== request {i} — model={} messages={} chars={} tools={}",
            r.model,
            r.message_count,
            r.total_chars(),
            r.tool_names.join(",")
        )?;
        if i > 0 {
            let added = r.appended_since(&requests[i - 1]);
            writeln!(out, "--- {} message(s) this turn ADDED:", added.len())?;
            for m in added {
                writeln!(out, "  + {m}")?;
            }
        }
        writeln!(out, "--- all {} message(s):", r.message_texts.len())?;
        for (j, m) in r.message_texts.iter().enumerate() {
            writeln!(out, "  [{j}] {m}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}

/// Run every requested scenario and assemble the report.
pub fn run_all(
    selected: &[Tier0Scenario],
    permagent_bin: &str,
    work_root: &Path,
) -> Result<Tier0Report> {
    let mut results = Vec::new();
    for s in selected {
        eprintln!("--> tier0 {}", s.id);
        results.push(run_scenario(s, permagent_bin, work_root)?);
    }
    Ok(Tier0Report::new(results))
}

/// Group the recorded requests by model — a sanity check that nothing but the
/// stub was ever called. Any other model name in this map means real spend.
pub fn models_seen(requests: &[RecordedRequest]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for r in requests {
        *out.entry(r.model.clone()).or_insert(0) += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the scripted model's wire format ----------------------------------

    #[test]
    fn a_text_turn_renders_the_fields_the_openai_format_requires() {
        let v = StubTurn::text("hello").to_completion(STUB_MODEL, 7);
        assert_eq!(v["choices"][0]["message"]["role"], "assistant");
        assert_eq!(v["choices"][0]["message"]["content"], "hello");
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
        assert!(v["usage"]["prompt_tokens"].is_number());
    }

    #[test]
    fn a_tool_call_turn_carries_id_name_and_string_encoded_arguments() {
        let v = StubTurn::shell("c1", "ls").to_completion(STUB_MODEL, 7);
        let call = &v["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "c1");
        assert_eq!(call["function"]["name"], "shell");
        // Arguments must be a STRING containing JSON, not a JSON object — the
        // provider parses it itself.
        let args = call["function"]["arguments"]
            .as_str()
            .expect("arguments must be a string");
        assert_eq!(
            serde_json::from_str::<Value>(args).unwrap()["command"],
            "ls"
        );
        assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn the_malformed_turn_is_actually_malformed() {
        let v = StubTurn::malformed_shell("c1").to_completion(STUB_MODEL, 7);
        let args = v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(
            serde_json::from_str::<Value>(args).is_err(),
            "the scenario depends on these arguments failing to parse"
        );
    }

    // --- the stub server ----------------------------------------------------

    /// A minimal client, so the server is exercised over a real socket without
    /// pulling an HTTP crate into this dependency-light build.
    fn post(url_port: u16, body: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", url_port)).unwrap();
        write!(
            s,
            "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        s.flush().unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn the_stub_answers_the_script_in_order_and_records_every_request() {
        let server = StubServer::start(vec![
            StubTurn::shell("c1", "ls"),
            StubTurn::text("finished"),
        ])
        .unwrap();
        let port = server.port();

        let first = post(
            port,
            r#"{"model":"stub-model","messages":[{"role":"user","content":"hi"}],"tools":[{"function":{"name":"developer__shell"}}]}"#,
        );
        assert!(first.starts_with("HTTP/1.1 200 OK"), "{first}");
        assert!(first.contains("shell"));

        let second = post(
            port,
            r#"{"model":"stub-model","messages":[{"role":"user","content":"hi"},{"role":"tool","content":"out"}]}"#,
        );
        assert!(second.contains("finished"), "{second}");

        // Past the end of the script the stub says done rather than hanging.
        let third = post(port, r#"{"model":"stub-model","messages":[]}"#);
        assert!(third.contains("done"), "{third}");

        let reqs = server.stop();
        assert_eq!(reqs.len(), 3);
        assert_eq!(reqs[0].model, "stub-model");
        assert_eq!(reqs[0].message_count, 1);
        assert_eq!(reqs[0].tool_names, vec!["developer__shell".to_string()]);
        assert_eq!(reqs[1].message_count, 2);
        assert!(reqs[1].contains("out"));
        assert_eq!(models_seen(&reqs).get("stub-model"), Some(&3));
    }

    #[test]
    fn the_base_url_is_loopback_only() {
        let server = StubServer::start(vec![]).unwrap();
        let url = server.base_url();
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        assert!(url.ends_with("/v1/chat/completions"), "{url}");
    }

    // --- provider wiring ----------------------------------------------------

    #[test]
    fn the_provider_declaration_needs_no_key_and_no_streaming() {
        let v: Value =
            serde_json::from_str(&provider_json("http://127.0.0.1:9/v1/chat/completions")).unwrap();
        assert_eq!(v["name"], STUB_PROVIDER);
        assert_eq!(v["engine"], "openai");
        assert_eq!(v["requires_auth"], false);
        assert_eq!(v["supports_streaming"], false);
        assert_eq!(v["api_key_env"], "");
        assert_eq!(v["models"][0]["name"], STUB_MODEL);
        // The model name must not look like an OpenAI model that routes to the
        // responses API — that would take the harness off the measured path.
        let name = v["models"][0]["name"].as_str().unwrap();
        assert!(!name.starts_with("gpt-5"), "{name}");
    }

    #[test]
    fn installing_the_provider_writes_it_where_the_harness_looks() {
        let tmp = tempfile::tempdir().unwrap();
        install_provider(tmp.path(), "http://127.0.0.1:1234/v1/chat/completions").unwrap();
        let path = tmp
            .path()
            .join("custom_providers")
            .join(format!("{STUB_PROVIDER}.json"));
        assert!(path.exists());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["base_url"], "http://127.0.0.1:1234/v1/chat/completions");
    }

    #[test]
    fn the_tier0_recipe_embeds_the_prompt_and_avoids_the_coding_harness_title() {
        let r = tier0_recipe("do the thing");
        assert!(r.contains("prompt: |-\n  do the thing\n"), "{r}");
        // The repo-map gate matches the coding-harness recipe title exactly;
        // a Tier-0 prompt must not accidentally opt into nondeterminism.
        assert!(!r.contains("Permagent Coding"), "{r}");
        // …and it must still be loadable YAML.
        let v: serde_yaml::Value = serde_yaml::from_str(&r).unwrap();
        assert_eq!(v["prompt"].as_str().unwrap(), "do the thing");
    }

    // --- the checks, over synthetic request logs ----------------------------

    fn req(texts: &[&str]) -> RecordedRequest {
        RecordedRequest {
            model: STUB_MODEL.to_string(),
            message_count: texts.len(),
            message_texts: texts.iter().map(|s| s.to_string()).collect(),
            tool_names: vec![],
        }
    }

    fn check(id: &str, reqs: &[RecordedRequest]) -> CheckOutcome {
        (scenario(id).expect("scenario exists").check)(reqs)
    }

    #[test]
    fn retry_check_fails_when_the_loop_gave_up_and_passes_when_it_recovered() {
        let gave_up = check("retry-on-malformed-tool-call", &[req(&["hi"])]);
        assert!(!gave_up.passed);
        assert!(gave_up.detail.contains("stopped after 1"), "{gave_up:?}");

        let blind = check(
            "retry-on-malformed-tool-call",
            &[req(&["hi"]), req(&["hi", "try again"])],
        );
        assert!(!blind.passed, "a blind retry is not a repair");

        let repaired = check(
            "retry-on-malformed-tool-call",
            &[
                req(&["hi"]),
                req(&[
                    "hi",
                    "Your last tool call could not be parsed and did not run.",
                ]),
            ],
        );
        assert!(repaired.passed, "{repaired:?}");
    }

    /// The system prompt is in every request and is full of words like "error".
    /// A check that searched the whole prompt would pass no matter what the
    /// harness did — so only the messages the turn ADDED may count.
    #[test]
    fn retry_check_ignores_error_words_that_were_already_in_the_prompt() {
        let system = "If a tool call fails with an error, explain it and retry.";
        let vacuous = check(
            "retry-on-malformed-tool-call",
            &[req(&[system]), req(&[system, "ok, continuing"])],
        );
        assert!(
            !vacuous.passed,
            "a word from the system prompt must not count as evidence: {vacuous:?}"
        );
    }

    /// Regression: a request with no `messages` is not a chat completion. It
    /// must be answered WITHOUT recording it and WITHOUT advancing the script.
    /// When it did both, the provider probe ate the scripted tool call, so no
    /// tool ever ran and every check's "follow-up" was really the first prompt —
    /// three scenarios failed for one invisible reason.
    #[test]
    fn a_non_completion_request_neither_records_nor_advances_the_script() {
        let server = StubServer::start(vec![
            StubTurn::shell("c1", "ls"),
            StubTurn::text("finished"),
        ])
        .unwrap();
        let port = server.port();

        // A probe: no `messages` key at all.
        let probe = post(port, r#"{"model":"stub-model"}"#);
        assert!(probe.starts_with("HTTP/1.1 200 OK"), "{probe}");
        assert!(
            !probe.contains("tool_calls"),
            "a probe must not be handed the scripted tool call: {probe}"
        );

        // The first REAL completion must still receive script turn 1.
        let first = post(
            port,
            r#"{"model":"stub-model","messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert!(
            first.contains("tool_calls"),
            "the first completion must get the scripted tool call: {first}"
        );

        let reqs = server.stop();
        assert_eq!(
            reqs.len(),
            1,
            "only the completion is a measurement; the probe must not be recorded"
        );
    }

    /// Regression: reported usage must track the prompt. The harness trusts a
    /// provider's total over its own estimate, so a constant `total_tokens`
    /// silently tells it the context is empty and auto-compaction can never
    /// trigger however large the conversation grows.
    #[test]
    fn reported_usage_tracks_the_prompt_instead_of_being_a_constant() {
        let small = StubTurn::text("ok").to_completion(STUB_MODEL, 10);
        let large = StubTurn::text("ok").to_completion(STUB_MODEL, 9_000);
        let small_total = small["usage"]["total_tokens"].as_u64().unwrap();
        let large_total = large["usage"]["total_tokens"].as_u64().unwrap();
        assert!(
            large_total > small_total * 10,
            "usage must move with the prompt: {small_total} vs {large_total}"
        );
        assert!(small_total > 0, "usage must still be present");
    }

    /// Regression: no complete sentinel may appear in the command. The
    /// assistant's tool call is echoed into the next prompt, so a command
    /// containing the sentinels put them in the prompt regardless of what the
    /// tool returned — the pruning check was matching its own input and could
    /// not tell surviving output from a mentioned command.
    #[test]
    fn the_huge_output_command_never_contains_a_whole_sentinel() {
        let cmd = huge_output_command();
        for sentinel in [HEAD_SENTINEL, MID_SENTINEL, TAIL_SENTINEL] {
            assert!(
                !cmd.contains(sentinel),
                "command leaks {sentinel}, so the check would match its own input: {cmd}"
            );
        }
    }

    /// Regression: feedback the harness delivers by AMENDING an existing message
    /// must still count. The malformed-tool-call guidance is rewritten into the
    /// per-turn status block rather than appended, so an append-only diff
    /// reported "no sign the call failed" about a prompt that carried a precise
    /// parse error.
    #[test]
    fn gained_signal_sees_feedback_amended_into_an_existing_message() {
        let before = req(&["system", "ask", "status: idle"]);
        let after = req(&["system", "ask", "status: could not be parsed, nothing ran"]);
        assert_eq!(
            after.appended_since(&before).len(),
            0,
            "the message count is unchanged — that is the whole trap"
        );
        assert!(gained_signal(&before, &after, &["could not be parsed"]));
        // And it must not fire on wording that was already there.
        assert!(!gained_signal(&before, &after, &["status"]));
    }

    #[test]
    fn appended_since_returns_only_the_new_messages_and_nothing_when_compacted() {
        let a = req(&["one", "two"]);
        let b = req(&["one", "two", "three"]);
        assert_eq!(b.appended_since(&a), ["three".to_string()].as_slice());
        // A compacted prompt is shorter than its predecessor: no evidence,
        // rather than a panic or a wrong slice.
        assert!(a.appended_since(&b).is_empty());
    }

    #[test]
    fn pruning_check_distinguishes_truncated_from_verbatim_from_dropped() {
        let truncated_ok = format!("{TAIL_SENTINEL} .. Output exceeded 2000 line limit");
        let good = check(
            "tool-result-pruning",
            &[req(&["go"]), req(&["go", &truncated_ok])],
        );
        assert!(good.passed, "{good:?}");

        // The middle surviving means it was not reduced to a preview.
        let verbatim = check(
            "tool-result-pruning",
            &[
                req(&["go"]),
                req(&["go", &format!("{MID_SENTINEL} {truncated_ok}")]),
            ],
        );
        assert!(!verbatim.passed);
        assert!(verbatim.detail.contains("middle"), "{verbatim:?}");

        // Losing the END loses the part that says whether the command worked.
        let no_tail = check(
            "tool-result-pruning",
            &[
                req(&["go"]),
                req(&["go", "exceeded the limit, saved elsewhere"]),
            ],
        );
        assert!(!no_tail.passed);
        assert!(no_tail.detail.contains("END"), "{no_tail:?}");

        // Truncating silently leaves the model unable to know output is missing.
        let untold = check(
            "tool-result-pruning",
            &[req(&["go"]), req(&["go", TAIL_SENTINEL])],
        );
        assert!(!untold.passed);
        assert!(untold.detail.contains("not told"), "{untold:?}");

        // An unbounded result is a failure whatever else survived.
        let huge = "x".repeat(50_000);
        let unbounded = check(
            "tool-result-pruning",
            &[
                req(&["go"]),
                req(&["go", &format!("{TAIL_SENTINEL}{huge}")]),
            ],
        );
        assert!(!unbounded.passed);
        assert!(unbounded.detail.contains("not bounded"), "{unbounded:?}");
    }

    #[test]
    fn the_premature_done_pair_only_passes_when_the_two_arms_differ() {
        // Both arms now edit first, so the shapes are: ask -> write, then the
        // unverified 'done'. Guard ON holds and produces a 3rd request; guard
        // OFF stops at 2. Anything else makes the pair meaningless.
        assert!(
            check(
                "premature-done-guard-on",
                &[req(&["a"]), req(&["b"]), req(&["c"])]
            )
            .passed
        );
        assert!(!check("premature-done-guard-on", &[req(&["a"]), req(&["b"])]).passed);
        assert!(check("premature-done-guard-off", &[req(&["a"]), req(&["b"])]).passed);
        let leaky = check(
            "premature-done-guard-off",
            &[req(&["a"]), req(&["b"]), req(&["c"])],
        );
        assert!(!leaky.passed);
        assert!(
            leaky.detail.contains("cannot be attributed"),
            "the control must explain why the pair is now meaningless: {leaky:?}"
        );
    }

    #[test]
    fn compaction_check_requires_a_shrink_that_keeps_state_and_drops_spew() {
        let big = "x".repeat(5000);
        let grew_only = check(
            "compaction-preserve-list",
            &[
                req(&[KEEP_SENTINEL]),
                req(&[KEEP_SENTINEL, &big]),
                req(&[KEEP_SENTINEL, &big, &big]),
            ],
        );
        assert!(!grew_only.passed);
        assert!(grew_only.detail.contains("never shrank"), "{grew_only:?}");

        let good = check(
            "compaction-preserve-list",
            &[
                req(&[KEEP_SENTINEL]),
                req(&[KEEP_SENTINEL, &big, MID_SENTINEL]),
                req(&[KEEP_SENTINEL, "summary"]),
            ],
        );
        assert!(good.passed, "{good:?}");

        let forgot = check(
            "compaction-preserve-list",
            &[
                req(&[KEEP_SENTINEL]),
                req(&[KEEP_SENTINEL, &big, MID_SENTINEL]),
                req(&["summary"]),
            ],
        );
        assert!(!forgot.passed);
        assert!(
            forgot.detail.contains("dropped the working state"),
            "{forgot:?}"
        );

        let too_short = check("compaction-preserve-list", &[req(&["a"]), req(&["b"])]);
        assert!(!too_short.passed);
        assert!(
            too_short.detail.contains("never got far enough"),
            "{too_short:?}"
        );
    }

    // --- report -------------------------------------------------------------

    fn result(id: &str, passed: bool) -> Tier0Result {
        Tier0Result {
            id: id.to_string(),
            measures: "m".to_string(),
            passed,
            detail: "d".to_string(),
            model_requests: 2,
            prompt_chars: vec![10, 20],
            duration_secs: 1.0,
            error: None,
        }
    }

    #[test]
    fn the_report_states_its_zero_cost_and_round_trips() {
        let r = Tier0Report::new(vec![result("a", true), result("b", false)]);
        assert_eq!(r.measured_cost_usd, 0.0);
        assert_eq!(r.passed(), 1);
        assert!(!r.all_passed());
        let back: Tier0Report = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.schema, TIER0_SCHEMA);

        let md = render_markdown(&r);
        assert!(md.contains("$0.0000"), "{md}");
        assert!(md.contains("PASS") && md.contains("FAIL"), "{md}");
        assert!(
            md.contains("10 → 20"),
            "the context curve must be visible:\n{md}"
        );
    }

    #[test]
    fn an_empty_report_is_not_a_green_one() {
        assert!(
            !Tier0Report::new(vec![]).all_passed(),
            "zero scenarios must never read as success"
        );
    }

    #[test]
    fn every_scenario_has_a_unique_id_a_rationale_and_a_script() {
        let all = scenarios();
        assert!(all.len() >= 5);
        let mut ids: Vec<&str> = all.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "scenario ids must be unique");
        for s in &all {
            assert!(!s.measures.is_empty(), "{}", s.id);
            assert!(!(s.script)().is_empty(), "{}", s.id);
            assert!(scenario(s.id).is_some());
        }
        assert!(scenario("nope").is_none());
    }

    /// End-to-end against the real binary. Ignored by default because it needs
    /// `permagent` built and on PATH; it spends nothing (loopback stub only).
    ///
    /// CI runs it as:
    ///   cargo build -p permagent && \
    ///   PERMAGENT_EVAL_BIN=target/debug/permagent \
    ///     cargo test -p permagent-eval --lib -- --ignored tier0_end_to_end
    #[test]
    #[ignore = "needs the permagent binary; run with --ignored in CI after building it"]
    fn tier0_end_to_end() {
        let bin = std::env::var("PERMAGENT_EVAL_BIN").unwrap_or_else(|_| "permagent".to_string());
        let tmp = tempfile::tempdir().unwrap();
        let report = run_all(&scenarios(), &bin, tmp.path()).unwrap();
        eprintln!("{}", render_markdown(&report));
        assert_eq!(report.measured_cost_usd, 0.0);
        for s in &report.scenarios {
            assert!(s.error.is_none(), "{}: {:?}", s.id, s.error);
            assert!(s.passed, "{}: {}", s.id, s.detail);
        }
    }
}
