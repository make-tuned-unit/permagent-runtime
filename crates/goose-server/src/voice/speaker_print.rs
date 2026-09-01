//! Learned speaker verification for the `/voice` route.
//!
//! The previous implementation averaged 32 log-mel bands. That can separate
//! tones, but it is not a speaker embedding and admitted same-room speech in
//! the 2026-08-27 kitchen session at 0.997–0.998. This module uses sherpa-onnx
//! with the English CAM++ model from 3D-Speaker instead. Enrollment audio is
//! processed in memory and never written to disk; only a normalized embedding
//! is persisted.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const NEED_UTTERANCES: usize = 3;
/// sherpa-onnx's speaker-verification examples use 0.6 for CAM++.
pub const ADMIT_THRESHOLD: f32 = 0.60;
/// Prevent an enrollment assembled from multiple substantially different
/// voices. This is deliberately lower than the runtime admission threshold so
/// natural changes between the three prompted sentences still pass.
const ENROLL_PAIR_THRESHOLD: f32 = 0.45;
const STORE_SCHEMA: u32 = 2;
pub const MODEL_ID: &str = "3dspeaker-campplus-en-voxceleb-16k";
pub const MODEL_FILENAME: &str = "3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";
pub const MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";
pub const MODEL_SHA256: &str = "357a834f702b80161e5b981182c038e18553c1f2ca752ed6cec2052365d4129b";
pub const MODEL_BYTES: u64 = 29_596_978;
pub const DOWNLOAD_ID: &str = "speaker-identity-campplus";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gate {
    /// No identity has been enrolled yet.
    Open,
    Admit {
        score: f32,
    },
    Reject {
        score: f32,
    },
    /// An enrolled identity exists but cannot safely be compared. Callers
    /// must fail closed rather than treating this as an open installation.
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicePrint {
    pub schema_version: u32,
    pub model: String,
    pub dim: usize,
    pub vector: Vec<f32>,
    pub created_at: String,
    pub n_utterances: usize,
}

#[derive(Clone, Debug)]
pub struct SpeakerModelPaths {
    pub model_path: PathBuf,
}

impl SpeakerModelPaths {
    pub fn default_paths() -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("permagent")
            .join("models")
            .join("voice")
            .join("speaker");
        Self {
            model_path: base.join(MODEL_FILENAME),
        }
    }

    pub fn models_exist(&self) -> bool {
        self.model_path.is_file()
    }
}

/// One shared extractor. Access is serialized because the native runtime's
/// thread-safety guarantee covers a single object, not simultaneous calls on
/// that object from multiple WebSocket blocking tasks.
pub struct SpeakerVerifier {
    extractor: Mutex<SpeakerEmbeddingExtractor>,
    dim: usize,
}

impl SpeakerVerifier {
    pub fn new(model_path: &Path) -> anyhow::Result<Self> {
        if !model_path.is_file() {
            anyhow::bail!("speaker model is missing: {}", model_path.display());
        }
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(model_path.to_string_lossy().into_owned()),
            num_threads: 2,
            debug: false,
            provider: Some("cpu".into()),
        };
        let extractor = SpeakerEmbeddingExtractor::create(&config)
            .context("sherpa-onnx could not load the CAM++ speaker model")?;
        let dim =
            usize::try_from(extractor.dim()).context("invalid speaker embedding dimension")?;
        if dim == 0 {
            anyhow::bail!("speaker model returned a zero embedding dimension");
        }
        Ok(Self {
            extractor: Mutex::new(extractor),
            dim,
        })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Compute a learned embedding. CAM++ needs enough voiced context to be
    /// meaningful, so sub-500ms captures are rejected before native inference.
    pub fn extract(&self, samples: &[f32], sample_rate: u32) -> anyhow::Result<Option<Vec<f32>>> {
        if sample_rate == 0 || samples.len() < sample_rate as usize / 2 {
            return Ok(None);
        }
        let extractor = self
            .extractor
            .lock()
            .map_err(|_| anyhow::anyhow!("speaker extractor lock poisoned"))?;
        let stream = extractor
            .create_stream()
            .context("could not create speaker embedding stream")?;
        stream.accept_waveform(sample_rate as i32, samples);
        stream.input_finished();
        if !extractor.is_ready(&stream) {
            return Ok(None);
        }
        let mut embedding = extractor
            .compute(&stream)
            .context("speaker model returned no embedding")?;
        if embedding.len() != self.dim {
            anyhow::bail!(
                "speaker model returned {} dimensions; expected {}",
                embedding.len(),
                self.dim
            );
        }
        l2_normalize(&mut embedding)?;
        Ok(Some(embedding))
    }
}

pub fn store_path() -> PathBuf {
    permagent::config::paths::Paths::in_state_dir("data").join("voice_print.json")
}

pub fn load() -> Option<VoicePrint> {
    load_from(&store_path())
}

fn load_from(path: &Path) -> Option<VoicePrint> {
    let raw = std::fs::read_to_string(path).ok()?;
    let print: VoicePrint = serde_json::from_str(&raw).ok()?;
    valid_print(&print).then_some(print)
}

fn valid_print(print: &VoicePrint) -> bool {
    print.schema_version == STORE_SCHEMA
        && print.model == MODEL_ID
        && print.n_utterances >= NEED_UTTERANCES
        && print.dim > 0
        && print.vector.len() == print.dim
        && print.vector.iter().all(|x| x.is_finite())
        && print.vector.iter().map(|x| x * x).sum::<f32>() > 0.5
}

pub fn save(print: &VoicePrint) -> std::io::Result<()> {
    save_to(&store_path(), print)
}

fn save_to(path: &Path, print: &VoicePrint) -> std::io::Result<()> {
    if !valid_print(print) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid or obsolete learned speaker print",
        ));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(print)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
    )?;
    std::fs::rename(&tmp, path)
}

pub fn clear() -> std::io::Result<()> {
    match std::fs::remove_file(store_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denom = na * nb;
    (denom >= 1e-9).then(|| (dot / denom).clamp(-1.0, 1.0))
}

pub fn gate_against(print: Option<&VoicePrint>, embedding: &[f32]) -> Gate {
    let Some(print) = print else {
        return Gate::Open;
    };
    if !valid_print(print) || embedding.len() != print.dim {
        return Gate::Unavailable;
    }
    match cosine(&print.vector, embedding) {
        Some(score) if score >= ADMIT_THRESHOLD => Gate::Admit { score },
        Some(score) => Gate::Reject { score },
        None => Gate::Unavailable,
    }
}

pub fn mean_l2(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let dim = vectors.first()?.len();
    if dim == 0 || vectors.iter().any(|v| v.len() != dim) {
        return None;
    }
    let mut acc = vec![0.0; dim];
    for v in vectors {
        for (slot, value) in acc.iter_mut().zip(v) {
            *slot += *value / vectors.len() as f32;
        }
    }
    l2_normalize(&mut acc).ok()?;
    Some(acc)
}

fn enrollment_is_coherent(vectors: &[Vec<f32>]) -> bool {
    for i in 0..vectors.len() {
        for j in i + 1..vectors.len() {
            if cosine(&vectors[i], &vectors[j]).is_none_or(|score| score < ENROLL_PAIR_THRESHOLD) {
                return false;
            }
        }
    }
    true
}

fn l2_normalize(vector: &mut [f32]) -> anyhow::Result<()> {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !norm.is_finite() || norm < 1e-9 {
        anyhow::bail!("speaker model returned an empty embedding");
    }
    for value in vector {
        *value /= norm;
    }
    Ok(())
}

pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

pub fn from_utterances(vectors: &[Vec<f32>]) -> Option<VoicePrint> {
    if vectors.len() < NEED_UTTERANCES || !enrollment_is_coherent(vectors) {
        return None;
    }
    let vector = mean_l2(vectors)?;
    Some(VoicePrint {
        schema_version: STORE_SCHEMA,
        model: MODEL_ID.into(),
        dim: vector.len(),
        n_utterances: vectors.len(),
        vector,
        created_at: now_rfc3339(),
    })
}

/// Setup prompts are deliberately agent-name-independent. The user's agent can
/// be renamed at any time without invalidating the identity workflow.
pub const PROMPTS: [&str; NEED_UTTERANCES] = [
    "What's on my board?",
    "This is the voice I want you to answer.",
    "Tell me something interesting.",
];

pub fn prompt_at(have: usize) -> Option<&'static str> {
    PROMPTS.get(have).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print(vector: Vec<f32>) -> VoicePrint {
        VoicePrint {
            schema_version: STORE_SCHEMA,
            model: MODEL_ID.into(),
            dim: vector.len(),
            vector,
            created_at: "t0".into(),
            n_utterances: NEED_UTTERANCES,
        }
    }

    #[test]
    fn no_print_is_fail_open() {
        assert_eq!(gate_against(None, &[0.1, 0.2, 0.3]), Gate::Open);
    }

    #[test]
    fn learned_embeddings_admit_self_and_reject_other() {
        let me = print(vec![1.0, 0.0, 0.0]);
        assert!(matches!(
            gate_against(Some(&me), &[0.99, 0.01, 0.0]),
            Gate::Admit { .. }
        ));
        assert!(matches!(
            gate_against(Some(&me), &[0.0, 0.0, 1.0]),
            Gate::Reject { .. }
        ));
    }

    #[test]
    fn enrolled_dimension_mismatch_fails_closed() {
        let me = print(vec![1.0, 0.0]);
        assert_eq!(gate_against(Some(&me), &[1.0, 0.0, 0.0]), Gate::Unavailable);
    }

    #[test]
    fn incoherent_enrollment_is_rejected() {
        assert!(from_utterances(&[
            vec![1.0, 0.0, 0.0],
            vec![0.99, 0.01, 0.0],
            vec![0.0, 0.0, 1.0],
        ])
        .is_none());
    }

    #[test]
    fn three_coherent_utterances_make_versioned_print() {
        let print = from_utterances(&[
            vec![1.0, 0.0, 0.0],
            vec![0.99, 0.01, 0.0],
            vec![0.98, 0.02, 0.0],
        ])
        .unwrap();
        assert_eq!(print.n_utterances, NEED_UTTERANCES);
        assert_eq!(print.schema_version, STORE_SCHEMA);
        assert_eq!(print.model, MODEL_ID);
    }

    #[test]
    fn obsolete_spectral_print_does_not_load() {
        let dir = std::env::temp_dir().join(format!("voice-print-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voice_print.json");
        std::fs::write(
            &path,
            r#"{"dim":2,"vector":[0.6,0.8],"created_at":"t0","n_utterances":3}"#,
        )
        .unwrap();
        assert!(load_from(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_round_trip() {
        let dir = std::env::temp_dir().join(format!("voice-print-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voice_print.json");
        let print = print(vec![0.6, 0.8]);
        save_to(&path, &print).unwrap();
        assert_eq!(load_from(&path).unwrap(), print);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompts_never_hardcode_an_agent_name() {
        for prompt in PROMPTS {
            assert!(!prompt.to_ascii_lowercase().contains("henry"));
        }
    }

    /// Opt-in integration gate for the pinned release asset. CI/unit runs do
    /// not download 29.6MB; the audit invokes this with the verified /tmp file.
    #[test]
    fn real_campp_model_loads_and_embeds_speech() {
        let Ok(model) = std::env::var("PERMAGENT_SPEAKER_MODEL_TEST") else {
            return;
        };
        let wav_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../goose/src/dictation/testdata/speech_mono_16k.wav");
        let mut reader = hound::WavReader::open(wav_path).expect("open speech fixture");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        let samples = reader
            .samples::<i16>()
            .map(|s| s.expect("wav sample") as f32 / i16::MAX as f32)
            .collect::<Vec<_>>();

        let verifier = SpeakerVerifier::new(Path::new(&model)).expect("load CAM++");
        let first = verifier
            .extract(&samples, spec.sample_rate)
            .expect("run CAM++")
            .expect("embedding ready");
        let second = verifier
            .extract(&samples, spec.sample_rate)
            .expect("run CAM++ again")
            .expect("embedding ready");
        assert_eq!(first.len(), verifier.dim());
        assert!(verifier.dim() >= 128);
        assert!(cosine(&first, &second).unwrap() > 0.999);
    }
}
