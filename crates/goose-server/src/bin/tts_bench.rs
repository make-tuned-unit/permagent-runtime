//! TTS benchmark: tests dual-onnxruntime contention hypothesis.
//! MODE A: TTS alone (baseline)
//! MODE E: TTS after sherpa-onnx STT init (dual-runtime)
//! Usage: cargo run --release --bin tts_bench

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const SAMPLE_RATE: u32 = 24000;
const RUNS: usize = 5;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let _bg_mode = args.iter().any(|a| a == "--bg");

    // If --bg, set background QoS BEFORE any ort initialization
    // This simulates launchd ProcessType=Background where ALL threads
    // (including ort's global thread pool) start at background QoS
    #[cfg(target_os = "macos")]
    if _bg_mode {
        println!("*** --bg: Setting QOS_CLASS_BACKGROUND before ort init ***");
        set_qos(0x09);
    }

    let base = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("permagent")
        .join("models")
        .join("voice");
    let model_path = base.join("kokoro-v1.0.onnx");
    let voices_path = base.join("voices-v1.0.bin");
    let stt_dir = base.join("sherpa-onnx-moonshine-tiny-en-int8");

    if !model_path.exists() || !voices_path.exists() {
        eprintln!("TTS model files not found at {:?}", base);
        std::process::exit(1);
    }

    let sentences = [
        "Hey! I'm doing great, thanks for asking.",
        "Ready to help with whatever you need tonight.",
        "How's your evening going?",
    ];

    // Shared TTS state across all modes
    println!("Loading Kokoro model (inter=4, intra=4)...");
    let session = ort::session::Session::builder()
        .unwrap()
        .with_inter_threads(4)
        .unwrap()
        .with_intra_threads(4)
        .unwrap()
        .commit_from_file(&model_path)
        .unwrap();

    let voice_data = std::fs::read(&voices_path).unwrap();
    let voices = load_npz_voice(&voice_data, "bm_lewis").unwrap();
    let g2p = misaki_rs::G2P::new(misaki_rs::Language::EnglishGB);
    let vocab = build_vocab();
    let session = Arc::new(Mutex::new(session));
    let g2p = Arc::new(Mutex::new(g2p));

    // === MODE A: TTS only (baseline, no sherpa-onnx) ===
    println!("=== MODE A: TTS only (no sherpa-onnx loaded) ===");
    {
        print_header();
        for sentence in &sentences {
            let mut rtfs = Vec::new();
            for _ in 0..RUNS {
                let rtf = run_once(sentence, &session, &g2p, &vocab, &voices);
                rtfs.push(rtf);
            }
            print_row(sentence, &rtfs);
        }
    }

    // === MODE E: Load sherpa-onnx STT first, then run TTS ===
    let has_stt = stt_dir.join("tokens.txt").exists();
    if has_stt {
        println!("\n=== MODE E: TTS WITH sherpa-onnx STT loaded (dual-runtime) ===");

        // Initialize sherpa-onnx STT (this loads onnxruntime 1.24.4 dylib)
        println!("Loading sherpa-onnx Moonshine STT (num_threads=4)...");
        let stt_start = Instant::now();
        let _stt = sherpa_onnx::OfflineRecognizer::create(&sherpa_onnx::OfflineRecognizerConfig {
            model_config: sherpa_onnx::OfflineModelConfig {
                moonshine: sherpa_onnx::OfflineMoonshineModelConfig {
                    preprocessor: Some(stt_dir.join("preprocess.onnx").to_string_lossy().into()),
                    encoder: Some(stt_dir.join("encode.int8.onnx").to_string_lossy().into()),
                    uncached_decoder: Some(
                        stt_dir
                            .join("uncached_decode.int8.onnx")
                            .to_string_lossy()
                            .into(),
                    ),
                    cached_decoder: Some(
                        stt_dir
                            .join("cached_decode.int8.onnx")
                            .to_string_lossy()
                            .into(),
                    ),
                    ..Default::default()
                },
                tokens: Some(stt_dir.join("tokens.txt").to_string_lossy().into()),
                num_threads: 4,
                ..Default::default()
            },
            ..Default::default()
        })
        .expect("Failed to create STT recognizer");
        println!("STT loaded in {}ms", stt_start.elapsed().as_millis());

        // Now load TTS (ort) — this creates a second onnxruntime instance
        println!("Loading Kokoro TTS model (inter=4, intra=4)...");
        let session = ort::session::Session::builder()
            .unwrap()
            .with_inter_threads(4)
            .unwrap()
            .with_intra_threads(4)
            .unwrap()
            .commit_from_file(&model_path)
            .unwrap();

        let voice_data = std::fs::read(&voices_path).unwrap();
        let voices = load_npz_voice(&voice_data, "bm_lewis").unwrap();
        let g2p = misaki_rs::G2P::new(misaki_rs::Language::EnglishGB);
        let vocab = build_vocab();
        let session = Arc::new(Mutex::new(session));
        let g2p = Arc::new(Mutex::new(g2p));

        print_header();
        for sentence in &sentences {
            let mut rtfs = Vec::new();
            for _ in 0..RUNS {
                let rtf = run_once(sentence, &session, &g2p, &vocab, &voices);
                rtfs.push(rtf);
            }
            print_row(sentence, &rtfs);
        }
    } else {
        println!(
            "\n=== MODE E: SKIPPED — sherpa-onnx STT models not found at {:?} ===",
            stt_dir
        );
    }

    // === MODE F: Background QoS with session created UNDER background QoS ===
    // This simulates the daemon exactly: ProcessType=Background means all threads
    // (including ort's internal thread pool) are created at background QoS.
    #[cfg(target_os = "macos")]
    {
        println!("\n=== MODE F: QOS_CLASS_BACKGROUND + fresh session (full daemon sim) ===");
        set_qos(0x09); // QOS_CLASS_BACKGROUND

        // Create a NEW ort session so its thread pool inherits background QoS
        println!("  Creating ort session under background QoS...");
        let session_bg = ort::session::Session::builder()
            .unwrap()
            .with_inter_threads(4)
            .unwrap()
            .with_intra_threads(4)
            .unwrap()
            .commit_from_file(&model_path)
            .unwrap();
        let session_bg = Arc::new(Mutex::new(session_bg));
        let g2p_bg = Arc::new(Mutex::new(misaki_rs::G2P::new(
            misaki_rs::Language::EnglishGB,
        )));

        print_header();
        for sentence in &sentences {
            let mut rtfs = Vec::new();
            for _ in 0..RUNS {
                let rtf = run_once(sentence, &session_bg, &g2p_bg, &vocab, &voices);
                rtfs.push(rtf);
            }
            print_row(sentence, &rtfs);
        }
        set_qos(0x15); // QOS_CLASS_DEFAULT — restore
    }

    // === TEST 3: Thread count check ===
    println!("\n=== TEST 3: Process thread count ===");
    #[cfg(target_os = "macos")]
    {
        let pid = std::process::id();
        println!("PID={}, checking thread count via /proc or ps...", pid);
        let output = std::process::Command::new("ps")
            .args(["-M", "-p", &pid.to_string()])
            .output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            let thread_count = text.lines().count().saturating_sub(1); // subtract header
            println!("Thread count: {}", thread_count);
            // Print first 30 lines to see thread names
            for line in text.lines().take(30) {
                println!("  {}", line);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn set_qos(qos_class: u32) {
    extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    let ret = unsafe { pthread_set_qos_class_self_np(qos_class, 0) };
    let name = match qos_class {
        0x21 => "USER_INTERACTIVE",
        0x19 => "USER_INITIATED",
        0x15 => "DEFAULT",
        0x11 => "UTILITY",
        0x09 => "BACKGROUND",
        _ => "UNKNOWN",
    };
    println!("  set QoS to {} (0x{:02x}), ret={}", name, qos_class, ret);
}

fn run_once(
    sentence: &str,
    session: &Arc<Mutex<ort::session::Session>>,
    g2p: &Arc<Mutex<misaki_rs::G2P>>,
    vocab: &std::collections::HashMap<char, i64>,
    voices: &[f32],
) -> (u128, f32, f32) {
    let t = Instant::now();

    let phonemes = {
        let g = g2p.lock().unwrap();
        let (p, _) = g.g2p(sentence).unwrap();
        p
    };
    let tokens = phonemes_to_tokens(&phonemes, vocab);
    let style_index = (tokens.len().saturating_sub(2)).min(509);
    let style = voices[style_index * 256..(style_index + 1) * 256].to_vec();

    use ort::value::Tensor;
    let token_val = Tensor::from_array(([1usize, tokens.len()], tokens)).unwrap();
    let style_val = Tensor::from_array(([1usize, style.len()], style)).unwrap();
    let speed_val = Tensor::from_array(([1usize], vec![1.0f32])).unwrap();

    let mut sess = session.lock().unwrap();
    let outputs = sess
        .run(ort::inputs![
            "tokens" => token_val,
            "style" => style_val,
            "speed" => speed_val,
        ])
        .unwrap();

    let (_shape, raw) = outputs[0].try_extract_tensor::<f32>().unwrap();
    let samples = raw.len();
    let elapsed_ms = t.elapsed().as_millis();
    let audio_dur = samples as f32 / SAMPLE_RATE as f32;
    let rtf = elapsed_ms as f32 / 1000.0 / audio_dur.max(0.01);

    (elapsed_ms, audio_dur, rtf)
}

fn print_header() {
    println!(
        "{:<50} {:>8} {:>8} {:>8} {:>8}",
        "Sentence", "avg(ms)", "avg RTF", "min RTF", "max RTF"
    );
    println!("{}", "-".repeat(90));
}

fn print_row(sentence: &str, rtfs: &[(u128, f32, f32)]) {
    let avg_ms = rtfs.iter().map(|r| r.0).sum::<u128>() / rtfs.len() as u128;
    let avg_rtf = rtfs.iter().map(|r| r.2).sum::<f32>() / rtfs.len() as f32;
    let min_rtf = rtfs.iter().map(|r| r.2).fold(f32::MAX, f32::min);
    let max_rtf = rtfs.iter().map(|r| r.2).fold(0.0f32, f32::max);
    println!(
        "{:<50} {:>8} {:>7.3}x {:>7.3}x {:>7.3}x",
        sentence.chars().take(50).collect::<String>(),
        avg_ms,
        avg_rtf,
        min_rtf,
        max_rtf
    );
}

fn build_vocab() -> std::collections::HashMap<char, i64> {
    [
        (';', 1),
        (':', 2),
        (',', 3),
        ('.', 4),
        ('!', 5),
        ('?', 6),
        ('\u{2014}', 9),
        ('\u{2026}', 10),
        ('"', 11),
        ('(', 12),
        (')', 13),
        ('\u{201C}', 14),
        ('\u{201D}', 15),
        (' ', 16),
        ('\u{0303}', 17),
        ('\u{02A3}', 18),
        ('\u{02A5}', 19),
        ('\u{02A6}', 20),
        ('\u{02A8}', 21),
        ('\u{1D5D}', 22),
        ('\u{AB67}', 23),
        ('A', 24),
        ('I', 25),
        ('O', 31),
        ('Q', 33),
        ('S', 35),
        ('T', 36),
        ('W', 39),
        ('Y', 41),
        ('\u{1D4A}', 42),
        ('a', 43),
        ('b', 44),
        ('c', 45),
        ('d', 46),
        ('e', 47),
        ('f', 48),
        ('h', 50),
        ('i', 51),
        ('j', 52),
        ('k', 53),
        ('l', 54),
        ('m', 55),
        ('n', 56),
        ('o', 57),
        ('p', 58),
        ('q', 59),
        ('r', 60),
        ('s', 61),
        ('t', 62),
        ('u', 63),
        ('v', 64),
        ('w', 65),
        ('x', 66),
        ('y', 67),
        ('z', 68),
        ('\u{0251}', 69),
        ('\u{0250}', 70),
        ('\u{0252}', 71),
        ('\u{00E6}', 72),
        ('\u{03B2}', 75),
        ('\u{0254}', 76),
        ('\u{0255}', 77),
        ('\u{00E7}', 78),
        ('\u{0256}', 80),
        ('\u{00F0}', 81),
        ('\u{02A4}', 82),
        ('\u{0259}', 83),
        ('\u{025A}', 85),
        ('\u{025B}', 86),
        ('\u{025C}', 87),
        ('\u{025F}', 90),
        ('\u{0261}', 92),
        ('\u{0265}', 99),
        ('\u{0268}', 101),
        ('\u{026A}', 102),
        ('\u{029D}', 103),
        ('\u{026F}', 110),
        ('\u{0270}', 111),
        ('\u{014B}', 112),
        ('\u{0273}', 113),
        ('\u{0272}', 114),
        ('\u{0274}', 115),
        ('\u{00F8}', 116),
        ('\u{0278}', 118),
        ('\u{03B8}', 119),
        ('\u{0153}', 120),
        ('\u{0279}', 123),
        ('\u{027E}', 125),
        ('\u{027B}', 126),
        ('\u{0281}', 128),
        ('\u{027D}', 129),
        ('\u{0282}', 130),
        ('\u{0283}', 131),
        ('\u{0288}', 132),
        ('\u{02A7}', 133),
        ('\u{028A}', 135),
        ('\u{028B}', 136),
        ('\u{028C}', 138),
        ('\u{0263}', 139),
        ('\u{0264}', 140),
        ('\u{03C7}', 142),
        ('\u{028E}', 143),
        ('\u{0292}', 147),
        ('\u{0294}', 148),
        ('\u{02C8}', 156),
        ('\u{02CC}', 157),
        ('\u{02D0}', 158),
        ('\u{02B0}', 162),
        ('\u{02B2}', 164),
        ('\u{2193}', 169),
        ('\u{2192}', 171),
        ('\u{2197}', 172),
        ('\u{2198}', 173),
        ('\u{1D7B}', 177),
    ]
    .into_iter()
    .collect()
}

fn phonemes_to_tokens(phonemes: &str, vocab: &std::collections::HashMap<char, i64>) -> Vec<i64> {
    let mut tokens = vec![0i64];
    for ch in phonemes.chars() {
        if ch == '\u{200D}' {
            continue;
        }
        if let Some(&id) = vocab.get(&ch) {
            tokens.push(id);
        }
    }
    tokens.push(0);
    if tokens.len() > 512 {
        tokens.truncate(512);
    }
    tokens
}

fn load_npz_voice(data: &[u8], voice_name: &str) -> anyhow::Result<Vec<f32>> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().trim_end_matches(".npy").to_string();
        if name != voice_name {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        if buf.len() < 10 || &buf[0..6] != b"\x93NUMPY" {
            anyhow::bail!("Invalid npy header");
        }
        let header_len = u16::from_le_bytes([buf[8], buf[9]]) as usize;
        let data_start = 10 + header_len;
        let floats: Vec<f32> = buf[data_start..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        return Ok(floats);
    }
    anyhow::bail!("Voice '{}' not found in NPZ", voice_name)
}
