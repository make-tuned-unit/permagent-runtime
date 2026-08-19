//! What `Brain::forget` costs compared with the raw `DELETE FROM memories` it
//! replaced in the pruning and cleanup paths.
//!
//! `forget` reaches substrates a raw delete cannot — the recognition sidecar
//! lives in a separate database file, so no foreign key cascades into it — and
//! it verifies afterwards with a recall probe and a recognition probe. That
//! verification is real work, and the Librarian's pruning pass can be large, so
//! the cost is measured rather than assumed. Two corpus sizes, because what
//! matters for a big pass is whether the per-delete cost grows with the corpus.
//!
//! Ignored by default — it builds corpora and is a measurement, not a gate:
//!
//! ```text
//! cargo test -p permagent-brain-tests --test forget_cost -- --ignored --nocapture
//! ```

use spectral::{Brain, RememberOpts, Visibility};
use std::time::{Duration, Instant};

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

const SAMPLE: usize = 40;

struct Measurement {
    corpus: usize,
    seed: Duration,
    forget: Duration,
    recognize: Duration,
    raw: Duration,
}

fn measure(corpus: usize) -> Measurement {
    let temp = tempfile::tempdir().expect("tempdir");
    let brain_path = temp.path().join("brain");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ONTOLOGY_TOML).unwrap();
    let brain = Brain::builder()
        .data_dir(&brain_path)
        .ontology_path(&ontology_path)
        .device_id(spectral::DeviceId::from_descriptor("forget-cost"))
        .build()
        .expect("bench brain");

    let mut keys = Vec::with_capacity(corpus);
    let mut contents = Vec::with_capacity(corpus);
    let started = Instant::now();
    for i in 0..corpus {
        let key = format!("activity:{i}:browser_navigated:{i:08x}");
        let content =
            format!("Navigated to Example Page {i} (https://example-{i}.test/page) in tab t{i}.");
        brain
            .remember_with(
                &key,
                &content,
                RememberOpts {
                    source: Some("permagent.activity".to_string()),
                    visibility: Visibility::Private,
                    compaction_tier: Some(spectral::ingest::CompactionTier::Raw),
                    ..Default::default()
                },
            )
            .expect("seed write");
        keys.push(key);
        contents.push(content);
    }
    let seed = started.elapsed();

    // One half of the verification `forget` performs, on its own, so the report
    // can say whether verification is what costs.
    let started = Instant::now();
    for content in contents.iter().take(SAMPLE) {
        brain.recognize(content).expect("recognize");
    }
    let recognize = started.elapsed();

    let started = Instant::now();
    for key in keys.iter().take(SAMPLE) {
        let report = brain.forget(key).expect("forget");
        assert!(report.store.existed);
    }
    let forget = started.elapsed();

    let db = brain_path.join("memory.db");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let raw = rt.block_on(async {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite://{}", db.display()))
            .await
            .expect("open memory.db");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        let started = Instant::now();
        for key in keys.iter().skip(SAMPLE).take(SAMPLE) {
            sqlx::query("DELETE FROM memories WHERE key = ?")
                .bind(key)
                .execute(&pool)
                .await
                .expect("raw delete");
        }
        started.elapsed()
    });

    Measurement {
        corpus,
        seed,
        forget,
        recognize,
        raw,
    }
}

fn per_op_ms(d: Duration, n: usize) -> f64 {
    d.as_secs_f64() * 1000.0 / n as f64
}

#[test]
#[ignore = "measurement, not a gate — run with --ignored --nocapture"]
fn forget_versus_raw_delete() {
    println!(
        "{:>7} | {:>12} | {:>12} | {:>12} | {:>12} | {:>6}",
        "corpus", "write ms/op", "forget ms/op", "recog ms/op", "raw ms/op", "ratio"
    );
    for corpus in [250usize, 1_000] {
        let m = measure(corpus);
        println!(
            "{:>7} | {:>12.1} | {:>12.1} | {:>12.1} | {:>12.1} | {:>5.0}x",
            m.corpus,
            per_op_ms(m.seed, m.corpus),
            per_op_ms(m.forget, SAMPLE),
            per_op_ms(m.recognize, SAMPLE),
            per_op_ms(m.raw, SAMPLE),
            per_op_ms(m.forget, SAMPLE) / per_op_ms(m.raw, SAMPLE),
        );
    }
    println!("(unoptimized `test` profile — release is materially faster)");
}
