use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 20_000;
const MAX_METADATA: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerState {
    Active,
    Configured,
    Paused,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub family: String,
    pub identity: String,
    pub path: PathBuf,
    pub size: u64,
    pub digest: Option<String>,
    pub quantization: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consumer {
    pub id: String,
    pub identity: Option<String>,
    pub provider: Option<String>,
    pub artifacts: Vec<PathBuf>,
    pub state: ConsumerState,
    pub uncertainty: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub artifacts: Vec<Artifact>,
    pub consumers: Vec<Consumer>,
    pub issues: Vec<String>,
    pub runtime_status: String,
    #[serde(skip)]
    metadata_bytes: u64,
}

/// Explicit roots make scans hermetic. Environment/config values are filtered
/// to model keys before capture; credentials and unrelated configuration never
/// enter a report. No registry loader (it creates a lock file) is called.
#[derive(Debug, Clone)]
pub struct ScanInput {
    pub data: PathBuf,
    pub config: PathBuf,
    pub ollama: PathBuf,
    pub voice: PathBuf,
    pub environment: BTreeMap<String, String>,
}

impl ScanInput {
    pub fn local() -> Self {
        let data = crate::config::paths::Paths::data_dir();
        let environment: BTreeMap<_, _> = std::env::vars().filter(|(k, _)| model_key(k)).collect();
        // Paths' explicit root override must also isolate auxiliary stores;
        // otherwise a controlled scan can accidentally inspect the real home.
        if std::env::var_os("PERMAGENT_PATH_ROOT").is_some() {
            return Self {
                config: data.join("config.yaml"),
                ollama: data.join("ollama/models"),
                voice: data.join("voice"),
                data,
                environment,
            };
        }
        let ollama = std::env::var_os("OLLAMA_MODELS")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".ollama/models"));
        let voice = dirs::data_dir()
            .unwrap_or_default()
            .join("permagent/models/voice");
        Self {
            data,
            config: crate::config::paths::Paths::in_config_dir("config.yaml"),
            ollama,
            voice,
            environment,
        }
    }
}

fn model_key(key: &str) -> bool {
    ((key.starts_with("PERMAGENT_ROLE_") || key.starts_with("PERMAGENT_PACK_"))
        && (key.ends_with("_MODEL") || key.ends_with("_PROVIDER")))
        || matches!(
            key,
            "GOOSE_MODEL"
                | "GOOSE_PROVIDER"
                | "chat_model"
                | "chat_provider"
                | "harness_model"
                | "harness_provider"
                | "voice_model"
                | "voice_provider"
                | "LOCAL_WHISPER_MODEL"
                | "GOOSE_TOOLSHIM_OLLAMA_MODEL"
                | "INITIATIVE_DRAFT_MODEL"
                | "INITIATIVE_DRAFT_PROVIDER"
                | "PLAYBOOK_DISTILL_MODEL"
                | "PLAYBOOK_DISTILL_PROVIDER"
                | "ORCHESTRATOR_DECOMPOSITION_MODEL"
                | "ORCHESTRATOR_DECOMPOSITION_PROVIDER"
        )
}

fn read_small(path: &Path, inv: &mut Inventory) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        inv.metadata_bytes < 16 * MAX_METADATA,
        "scan metadata budget exhausted"
    );
    // Check before opening: a FIFO must not block a weekly scan waiting for a
    // writer. Symlinked metadata is explicitly outside this bounded snapshot.
    anyhow::ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "not a regular metadata file"
    );
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(file.metadata()?.is_file(), "not a regular file");
    let mut bytes = Vec::new();
    file.take(MAX_METADATA + 1).read_to_end(&mut bytes)?;
    inv.metadata_bytes += bytes.len() as u64;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_METADATA,
        "metadata exceeds scan bound"
    );
    Ok(bytes)
}

fn json(path: &Path, inv: &mut Inventory) -> Option<Value> {
    match read_small(path, inv).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
        Ok(v) => Some(v),
        Err(e) => {
            inv.issues.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

/// Sorted, bounded traversal, without following directory symlinks. Every
/// truncation/error is durable report coverage, never a successful empty scan.
fn files(root: &Path, inv: &mut Inventory) -> Vec<PathBuf> {
    let mut pending = BTreeSet::from([root.to_path_buf()]);
    let mut output = Vec::new();
    let mut visited = 0;
    while let Some(path) = pending.pop_first() {
        visited += 1;
        if visited > MAX_ENTRIES {
            inv.issues
                .push(format!("{}: scan entry bound reached", root.display()));
            break;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let mut children = Vec::new();
                    for entry in entries.take(MAX_ENTRIES + 1) {
                        match entry {
                            Ok(e) => children.push(e.path()),
                            Err(e) => inv.issues.push(format!("{}: {e}", path.display())),
                        }
                    }
                    if children.len() + pending.len() > MAX_ENTRIES {
                        // Skip the entire oversized directory; OS enumeration
                        // order must not decide which artifacts make the report.
                        inv.issues
                            .push(format!("{}: directory entry bound reached", path.display()));
                    } else {
                        pending.extend(children);
                    }
                }
                Err(e) => inv.issues.push(format!("{}: {e}", path.display())),
            },
            Ok(m) if m.is_file() => output.push(path),
            Ok(_) => inv.issues.push(format!(
                "{}: symlink/special file not traversed",
                path.display()
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && path == root => {}
            Err(e) => inv.issues.push(format!("{}: {e}", path.display())),
        }
    }
    output.sort();
    output
}

fn quantization(identity: &str) -> Option<String> {
    let upper = identity.to_ascii_uppercase();
    [
        "Q8_0", "Q8", "Q6_K", "Q5_K_M", "Q5_0", "Q4_K_M", "Q4_0", "BF16", "F16", "INT8",
    ]
    .iter()
    .find(|q| upper.contains(**q))
    .map(|q| (*q).to_string())
}

fn artifact(
    inv: &mut Inventory,
    path: &Path,
    family: &str,
    identity: &str,
    digest: Option<String>,
    source: &str,
) {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {
            // Large weight files are never hashed by the weekly scan. Small
            // files and content-addressed blob names provide cheap identities.
            let digest = digest.or_else(|| {
                if m.len() <= 64 * 1024 {
                    match read_small(path, inv) {
                        Ok(b) => Some(format!("sha256:{}", hex::encode(Sha256::digest(b)))),
                        Err(e) => {
                            inv.issues
                                .push(format!("{}: cheap digest unavailable: {e}", path.display()));
                            None
                        }
                    }
                } else {
                    None
                }
            });
            inv.artifacts.push(Artifact {
                family: family.into(),
                identity: identity.into(),
                path: path.into(),
                size: m.len(),
                digest,
                quantization: quantization(identity),
                source: source.into(),
            });
        }
        Ok(_) => {}
        Err(e) => inv.issues.push(format!("{}: {e}", path.display())),
    }
}

fn consumer(
    inv: &mut Inventory,
    id: &str,
    identity: Option<String>,
    provider: Option<String>,
    paths: Vec<PathBuf>,
    state: ConsumerState,
    uncertainty: &str,
) {
    inv.consumers.push(Consumer {
        id: id.into(),
        identity,
        provider,
        artifacts: paths,
        state,
        uncertainty: uncertainty.into(),
    });
}

fn scan_ollama(input: &ScanInput, inv: &mut Inventory) {
    let manifests = input.ollama.join("manifests");
    for path in files(&manifests, inv) {
        let Some(value) = json(&path, inv) else {
            continue;
        };
        let parts: Vec<_> = path
            .strip_prefix(&manifests)
            .unwrap_or(&path)
            .components()
            .map(|p| p.as_os_str().to_string_lossy().into_owned())
            .collect();
        let Some(tag) = parts.last() else { continue };
        let name = parts.iter().rev().nth(1).cloned().unwrap_or_default();
        let namespace = parts.iter().rev().nth(2).map(String::as_str).unwrap_or("");
        let family = if namespace == "library" {
            name.clone()
        } else {
            format!("{namespace}/{name}")
        };
        let identity = format!("{family}:{tag}");
        let manifest_digest = read_small(&path, inv)
            .ok()
            .map(|b| format!("sha256:{}", hex::encode(Sha256::digest(b))));
        artifact(
            inv,
            &path,
            &family,
            &identity,
            manifest_digest,
            "Ollama on-disk manifest identity",
        );
        let mut paths = vec![path.clone()];
        let layers = value
            .get("layers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| {
                inv.issues
                    .push(format!("{}: manifest layers missing", path.display()));
                Vec::new()
            });
        for layer in layers.iter().chain(value.get("config")) {
            let Some(digest) = layer.get("digest").and_then(Value::as_str) else {
                continue;
            };
            let Some(hex) = digest
                .strip_prefix("sha256:")
                .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            else {
                inv.issues
                    .push(format!("{}: invalid blob digest", path.display()));
                continue;
            };
            let blob = input.ollama.join("blobs").join(format!("sha256-{hex}"));
            artifact(
                inv,
                &blob,
                &family,
                &identity,
                Some(digest.into()),
                "Ollama on-disk manifest; blob digest is a name, not a rehash",
            );
            paths.push(blob);
        }
        let state = if paths.iter().all(|p| p.is_file()) {
            ConsumerState::Configured
        } else {
            ConsumerState::Absent
        };
        consumer(
            inv,
            &format!("ollama:{}", path.display()),
            Some(identity),
            Some("ollama".into()),
            paths,
            state,
            "Runtime reachability and loaded state unknown; manifest only, no probe",
        );
    }
    // Include unreferenced blobs: installed bytes are artifacts even when no tag
    // currently consumes them. Their upstream identity remains unknown.
    for path in files(&input.ollama.join("blobs"), inv) {
        if !inv.artifacts.iter().any(|a| a.path == path) {
            let identity = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let digest = identity
                .strip_prefix("sha256-")
                .filter(|hex| hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit()))
                .map(|hex| format!("sha256:{hex}"));
            artifact(
                inv,
                &path,
                "unknown",
                &identity,
                digest,
                "unreferenced Ollama blob; digest from filename, not a rehash",
            );
        }
    }
}

fn scan_registry(input: &ScanInput, inv: &mut Inventory) {
    let path = input.data.join("models/registry.json");
    if !path.exists() {
        consumer(
            inv,
            "local-inference registry",
            None,
            None,
            vec![],
            ConsumerState::Absent,
            "No registry file",
        );
        return;
    }
    let Some(value) = json(&path, inv) else {
        consumer(
            inv,
            "local-inference registry",
            None,
            None,
            vec![],
            ConsumerState::Unknown,
            "Registry unreadable or malformed",
        );
        return;
    };
    let Some(models) = value.get("models").and_then(Value::as_array) else {
        inv.issues.push("registry models array missing".into());
        return;
    };
    if models.len() > MAX_ENTRIES {
        inv.issues.push("registry entry bound reached".into());
    }
    for model in models.iter().take(MAX_ENTRIES) {
        let Some(id) = model.get("id").and_then(Value::as_str) else {
            inv.issues.push("registry id missing".into());
            continue;
        };
        let family = model.get("repo_id").and_then(Value::as_str).unwrap_or(id);
        let mut paths = Vec::new();
        for field in ["local_path", "mmproj_path"] {
            if let Some(path) = model.get(field).and_then(Value::as_str) {
                paths.push(PathBuf::from(path));
            }
        }
        if let Some(shards) = model.get("shard_files").and_then(Value::as_array) {
            paths.extend(
                shards
                    .iter()
                    .filter_map(|s| s.get("local_path").and_then(Value::as_str))
                    .map(PathBuf::from),
            );
        }
        paths.sort();
        paths.dedup();
        for path in &paths {
            artifact(inv, path, family, id, None, "local-inference registry");
            if let Some(a) = inv.artifacts.last_mut().filter(|a| a.path == *path) {
                a.quantization = model
                    .get("quantization")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(a.quantization.take());
            }
        }
        let state = if paths.is_empty() {
            ConsumerState::Unknown
        } else if paths.iter().all(|p| p.is_file()) {
            ConsumerState::Configured
        } else {
            ConsumerState::Absent
        };
        consumer(
            inv,
            &format!("registry:{id}"),
            Some(id.into()),
            Some("local-inference".into()),
            paths,
            state,
            "Registry presence does not prove a loaded model",
        );
    }
}

pub fn scan(input: &ScanInput) -> Inventory {
    let mut inv = Inventory {
        runtime_status: "unknown: Ollama not probed; unreachable runtimes are never started".into(),
        ..Default::default()
    };
    scan_ollama(input, &mut inv);
    scan_registry(input, &mut inv);
    let mut config = BTreeMap::new();
    match read_small(&input.config, &mut inv).and_then(|b| Ok(serde_yaml::from_slice::<Value>(&b)?))
    {
        Ok(v) => {
            if let Some(map) = v.as_object() {
                config.extend(
                    map.iter()
                        .filter(|(k, _)| model_key(k))
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))),
                );
            } else {
                inv.issues.push("configuration is not a mapping".into());
            }
        }
        Err(e) => inv.issues.push(format!("{}: {e}", input.config.display())),
    }
    config.extend(
        input
            .environment
            .iter()
            .filter(|(k, _)| model_key(k))
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    let mut keys = BTreeSet::from([
        "GOOSE_MODEL".to_string(),
        "chat_model".into(),
        "harness_model".into(),
        "voice_model".into(),
    ]);
    for (namespace, roles) in [
        (
            "ROLE",
            vec!["ORCHESTRATE", "EDIT", "MECHANICAL", "LOCAL", "REVIEW"],
        ),
        (
            "PACK",
            vec!["EDIT", "HARD", "MECHANICAL", "LOCAL", "REVIEW"],
        ),
    ] {
        for role in roles {
            keys.insert(format!("PERMAGENT_{namespace}_{role}_MODEL"));
        }
    }
    keys.extend(
        config
            .keys()
            .filter(|k| k.ends_with("_MODEL") && *k != "LOCAL_WHISPER_MODEL")
            .cloned(),
    );
    for key in keys {
        let provider_key = key
            .strip_suffix("_MODEL")
            .map(|p| format!("{p}_PROVIDER"))
            .unwrap_or_else(|| key.replace("_model", "_provider"));
        let identity = config.get(&key).cloned();
        let provider = config.get(&provider_key).cloned();
        let state = if identity.as_deref().is_some_and(|v| v.trim().is_empty()) {
            if matches!(
                key.as_str(),
                "INITIATIVE_DRAFT_MODEL" | "PLAYBOOK_DISTILL_MODEL"
            ) {
                ConsumerState::Paused
            } else {
                ConsumerState::Unknown
            }
        } else if provider.as_deref() == Some("qwen38_split") {
            ConsumerState::Paused
        } else if identity.is_some() && provider.is_some() {
            ConsumerState::Configured
        } else if identity.is_none() && provider.is_none() {
            ConsumerState::Absent
        } else {
            ConsumerState::Unknown
        };
        consumer(&mut inv, &key, identity, provider, vec![], state,
            "Explicit configuration only; unset/partial mappings inherit runtime defaults, not inferred installation");
    }
    let schedule_path = input.data.join("librarian_schedule.json");
    let librarian = if schedule_path.exists() {
        json(&schedule_path, &mut inv)
    } else {
        None
    };
    let model = librarian
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .unwrap_or("qwen2.5:7b");
    consumer(
        &mut inv,
        "librarian_schedule.model",
        Some(model.into()),
        Some("ollama".into()),
        vec![],
        if librarian
            .as_ref()
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            == Some(false)
        {
            ConsumerState::Paused
        } else if librarian.is_some() {
            ConsumerState::Configured
        } else {
            ConsumerState::Unknown
        },
        "Fallback configuration; dedicated endpoint/runtime not probed",
    );
    for (id, model) in [
        ("reader", "qwen2.5:7b"),
        ("initiative", "qwen2.5:7b"),
        ("playbook", "qwen2.5:7b"),
        ("ollama provider default", "qwen3"),
        ("toolshim", "mistral-nemo"),
    ] {
        let key = match id {
            "toolshim" => "GOOSE_TOOLSHIM_OLLAMA_MODEL",
            "initiative" => "INITIATIVE_DRAFT_MODEL",
            "playbook" => "PLAYBOOK_DISTILL_MODEL",
            _ => "",
        };
        let model = if id == "toolshim" {
            input.environment.get(key)
        } else {
            config.get(key)
        }
        .map(String::as_str)
        .unwrap_or(model);
        let provider_key = key.strip_suffix("_MODEL").map(|p| format!("{p}_PROVIDER"));
        let provider = provider_key
            .as_ref()
            .and_then(|k| config.get(k))
            .map(String::as_str)
            .unwrap_or("ollama");
        consumer(&mut inv, id, Some(model.into()), Some(provider.into()), vec![],
            if model.trim().is_empty() && matches!(id, "initiative" | "playbook") { ConsumerState::Paused } else { ConsumerState::Unknown },
            "Source default/reference or explicit override only; not evidence of installation or activity");
    }
    consumer(
        &mut inv,
        "ORCHESTRATOR_DECOMPOSITION_MODEL",
        config.get("ORCHESTRATOR_DECOMPOSITION_MODEL").cloned(),
        config.get("ORCHESTRATOR_DECOMPOSITION_PROVIDER").cloned(),
        vec![],
        ConsumerState::Unknown,
        "Explicit decomposition pin; derived routing not probed",
    );
    consumer(
        &mut inv,
        "paused split",
        Some("qwen3.8-27b".into()),
        Some("qwen38_split".into()),
        vec![],
        ConsumerState::Paused,
        "Operationally paused per W1 ruling 3; inventory only; never activate",
    );
    scan_harvester(input, &mut inv);
    for (id, filename) in [
        ("kokoro", "kokoro-v1.0.onnx"),
        ("kokoro voices", "voices-v1.0.bin"),
        ("moonshine", "sherpa-onnx-moonshine-tiny-en-int8"),
        ("streaming STT", "sherpa-onnx-streaming-zipformer-en"),
        (
            "wake word",
            "sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01",
        ),
        (
            "speaker",
            "speaker/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx",
        ),
    ] {
        let path = input.voice.join(filename);
        let paths = files(&path, &mut inv);
        for p in &paths {
            artifact(&mut inv, p, id, filename, None, "voice backend local path");
        }
        consumer(
            &mut inv,
            id,
            Some(filename.into()),
            None,
            paths.clone(),
            if paths.is_empty() {
                ConsumerState::Absent
            } else {
                ConsumerState::Configured
            },
            "Presence only; bundle completeness and runtime loading unverified",
        );
    }
    for path in files(&input.data.join("models"), &mut inv) {
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("gguf" | "onnx" | "bin" | "safetensors")
        ) && !inv.artifacts.iter().any(|a| a.path == path)
        {
            let family = if filename.contains("whisper") || filename == "model-tiny-q80.gguf" {
                "whisper"
            } else {
                "unknown"
            };
            artifact(
                &mut inv,
                &path,
                family,
                &filename,
                None,
                "local model store; filename identity only",
            );
        }
    }
    let whisper = config.get("LOCAL_WHISPER_MODEL").cloned();
    // Catalog filenames copied from dictation/whisper.rs::MODELS. An installed
    // optional size is not automatically a consumer of the selected size.
    let filename = match whisper.as_deref() {
        Some("tiny") => Some("model-tiny-q80.gguf"),
        Some("base") => Some("whisper-base-q8_0.gguf"),
        Some("small") => Some("whisper-small-q8_0.gguf"),
        Some("medium") => Some("whisper-medium-q8_0.gguf"),
        _ => None,
    };
    let whisper_paths: Vec<_> = filename
        .map(|f| input.data.join("models").join(f))
        .into_iter()
        .collect();
    let state = if whisper.is_none() {
        ConsumerState::Absent
    } else if whisper_paths.is_empty() {
        ConsumerState::Unknown
    } else if whisper_paths.iter().all(|p| p.is_file()) {
        ConsumerState::Configured
    } else {
        ConsumerState::Absent
    };
    consumer(
        &mut inv,
        "LOCAL_WHISPER_MODEL",
        whisper,
        None,
        whisper_paths,
        state,
        "Whisper selection is a catalog id; presence does not prove runtime loading",
    );
    consumer(&mut inv, "signed Whisper bundle", Some("base".into()), None, vec![], ConsumerState::Unknown,
        "Signed app bundle path is supplied by frontend and unavailable to this scan; application update required for bundled bytes");
    // Link explicit consumers to exact observed identities only, never fuzzy tag
    // matching. Multiple consumers can reference one physical artifact.
    for c in &mut inv.consumers {
        if c.artifacts.is_empty() {
            c.artifacts = inv
                .artifacts
                .iter()
                .filter(|a| c.identity.as_deref() == Some(a.identity.as_str()))
                .map(|a| a.path.clone())
                .collect();
        }
    }
    inv.artifacts
        .sort_by(|a, b| (&a.path, &a.identity).cmp(&(&b.path, &b.identity)));
    inv.artifacts
        .dedup_by(|a, b| a.path == b.path && a.identity == b.identity);
    inv.consumers.sort_by(|a, b| a.id.cmp(&b.id));
    inv.consumers.dedup_by(|a, b| a.id == b.id);
    inv.issues.sort();
    inv.issues.dedup();
    inv
}

fn scan_harvester(input: &ScanInput, inv: &mut Inventory) {
    let path = input.data.join("harvester.toml");
    let mut models = BTreeMap::new();
    // Bounded subset reader: only literal model assignments in [models]. Any
    // syntax outside this subset is explicitly uncertain, never executed.
    if path.exists() {
        match read_small(&path, inv).and_then(|b| Ok(String::from_utf8(b)?)) {
            Ok(text) => {
                let mut in_models = false;
                for line in text.lines().map(str::trim) {
                    if line.starts_with('[') {
                        in_models = line == "[models]";
                        continue;
                    }
                    if !in_models || line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if let Some((key, value)) = line.split_once('=') {
                        let key = key.trim();
                        if matches!(key, "generation" | "embedding") {
                            match serde_json::from_str::<String>(value.trim()) {
                                Ok(v) => {
                                    models.insert(key.to_string(), v);
                                }
                                Err(_) => inv.issues.push(format!(
                                    "harvester {key}: unsupported TOML literal; identity unknown"
                                )),
                            }
                        }
                    }
                }
            }
            Err(e) => inv.issues.push(format!("harvester configuration: {e}")),
        }
    }
    for key in ["generation", "embedding"] {
        consumer(inv, &format!("harvester.{key}"), models.get(key).cloned(), None, vec![],
            if models.contains_key(key) { ConsumerState::Configured } else { ConsumerState::Unknown },
            "Default deployment file only; CLI --config override and deployed process state unknown");
    }
}
