//! Resident TTS provider warm-up — one short synthesis per provider/voice generation,
//! shared across socket reconnects (V5).

use crate::voice::provider::{AudioOutput, TextToSpeech, TtsConfig};
use futures::FutureExt;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Fixed warm-up utterance (≤16 words). Acceptance latency trials use this same
/// text so log `first_chunk_ms` on the reply path is comparable.
pub const TTS_PROVIDER_WARM_SENTENCE: &str = "The voice assistant is ready when you are.";

const WARM_RETRY_MIN_INTERVAL: Duration = Duration::from_secs(60);

type SharedWarmFuture = futures::future::Shared<Pin<Box<dyn Future<Output = ()> + Send>>>;

pub fn generation_key(provider_generation: u64, voice_id: Option<String>) -> String {
    format!(
        "{}:{}",
        provider_generation,
        voice_id.unwrap_or_else(|| "default".to_string())
    )
}

struct GenerationWarmSlot {
    warmed: bool,
    failed_attempts: u8,
    last_failure_at: Option<Instant>,
    in_flight: Option<SharedWarmFuture>,
}

impl GenerationWarmSlot {
    fn may_start_attempt(&self, now: Instant) -> bool {
        if self.warmed || self.in_flight.is_some() {
            return false;
        }
        match self.failed_attempts {
            0 => true,
            1 => self
                .last_failure_at
                .is_some_and(|t| now.duration_since(t) >= WARM_RETRY_MIN_INTERVAL),
            _ => false,
        }
    }
}

struct TtsWarmRegistry {
    slots: HashMap<String, GenerationWarmSlot>,
}

impl TtsWarmRegistry {
    fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }
}

static TTS_WARM_REGISTRY: std::sync::OnceLock<Mutex<TtsWarmRegistry>> = std::sync::OnceLock::new();

fn registry() -> &'static Mutex<TtsWarmRegistry> {
    TTS_WARM_REGISTRY.get_or_init(|| Mutex::new(TtsWarmRegistry::new()))
}

#[cfg(test)]
pub(crate) static WARM_TEST_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
pub(crate) async fn reset_registry_for_test() {
    if let Some(lock) = TTS_WARM_REGISTRY.get() {
        lock.lock().await.slots.clear();
    }
}

#[cfg(test)]
static TEST_WARM_CLOCK: std::sync::OnceLock<std::sync::Mutex<Instant>> = std::sync::OnceLock::new();

#[cfg(test)]
pub(crate) fn set_test_warm_clock(now: Instant) {
    let lock = TEST_WARM_CLOCK.get_or_init(|| std::sync::Mutex::new(now));
    *lock.lock().unwrap_or_else(|e| e.into_inner()) = now;
}

#[cfg(test)]
pub(crate) fn advance_test_warm_clock(by: Duration) {
    let lock = TEST_WARM_CLOCK.get_or_init(|| std::sync::Mutex::new(Instant::now()));
    let mut now = lock.lock().unwrap_or_else(|e| e.into_inner());
    *now += by;
}

fn warm_now() -> Instant {
    #[cfg(test)]
    {
        if let Some(lock) = TEST_WARM_CLOCK.get() {
            return *lock.lock().unwrap_or_else(|e| e.into_inner());
        }
    }
    Instant::now()
}

fn warm_speech_text() -> String {
    let plan = crate::voice::prosody::plan(TTS_PROVIDER_WARM_SENTENCE);
    crate::voice::speakable::speakable(&plan.speech)
        .unwrap_or_else(|| TTS_PROVIDER_WARM_SENTENCE.to_string())
}

fn run_blocking_warm(
    tts: Arc<dyn TextToSpeech>,
    voice_id: Option<String>,
    speed: f32,
) -> anyhow::Result<AudioOutput> {
    let speech = warm_speech_text();
    let mut audio = tts.synthesize(
        &speech,
        &TtsConfig {
            voice_id,
            speed,
            lexicon: crate::voice::user_lexicon::current(),
        },
    )?;
    crate::voice::loudness::master(&mut audio.samples, audio.sample_rate, &speech);
    Ok(audio)
}

async fn finish_warm_attempt(generation_key: &str, started_at: Instant, success: bool) {
    let mut reg = registry().lock().await;
    let slot = reg
        .slots
        .entry(generation_key.to_string())
        .or_insert_with(|| GenerationWarmSlot {
            warmed: false,
            failed_attempts: 0,
            last_failure_at: None,
            in_flight: None,
        });
    slot.in_flight = None;
    if success {
        slot.warmed = true;
        tracing::info!(
            target: "permagentd::voice",
            event = "voice_tts_warm",
            stage = "complete",
            generation_key,
            warm_ms = warm_now()
                .saturating_duration_since(started_at)
                .as_millis(),
            "voice TTS provider warm complete"
        );
    } else {
        slot.failed_attempts = slot.failed_attempts.saturating_add(1);
        slot.last_failure_at = Some(warm_now());
        tracing::warn!(
            target: "permagentd::voice",
            event = "voice_tts_warm",
            stage = "failure",
            generation_key,
            warm_ms = warm_now()
                .saturating_duration_since(started_at)
                .as_millis(),
            failed_attempts = slot.failed_attempts,
            "voice TTS provider warm failed"
        );
    }
}

async fn run_warm_job(
    generation_key: String,
    tts: Arc<dyn TextToSpeech>,
    voice_id: Option<String>,
) {
    let started_at = warm_now();
    tracing::info!(
        target: "permagentd::voice",
        event = "voice_tts_warm",
        stage = "start",
        generation_key = generation_key.as_str(),
        sentence = TTS_PROVIDER_WARM_SENTENCE,
        "voice TTS provider warm start"
    );
    let plan = crate::voice::prosody::plan(TTS_PROVIDER_WARM_SENTENCE);
    let speed = plan.speed;
    let voice_for_blocking = voice_id.clone();
    let synth =
        tokio::task::spawn_blocking(move || run_blocking_warm(tts, voice_for_blocking, speed))
            .await;
    let success = matches!(synth, Ok(Ok(_)));
    finish_warm_attempt(&generation_key, started_at, success).await;
}

/// Join an in-flight warm or start one when policy allows. Callers spawn this
/// (never await it on the socket path) — only this future awaits synthesis.
pub async fn join_tts_provider_warm(
    generation_key: String,
    tts: Arc<dyn TextToSpeech>,
    voice_id: Option<String>,
) {
    loop {
        let shared = {
            let mut reg = registry().lock().await;
            let slot =
                reg.slots
                    .entry(generation_key.clone())
                    .or_insert_with(|| GenerationWarmSlot {
                        warmed: false,
                        failed_attempts: 0,
                        last_failure_at: None,
                        in_flight: None,
                    });
            if slot.warmed {
                return;
            }
            if let Some(existing) = slot.in_flight.clone() {
                tracing::info!(
                    target: "permagentd::voice",
                    event = "voice_tts_warm",
                    stage = "join",
                    generation_key = generation_key.as_str(),
                    "voice TTS provider warm join"
                );
                drop(reg);
                existing.await;
                continue;
            }
            let now = warm_now();
            if !slot.may_start_attempt(now) {
                return;
            }
            let key = generation_key.clone();
            let tts_job = tts.clone();
            let voice_job = voice_id.clone();
            let fut = run_warm_job(key, tts_job, voice_job).boxed().shared();
            slot.in_flight = Some(fut.clone());
            fut
        };
        shared.await;
    }
}

/// Join warm using the live TTS slot and provider generation (not a session snapshot).
pub(crate) async fn warm_join_current_tts_slot(
    voice_tts: &tokio::sync::RwLock<Option<Arc<dyn TextToSpeech>>>,
    generation: fn() -> u64,
    voice_id: Option<String>,
) {
    for _ in 0..3 {
        let gen_a = generation();
        let tts = voice_tts.read().await.clone();
        let Some(tts) = tts else {
            return;
        };
        let gen_b = generation();
        if gen_a != gen_b {
            continue;
        }
        let key = generation_key(gen_b, voice_id.clone());
        join_tts_provider_warm(key, tts, voice_id).await;
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::provider::{AudioOutput, TextToSpeech, TtsConfig};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::RwLock;

    struct SlotWarmFakeTts {
        label: &'static str,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl SlotWarmFakeTts {
        fn with_calls(
            label: &'static str,
        ) -> (Arc<dyn TextToSpeech>, Arc<Mutex<Vec<&'static str>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let tts: Arc<dyn TextToSpeech> = Arc::new(Self {
                label,
                calls: calls.clone(),
            });
            (tts, calls)
        }
    }

    impl TextToSpeech for SlotWarmFakeTts {
        fn synthesize(&self, text: &str, _config: &TtsConfig) -> anyhow::Result<AudioOutput> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(self.label);
            let _ = text;
            Ok(AudioOutput {
                samples: vec![0.0; 64],
                sample_rate: 24_000,
            })
        }

        fn sample_rate(&self) -> u32 {
            24_000
        }

        fn list_voices(&self) -> Vec<String> {
            vec!["default".to_string()]
        }
    }

    #[tokio::test]
    async fn hot_swap_warm_uses_slot_provider_under_matching_generation() {
        let _serial = WARM_TEST_SERIAL.lock().await;
        reset_registry_for_test().await;
        set_test_warm_clock(Instant::now());

        static TEST_GEN: AtomicU64 = AtomicU64::new(1);
        let generation = || TEST_GEN.load(Ordering::Relaxed);

        let (tts_a, calls_a) = SlotWarmFakeTts::with_calls("A");
        let (tts_b, calls_b) = SlotWarmFakeTts::with_calls("B");
        let slot: Arc<RwLock<Option<Arc<dyn TextToSpeech>>>> =
            Arc::new(RwLock::new(Some(tts_a.clone())));

        let old_key = generation_key(generation(), None);
        join_tts_provider_warm(old_key.clone(), tts_a.clone(), None).await;
        assert_eq!(calls_a.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
        assert_eq!(calls_b.lock().unwrap_or_else(|e| e.into_inner()).len(), 0);

        *slot.write().await = Some(tts_b.clone());
        TEST_GEN.fetch_add(1, Ordering::Relaxed);
        let new_key = generation_key(generation(), None);

        warm_join_current_tts_slot(slot.as_ref(), generation, None).await;

        assert_eq!(calls_a.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
        assert_eq!(calls_b.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);

        join_tts_provider_warm(new_key.clone(), tts_a.clone(), None).await;
        assert_eq!(
            calls_a.lock().unwrap_or_else(|e| e.into_inner()).len(),
            1,
            "provider A must not warm under the new generation key"
        );
        join_tts_provider_warm(old_key, tts_a, None).await;
        assert_eq!(calls_a.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
    }
}
