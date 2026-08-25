//! Voice-model bench — which model should answer a VOICE turn?
//!
//! A voice turn is not a coding turn. It carries the same ~70k-token prompt (the
//! system prompt plus ~124 tool schemas) but it wants ONE thing the chat path does
//! not: the first spoken syllable, fast. `docs/research/VOICE_LATENCY_AND_ORB_2026-08-25.md`
//! measured the current path at a 7.4 s median TTFT — 73 % of a 10.6 s
//! speech-end→first-audio. This bench decides the replacement by measurement.
//!
//! It drives each candidate through the SAME provider stack the daemon uses
//! (`permagent::providers::create` → `Provider::stream_split`), so the
//! `cache_control` layout, the tool formatting and the streaming decode are the
//! daemon's, not a reimplementation.
//!
//! What it measures per turn:
//!   * TTFT — first *spoken* text delta (thinking blocks do not count; TTS cannot
//!     speak them)
//!   * time to the first speakable boundary — what `enqueue_ready_sentences` in
//!     `routes::voice` actually keys the first audio on, via the very same
//!     [`permagent_daemon::routes::voice::find_speakable_boundary`]
//!   * total stream latency, input/output/cache tokens, prompt-cache hit rate and
//!     cost from the canonical pricing table (fail-closed: an unpriced model is
//!     reported as UNKNOWN, never as free)
//!   * whether the expected tool was called, on the five turns that need one
//!
//! Inputs (nothing personal is read or written):
//!   * `--turns`  the synthetic turn corpus, `crates/goose-server/tests/fixtures/voice_bench_turns.json`
//!   * `--system` the repo's own prompt-manager snapshot, which is the real
//!     ~110k-char stable prefix with no user data in it
//!   * `--tools`  the 124 tool schemas, extracted from a daemon request log by
//!     `scripts/bench/extract_voice_tools.py` (schemas only — see that script)
//!
//! Usage:
//!   cargo run --release -p permagent-daemon --bin voice_model_bench -- --probe
//!   cargo run --release -p permagent-daemon --bin voice_model_bench -- --dry-run
//!   cargo run --release -p permagent-daemon --bin voice_model_bench -- --run --out results.json

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use permagent::conversation::message::{Message, MessageContent};
use permagent::cost_router::cache::SystemPromptParts;
use permagent::model::ModelConfig;
use permagent::providers::base::{Provider, Usage};
use permagent::providers::canonical::{cache_hit_rate_of, cost_of, maybe_get_canonical_model};
use permagent_daemon::routes::voice::find_speakable_boundary;
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Providers the bench knows how to reach, and the secret each needs. Presence is
/// checked through the repo's own config reader so no secret is ever printed.
const KNOWN_PROVIDERS: &[(&str, &str)] = &[
    ("minimax", "MINIMAX_API_KEY"),
    ("zai", "ZAI_API_KEY"),
    ("zhipu", "ZHIPU_API_KEY"),
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("openai", "OPENAI_API_KEY"),
    ("google", "GOOGLE_API_KEY"),
    ("deepseek", "DEEPSEEK_API_KEY"),
    ("moonshot", "MOONSHOT_API_KEY"),
    ("groq", "GROQ_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
];

/// The default candidate slate: `label=provider/model`.
const DEFAULT_CANDIDATES: &[&str] = &[
    "baseline=minimax/MiniMax-M2.7",
    "highspeed=minimax/MiniMax-M2.7-highspeed",
    "flashx=zai/glm-4.7-flashx",
    "haiku=anthropic/claude-haiku-4-5-20251001",
];

/// Hard ceiling on bench spend. The run aborts before the request that would
/// cross it — a bench that quietly outspends its mandate is a failed bench.
const DEFAULT_BUDGET_USD: f64 = 5.0;

/// A voice reply is short by construction. Capping output keeps the bench honest
/// (a model cannot win on latency by truncating) and bounds the output spend.
const MAX_OUTPUT_TOKENS: i32 = 1024;

// ── Corpus ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Corpus {
    turns: Vec<Turn>,
    volatile_system_block: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Turn {
    id: String,
    kind: String,
    prompt: String,
    expect_tool: Option<String>,
    rubric: String,
}

// ── Results ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TurnResult {
    turn_id: String,
    kind: String,
    /// What a good SPOKEN answer must do — handed to the judge verbatim.
    rubric: String,
    /// Milliseconds to the first *spoken* text delta.
    ttft_ms: Option<u128>,
    /// Milliseconds to the first speakable boundary — the first-audio trigger.
    first_boundary_ms: Option<u128>,
    total_ms: u128,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    cache_read_tokens: Option<i32>,
    cache_write_tokens: Option<i32>,
    cache_hit_rate: Option<f64>,
    /// `None` when the model has no canonical price — treated as UNKNOWN, never 0.
    cost_usd: Option<f64>,
    expected_tool: Option<String>,
    called_tools: Vec<String>,
    tool_correct: Option<bool>,
    /// The spoken text, kept so a judge can score it. Synthetic prompts only.
    reply_text: String,
    /// Whether the model emitted a thinking block before speaking.
    thought: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RunResult {
    candidate: String,
    provider: String,
    model: String,
    /// 0 = cold cache (first pass), 1 = warm cache (second pass).
    run_index: usize,
    turns: Vec<TurnResult>,
}

// ── CLI ──────────────────────────────────────────────────────────────────────

struct Args {
    probe: bool,
    dry_run: bool,
    run: bool,
    candidates: Vec<String>,
    runs: usize,
    limit: Option<usize>,
    turns_path: String,
    system_path: String,
    tools_path: String,
    out: Option<String>,
    budget_usd: f64,
    /// Re-judge a previously written results file instead of spending on new
    /// requests. Latency and cost come from the saved run; only the judge is new.
    judge_only: Option<String>,
    /// `provider/model` of the quality judge. Must be a DIFFERENT family than
    /// the candidates it scores — a model grading its own family's prose is not
    /// an independent verdict.
    judge: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        probe: false,
        dry_run: false,
        run: false,
        candidates: DEFAULT_CANDIDATES.iter().map(|s| s.to_string()).collect(),
        runs: 2,
        limit: None,
        turns_path: "crates/goose-server/tests/fixtures/voice_bench_turns.json".to_string(),
        system_path:
            "crates/goose/src/agents/snapshots/permagent__agents__prompt_manager__tests__all_platform_extensions.snap"
                .to_string(),
        tools_path: "/tmp/voice_bench_tools.json".to_string(),
        out: None,
        budget_usd: DEFAULT_BUDGET_USD,
        judge: None,
        judge_only: None,
    };

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let next = |i: &mut usize| -> Result<String> {
            *i += 1;
            argv.get(*i)
                .cloned()
                .ok_or_else(|| anyhow!("missing value for {}", argv[*i - 1]))
        };
        match argv[i].as_str() {
            "--probe" => args.probe = true,
            "--dry-run" => args.dry_run = true,
            "--run" => args.run = true,
            "--candidates" => {
                args.candidates = next(&mut i)?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            "--runs" => args.runs = next(&mut i)?.parse()?,
            "--limit" => args.limit = Some(next(&mut i)?.parse()?),
            "--turns" => args.turns_path = next(&mut i)?,
            "--system" => args.system_path = next(&mut i)?,
            "--tools" => args.tools_path = next(&mut i)?,
            "--out" => args.out = Some(next(&mut i)?),
            "--judge" => args.judge = Some(next(&mut i)?),
            "--judge-only" => args.judge_only = Some(next(&mut i)?),
            "--budget-usd" => args.budget_usd = next(&mut i)?.parse()?,
            other => return Err(anyhow!("unknown flag {other}")),
        }
        i += 1;
    }
    Ok(args)
}

// ── Inputs ───────────────────────────────────────────────────────────────────

/// An insta snapshot is a small YAML header, a `---` line, then the snapshot
/// body. The body here IS the real system prompt the daemon renders with every
/// platform extension loaded — generated from repo templates, so it carries the
/// true size and shape with none of a live session's personal context.
fn read_snapshot_body(path: &str) -> Result<String> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let mut parts = raw.splitn(3, "---\n");
    // ["", header, body]
    parts.next();
    parts
        .next()
        .ok_or_else(|| anyhow!("{path}: no snapshot header"))?;
    let body = parts
        .next()
        .ok_or_else(|| anyhow!("{path}: no snapshot body"))?;
    Ok(body.to_string())
}

fn read_tools(path: &str) -> Result<Vec<Tool>> {
    let raw = std::fs::read_to_string(path).with_context(|| {
        format!(
            "reading {path} — generate it first with \
             `python3 scripts/bench/extract_voice_tools.py <daemon llm_request jsonl> {path}`"
        )
    })?;
    let tools: Vec<Tool> = serde_json::from_str(&raw).with_context(|| format!("parsing {path}"))?;
    Ok(tools)
}

// ── The measured turn ────────────────────────────────────────────────────────

/// Stream one turn and time it the way the voice path experiences it.
async fn run_turn(
    provider: &Arc<dyn Provider>,
    model_config: &ModelConfig,
    provider_name: &str,
    session_id: &str,
    system: &SystemPromptParts,
    tools: &[Tool],
    turn: &Turn,
) -> TurnResult {
    let started = Instant::now();
    let user = Message::user().with_text(&turn.prompt);

    let mut result = TurnResult {
        turn_id: turn.id.clone(),
        kind: turn.kind.clone(),
        rubric: turn.rubric.clone(),
        ttft_ms: None,
        first_boundary_ms: None,
        total_ms: 0,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        cache_hit_rate: None,
        cost_usd: None,
        expected_tool: turn.expect_tool.clone(),
        called_tools: Vec::new(),
        tool_correct: None,
        reply_text: String::new(),
        thought: false,
        error: None,
    };

    let stream = provider
        .stream_split(
            model_config,
            session_id,
            system,
            std::slice::from_ref(&user),
            tools,
        )
        .await;

    let mut stream = match stream {
        Ok(s) => s,
        Err(e) => {
            result.error = Some(e.to_string());
            result.total_ms = started.elapsed().as_millis();
            return result;
        }
    };

    // `find_speakable_boundary` consumes a buffer; mirror the voice path's
    // "first chunk speaks sooner" rule by passing `first_chunk = true` until the
    // first boundary is found.
    let mut spoken_buf = String::new();
    let mut usage_seen: Option<Usage> = None;

    while let Some(item) = stream.next().await {
        match item {
            Ok((message, usage)) => {
                if let Some(u) = usage {
                    usage_seen = Some(u.usage);
                }
                let Some(message) = message else { continue };
                for content in &message.content {
                    match content {
                        MessageContent::Thinking(_) | MessageContent::RedactedThinking(_) => {
                            result.thought = true;
                        }
                        MessageContent::Text(t) if !t.text.is_empty() => {
                            if result.ttft_ms.is_none() {
                                result.ttft_ms = Some(started.elapsed().as_millis());
                            }
                            result.reply_text.push_str(&t.text);
                            if result.first_boundary_ms.is_none() {
                                spoken_buf.push_str(&t.text);
                                if find_speakable_boundary(&spoken_buf, true).is_some() {
                                    result.first_boundary_ms = Some(started.elapsed().as_millis());
                                }
                            }
                        }
                        MessageContent::ToolRequest(req) => {
                            if let Ok(call) = &req.tool_call {
                                result.called_tools.push(call.name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                result.error = Some(e.to_string());
                break;
            }
        }
    }

    result.total_ms = started.elapsed().as_millis();

    // A reply that never crosses a boundary is still spoken — the voice path
    // flushes the remainder at end of stream. Record that as the boundary time so
    // a terse one-word answer is not scored as "never spoke".
    if result.first_boundary_ms.is_none() && !result.reply_text.trim().is_empty() {
        result.first_boundary_ms = Some(result.total_ms);
    }

    if let Some(expected) = &turn.expect_tool {
        result.tool_correct = Some(result.called_tools.iter().any(|t| t == expected));
    }

    if let Some(usage) = usage_seen {
        result.input_tokens = usage.input_tokens;
        result.output_tokens = usage.output_tokens;
        result.cache_read_tokens = usage.cache_read_input_tokens;
        result.cache_write_tokens = usage.cache_write_input_tokens;
        match maybe_get_canonical_model(provider_name, &model_config.model_name) {
            Some(canonical) => {
                result.cache_hit_rate = cache_hit_rate_of(&usage);
                result.cost_usd = cost_of(&usage, &canonical.cost);
            }
            None => {
                result.cache_hit_rate = cache_hit_rate_of(&usage);
                result.cost_usd = None;
            }
        }
    }

    result
}

// ── Modes ────────────────────────────────────────────────────────────────────

fn probe() {
    let config = permagent::config::Config::global();
    println!("provider        secret            configured");
    println!("--------------- ----------------- ----------");
    for (provider, secret_key) in KNOWN_PROVIDERS {
        let present = config.get_secret::<String>(secret_key).is_ok();
        println!(
            "{provider:<15} {secret_key:<17} {}",
            if present { "yes" } else { "no" }
        );
    }
    println!("\n(presence only — no secret value is read into the report)");
}

/// Pre-flight spend estimate for one candidate over `turns × runs` requests.
///
/// It models the shape this bench actually has: every turn sends the SAME ~65k
/// prefix (system prompt + 124 tool schemas), so the first turn of a run pays the
/// cache-write premium and the rest are cache reads. A provider with no separate
/// cache-read rate is charged full fresh input on every turn — for those there is
/// no credit to assume. Output is charged at the cap on every turn.
///
/// This is an ESTIMATE, and it is not what enforces the budget: the run also
/// checks MEASURED spend before every request and stops there. So if caching
/// silently fails, the hard stop catches it rather than this arithmetic.
/// `None` means the model has no canonical price — the caller must refuse to
/// spend rather than assume free.
fn projected_cost(
    provider: &str,
    model: &str,
    approx_input_tokens: i32,
    turns: usize,
    runs: usize,
) -> Option<f64> {
    let pricing = maybe_get_canonical_model(provider, model)?.cost;
    let input_rate = pricing.input?;
    let output_rate = pricing.output?;

    let per_million = |tokens: i32, rate: f64| (tokens as f64 / 1_000_000.0) * rate;
    let turns_f = turns as f64;

    let input_cost_per_run = match pricing.cache_read {
        Some(read_rate) => {
            let write_rate = pricing.cache_write.unwrap_or(input_rate);
            per_million(approx_input_tokens, write_rate)
                + (turns_f - 1.0) * per_million(approx_input_tokens, read_rate)
        }
        None => turns_f * per_million(approx_input_tokens, input_rate),
    };
    let output_cost_per_run = turns_f * per_million(MAX_OUTPUT_TOKENS, output_rate);

    Some((input_cost_per_run + output_cost_per_run) * runs as f64)
}

fn parse_candidate(spec: &str) -> Result<(String, String, String)> {
    let (label, target) = spec
        .split_once('=')
        .ok_or_else(|| anyhow!("candidate '{spec}' must look like label=provider/model"))?;
    let (provider, model) = target
        .split_once('/')
        .ok_or_else(|| anyhow!("candidate '{spec}' must look like label=provider/model"))?;
    Ok((label.to_string(), provider.to_string(), model.to_string()))
}

/// Create a provider, retrying once. See the call site: a cold system keyring can
/// time out on its first read and report a present key as missing.
async fn create_provider_with_retry(
    provider_name: &str,
    model_config: &ModelConfig,
    label: &str,
    model_name: &str,
) -> Option<Arc<dyn Provider>> {
    for attempt in 0..2 {
        match permagent::providers::create(provider_name, model_config.clone(), Vec::new()).await {
            Ok(p) => return Some(p),
            Err(e) if attempt == 0 => {
                eprintln!("  retrying {label} ({provider_name}/{model_name}): {e}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                eprintln!("SKIP {label} ({provider_name}/{model_name}): {e}");
                return None;
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;

    if args.probe {
        probe();
        return Ok(());
    }

    let corpus: Corpus = serde_json::from_str(
        &std::fs::read_to_string(&args.turns_path)
            .with_context(|| format!("reading {}", args.turns_path))?,
    )?;
    let mut turns = corpus.turns;
    if let Some(limit) = args.limit {
        turns.truncate(limit);
    }

    // Re-judging reads the answers a previous run already paid for. Latency and
    // cost in the reprinted table are that run's; only the quality column is new.
    if let Some(path) = &args.judge_only {
        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
        )?;
        let runs: Vec<RunResult> =
            serde_json::from_value(saved.get("runs").cloned().unwrap_or_else(|| saved.clone()))?;
        let judge_spec = args
            .judge
            .as_ref()
            .ok_or_else(|| anyhow!("--judge-only also needs --judge provider/model"))?;
        let scores = judge_replies(judge_spec, &turns, &runs).await?;
        summarize(&runs, &scores);
        if let Some(out) = &args.out {
            let payload = serde_json::json!({ "runs": &runs, "quality": &scores });
            std::fs::write(out, serde_json::to_string_pretty(&payload)?)?;
            println!("wrote {out}");
        }
        return Ok(());
    }

    let stable = read_snapshot_body(&args.system_path)?;
    let system = SystemPromptParts::new(stable.clone(), corpus.volatile_system_block.clone());
    let tools = read_tools(&args.tools_path)?;

    // ~3.6 chars/token is the measured ratio for this prompt shape (the recorded
    // 274k-char payload reported ~70k input tokens). Used only for the budget
    // gate, never for reporting.
    let approx_input_tokens =
        ((system.render().len() + serde_json::to_string(&tools)?.len()) / 4) as i32;

    println!(
        "corpus: {} turns  |  system: {} chars stable + {} chars volatile  |  tools: {} schemas  |  ~{}k input tokens/turn",
        turns.len(),
        stable.len(),
        corpus.volatile_system_block.len(),
        tools.len(),
        approx_input_tokens / 1000,
    );

    let mut candidates = Vec::new();
    let mut projected_total = 0.0;
    for spec in &args.candidates {
        let (label, provider, model) = parse_candidate(spec)?;
        let projected = projected_cost(
            &provider,
            &model,
            approx_input_tokens,
            turns.len(),
            args.runs,
        );
        match projected {
            Some(p) => projected_total += p,
            None => {
                return Err(anyhow!(
                    "{provider}/{model} has no canonical price — refusing to spend on a model \
                     whose cost cannot be computed (fail closed)"
                ))
            }
        }
        println!(
            "  {label:<12} {provider}/{model:<32} est. ${:.2} over {} runs",
            projected.unwrap_or(0.0),
            args.runs
        );
        candidates.push((label, provider, model));
    }
    println!(
        "projected total: ${projected_total:.2} (budget ${:.2})",
        args.budget_usd
    );

    if projected_total > args.budget_usd {
        return Err(anyhow!(
            "projected spend ${projected_total:.2} exceeds the ${:.2} budget — \
             narrow --candidates, --runs or --limit",
            args.budget_usd
        ));
    }

    if !args.run {
        println!("\n(dry run — pass --run to spend)");
        return Ok(());
    }

    let mut all: Vec<RunResult> = Vec::new();
    let mut spent = 0.0_f64;
    let mut budget_exhausted = false;

    for (label, provider_name, model_name) in &candidates {
        let model_config = ModelConfig::new(model_name)?
            .with_canonical_limits(provider_name)
            .with_max_tokens(Some(MAX_OUTPUT_TOKENS));

        // The first read of the system keyring in a process can exceed the config
        // layer's own deadline and surface as a spurious "missing API key". Retry
        // once — a real absence fails the same way twice.
        let provider =
            match create_provider_with_retry(provider_name, &model_config, label, model_name).await
            {
                Some(p) => p,
                None => continue,
            };

        for run_index in 0..args.runs {
            // A fresh session id per (candidate, run) keeps the runs independent;
            // run 0 is the cold-cache pass and run 1 the warm one, because the
            // 5-minute prefix cache is keyed on the prompt prefix, not the id.
            let session_id = format!("vbench-{label}-{run_index}");
            let mut run = RunResult {
                candidate: label.clone(),
                provider: provider_name.clone(),
                model: model_name.clone(),
                run_index,
                turns: Vec::new(),
            };

            for turn in &turns {
                if spent > args.budget_usd {
                    eprintln!(
                        "BUDGET STOP: ${spent:.2} measured spend has reached the ${:.2} \
                         budget — halting before {}",
                        args.budget_usd, turn.id
                    );
                    budget_exhausted = true;
                    break;
                }
                let result = run_turn(
                    &provider,
                    &model_config,
                    provider_name,
                    &session_id,
                    &system,
                    &tools,
                    turn,
                )
                .await;
                spent += result.cost_usd.unwrap_or(0.0);
                println!(
                    "  {label} run{run_index} {:<4} ttft={:>6} boundary={:>6} total={:>6} tools={:?} cache_hit={:?} ${:.4} {}",
                    result.turn_id,
                    result
                        .ttft_ms
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                    result
                        .first_boundary_ms
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                    result.total_ms,
                    result.called_tools,
                    result.cache_hit_rate.map(|r| (r * 100.0).round() as i32),
                    result.cost_usd.unwrap_or(0.0),
                    result.error.as_deref().unwrap_or(""),
                );
                run.turns.push(result);
                // A short gap between turns keeps us off provider rate limits
                // without meaningfully changing cache warmth (TTL is 5 minutes).
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            all.push(run);
            if budget_exhausted {
                break;
            }
        }
        if budget_exhausted {
            break;
        }
    }

    println!("\ntotal measured spend: ${spent:.4}");

    let scores = match &args.judge {
        Some(spec) => match judge_replies(spec, &turns, &all).await {
            Ok(scores) => scores,
            Err(e) => {
                eprintln!("judging failed ({e}) — reporting latency and cost only");
                BTreeMap::new()
            }
        },
        None => BTreeMap::new(),
    };

    summarize(&all, &scores);

    if let Some(out) = &args.out {
        let payload = serde_json::json!({ "runs": &all, "quality": &scores });
        std::fs::write(out, serde_json::to_string_pretty(&payload)?)?;
        println!("wrote {out}");
    }
    Ok(())
}

// ── Quality: a cross-family judge ────────────────────────────────────────────

/// The judge's mandate. It scores SPOKEN answers, so the rubric it applies is
/// about being *heard*, not about being read: brevity, directness, no markdown,
/// no preamble. Answers arrive anonymised and shuffled, so the judge cannot
/// favour a family it recognises, and it is told to score each one on its own
/// merits rather than rank them.
const JUDGE_SYSTEM: &str = "\
You grade replies from a hands-free VOICE assistant. Each reply is going straight \
to a text-to-speech engine and will be heard, never read.

Score each candidate answer 0-5 against the stated requirement:
  5 — does exactly what the requirement asks, in speech a person would actually say
  4 — right substance, slightly long or slightly stilted for speech
  3 — usable but padded, or hedges instead of answering
  2 — partially wrong, or reads like written prose (lists, markdown, headings)
  1 — mostly wrong, or unusable as speech
  0 — empty, refuses without cause, or answers a different question

Penalise: markdown, bullet points, emoji, stage directions, meta-commentary about \
being an AI, restating the question before answering, and anything over about \
three sentences unless the requirement asks for more.
Do NOT reward length. Do NOT try to guess which model wrote which answer.

Reply with ONLY a JSON object mapping each candidate letter to its integer score, \
for example {\"A\": 4, \"B\": 2}. No prose, no code fence.";

/// Per-candidate quality: mean score and the per-turn detail.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct Quality {
    judge: String,
    mean_score: Option<f64>,
    scored_turns: usize,
    per_turn: BTreeMap<String, u8>,
}

fn extract_json_object(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

/// Score every candidate's answers with ONE judge, comparatively, turn by turn.
///
/// Comparative-per-turn (rather than one call per candidate) is deliberate: the
/// judge sees the same question answered several ways, which anchors the scale far
/// better than grading answers in isolation, and it costs 20 small calls instead of
/// one huge one. Only the FIRST run of each candidate is judged — the warm-cache
/// repeat measures latency, not a different answer, and paying to grade it twice
/// buys nothing.
async fn judge_replies(
    spec: &str,
    turns: &[Turn],
    runs: &[RunResult],
) -> Result<BTreeMap<String, Quality>> {
    let (judge_provider, judge_model) = spec
        .split_once('/')
        .ok_or_else(|| anyhow!("--judge must look like provider/model"))?;

    let model_config = ModelConfig::new(judge_model)?
        .with_canonical_limits(judge_provider)
        // Generous, because a reasoning judge spends most of a small budget
        // thinking and then returns an EMPTY message — which reads as a judge
        // failure rather than as "the cap was too low".
        .with_max_tokens(Some(2048));
    let provider =
        permagent::providers::create(judge_provider, model_config.clone(), Vec::new()).await?;

    // Judge the cold run of each candidate; that is the answer the model gives.
    let first_runs: Vec<&RunResult> = runs.iter().filter(|r| r.run_index == 0).collect();

    let mut quality: BTreeMap<String, Quality> = BTreeMap::new();
    for run in &first_runs {
        quality.insert(
            run.candidate.clone(),
            Quality {
                judge: spec.to_string(),
                ..Default::default()
            },
        );
    }

    for turn in turns {
        // Letters are assigned by a rotating offset per turn so the same
        // candidate is not always "A" — position bias is a real judge failure
        // mode and this costs nothing to remove.
        let offset = turn.id.bytes().map(|b| b as usize).sum::<usize>() % first_runs.len().max(1);
        let mut ordered: Vec<&&RunResult> = first_runs.iter().collect();
        ordered.rotate_left(offset);

        let mut prompt = format!(
            "Question the user spoke: {}\nRequirement for a good answer: {}\n\n",
            turn.prompt, turn.rubric
        );
        let mut letters: Vec<(char, String)> = Vec::new();
        for (idx, run) in ordered.iter().enumerate() {
            let Some(result) = run.turns.iter().find(|t| t.turn_id == turn.id) else {
                continue;
            };
            if result.error.is_some() {
                continue;
            }
            let letter = (b'A' + idx as u8) as char;
            let spoken = if result.reply_text.trim().is_empty() {
                "(no spoken text — the model only called a tool)"
            } else {
                result.reply_text.trim()
            };
            prompt.push_str(&format!("Candidate {letter}:\n{spoken}\n\n"));
            letters.push((letter, run.candidate.clone()));
        }
        if letters.is_empty() {
            continue;
        }

        let message = Message::user().with_text(prompt);
        let (reply, _usage) = provider
            .complete(
                &model_config,
                "vbench-judge",
                JUDGE_SYSTEM,
                std::slice::from_ref(&message),
                &[],
            )
            .await?;

        let Some(parsed) = extract_json_object(&reply.as_concat_text()) else {
            eprintln!("  judge returned unparseable output for {}", turn.id);
            continue;
        };
        for (letter, candidate) in letters {
            let Some(score) = parsed
                .get(letter.to_string())
                .and_then(|v| v.as_u64())
                .map(|v| v.min(5) as u8)
            else {
                continue;
            };
            if let Some(entry) = quality.get_mut(&candidate) {
                entry.per_turn.insert(turn.id.clone(), score);
            }
        }
    }

    for entry in quality.values_mut() {
        entry.scored_turns = entry.per_turn.len();
        if entry.scored_turns > 0 {
            entry.mean_score = Some(
                entry.per_turn.values().map(|s| *s as f64).sum::<f64>() / entry.scored_turns as f64,
            );
        }
    }
    Ok(quality)
}

// ── Summary ──────────────────────────────────────────────────────────────────

fn percentile(sorted: &[u128], p: f64) -> Option<u128> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted.get(idx).copied()
}

fn summarize(runs: &[RunResult], quality: &BTreeMap<String, Quality>) {
    let mut by_candidate: BTreeMap<String, (String, Vec<&TurnResult>)> = BTreeMap::new();
    for run in runs {
        by_candidate
            .entry(format!("{} (run {})", run.candidate, run.run_index))
            .or_insert_with(|| (run.candidate.clone(), Vec::new()))
            .1
            .extend(run.turns.iter());
    }

    println!(
        "\n| candidate | n | TTFT med | TTFT p90 | boundary med | total med | tools ok | cache hit | $/turn | thought | quality |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for (name, (candidate, turns)) in by_candidate {
        let mut ttft: Vec<u128> = turns.iter().filter_map(|t| t.ttft_ms).collect();
        let mut boundary: Vec<u128> = turns.iter().filter_map(|t| t.first_boundary_ms).collect();
        let mut total: Vec<u128> = turns.iter().map(|t| t.total_ms).collect();
        ttft.sort_unstable();
        boundary.sort_unstable();
        total.sort_unstable();

        let tool_turns: Vec<&&TurnResult> =
            turns.iter().filter(|t| t.expected_tool.is_some()).collect();
        let tool_ok = tool_turns
            .iter()
            .filter(|t| t.tool_correct == Some(true))
            .count();

        let hits: Vec<f64> = turns.iter().filter_map(|t| t.cache_hit_rate).collect();
        let mean_hit = if hits.is_empty() {
            "-".to_string()
        } else {
            format!(
                "{:.0}%",
                hits.iter().sum::<f64>() / hits.len() as f64 * 100.0
            )
        };

        let priced: Vec<f64> = turns.iter().filter_map(|t| t.cost_usd).collect();
        let mean_cost = if priced.is_empty() {
            "-".to_string()
        } else {
            format!("${:.4}", priced.iter().sum::<f64>() / priced.len() as f64)
        };

        let thought = turns.iter().filter(|t| t.thought).count();
        // A candidate that errored on some turns must say so in the table rather
        // than quietly showing a median over the handful that survived.
        let failed = turns.iter().filter(|t| t.error.is_some()).count();
        let name = if failed > 0 {
            format!("{name} — {failed} FAILED")
        } else {
            name
        };

        let quality_cell = quality
            .get(&candidate)
            .and_then(|q| q.mean_score)
            .map(|m| format!("{m:.2}/5"))
            .unwrap_or_else(|| "-".to_string());

        println!(
            "| {name} | {} | {} | {} | {} | {} | {}/{} | {mean_hit} | {mean_cost} | {}/{} | {quality_cell} |",
            turns.len(),
            percentile(&ttft, 0.5)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            percentile(&ttft, 0.9)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            percentile(&boundary, 0.5)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            percentile(&total, 0.5)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            tool_ok,
            tool_turns.len(),
            thought,
            turns.len(),
        );
    }
}
