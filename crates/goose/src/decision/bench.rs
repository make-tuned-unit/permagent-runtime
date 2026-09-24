//! B4 bench arm: latency, accuracy and flip rate of a judge backend on PUBLIC
//! fixtures (`bench_fixtures.json`, synthetic, no user data).
//!
//! Every item is asked twice through [`decide_guarded`], once with its options
//! in fixture order and once reversed. Accuracy counts every answered call;
//! the flip rate is the share of items answered both ways whose choice
//! changed with the order alone, the order sensitivity the DAG says to
//! measure rather than assume. Errors are counted, never retried or scored.
//!
//! Live run (local endpoints only; a hosted judge is paid and belongs on the
//! metered path, which this bench does not wire):
//! `PERMAGENT_BENCH_JUDGE_ENDPOINT=http://127.0.0.1:8765/v1/systemone \
//!  cargo test -p permagent --lib -- --ignored --nocapture decision::bench::tests::live_bench`
use super::{decide_guarded, Answer, DecisionBackend, DecisionState, Question, StateClass};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Deserialize)]
pub struct BenchItem {
    pub id: String,
    pub state: String,
    pub instructions: String,
    pub options: Vec<(String, String)>,
    pub expected: String,
}

#[derive(Deserialize)]
struct FixtureFile {
    items: Vec<BenchItem>,
}

/// The bundled public fixtures.
pub fn fixtures() -> anyhow::Result<Vec<BenchItem>> {
    let file: FixtureFile = serde_json::from_str(include_str!("bench_fixtures.json"))?;
    for item in &file.items {
        anyhow::ensure!(
            item.options.iter().any(|(key, _)| key == &item.expected),
            "fixture {} expects an option it does not offer",
            item.id
        );
    }
    Ok(file.items)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchReport {
    pub backend: String,
    pub model: Option<String>,
    pub items: usize,
    pub calls: usize,
    pub answered: usize,
    pub errors: usize,
    pub correct: usize,
    /// correct / answered; `None` when nothing was answered.
    pub accuracy: Option<f64>,
    /// Items answered in both orders.
    pub paired: usize,
    pub flips: usize,
    /// flips / paired; `None` when no item was answered both ways.
    pub flip_rate: Option<f64>,
    pub p50_ms: Option<u128>,
    pub p95_ms: Option<u128>,
    /// Item ids whose choice changed with option order.
    pub flipped_items: Vec<String>,
    /// Item ids answered wrongly in at least one order.
    pub wrong_items: Vec<String>,
    /// First error text per failing item (the judge's error, never state text).
    pub error_samples: Vec<String>,
}

fn question(item: &BenchItem, reversed: bool) -> anyhow::Result<Question> {
    let mut builder = Question::choice(item.id.clone(), item.instructions.clone());
    let mut options = item.options.clone();
    if reversed {
        options.reverse();
    }
    for (key, description) in options {
        builder = builder.option(key, description);
    }
    Ok(builder.exhaustive().build()?)
}

/// Nearest-rank percentile over sorted samples.
fn percentile(sorted: &[Duration], pct: usize) -> Option<u128> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (pct * sorted.len()).div_ceil(100).max(1);
    Some(sorted[rank - 1].as_millis())
}

pub async fn run(
    backend: &dyn DecisionBackend,
    items: &[BenchItem],
) -> anyhow::Result<BenchReport> {
    let mut latencies = Vec::new();
    let (mut answered, mut errors, mut correct, mut paired, mut flips) = (0, 0, 0, 0, 0);
    let (mut flipped_items, mut wrong_items, mut error_samples) =
        (Vec::new(), Vec::new(), Vec::new());
    for item in items {
        let state = DecisionState {
            text: item.state.clone(),
            class: StateClass::Public,
        };
        let mut choices = Vec::with_capacity(2);
        let mut wrong = false;
        for reversed in [false, true] {
            let q = question(item, reversed)?;
            let started = Instant::now();
            let result = decide_guarded(backend, &state, std::slice::from_ref(&q)).await;
            match result {
                Ok(answers) => {
                    latencies.push(started.elapsed());
                    answered += 1;
                    let choice = match answers.into_iter().next() {
                        Some(Answer::Choice { choice, .. }) => choice,
                        other => anyhow::bail!("a choice question got {other:?}"),
                    };
                    if choice == item.expected {
                        correct += 1;
                    } else {
                        wrong = true;
                    }
                    choices.push(choice);
                }
                Err(error) => {
                    errors += 1;
                    if error_samples.len() < 5 {
                        error_samples.push(format!("{}: {error}", item.id));
                    }
                }
            }
        }
        if wrong {
            wrong_items.push(item.id.clone());
        }
        if let [forward, backward] = choices.as_slice() {
            paired += 1;
            if forward != backward {
                flips += 1;
                flipped_items.push(item.id.clone());
            }
        }
    }
    latencies.sort();
    let ratio = |n: usize, d: usize| (d > 0).then(|| n as f64 / d as f64);
    Ok(BenchReport {
        backend: backend.name().to_string(),
        model: backend.model_name().map(str::to_owned),
        items: items.len(),
        calls: items.len() * 2,
        answered,
        errors,
        correct,
        accuracy: ratio(correct, answered),
        paired,
        flips,
        flip_rate: ratio(flips, paired),
        p50_ms: percentile(&latencies, 50),
        p95_ms: percentile(&latencies, 95),
        flipped_items,
        wrong_items,
        error_samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{DecisionError, DecisionResponse, DecisionUsage};
    use crate::sovereignty::DataLocality;
    use std::collections::BTreeMap;

    /// Answers by rule: `first` picks the first option sent (maximally order
    /// sensitive), `expected` answers every fixture correctly, `fail` errors.
    struct Scripted(&'static str, Vec<BenchItem>);

    #[async_trait::async_trait]
    impl DecisionBackend for Scripted {
        fn name(&self) -> &str {
            self.0
        }
        fn data_locality(&self) -> DataLocality {
            DataLocality::Local
        }
        async fn decide_with_usage(
            &self,
            _state: &DecisionState,
            questions: &[Question],
        ) -> Result<DecisionResponse, DecisionError> {
            if self.0 == "fail" {
                return Err(DecisionError::Unavailable);
            }
            let answers = questions
                .iter()
                .map(|q| {
                    let crate::decision::QuestionKind::Choice { options } = q.kind() else {
                        unreachable!()
                    };
                    let choice = if self.0 == "first" {
                        options[0].0.clone()
                    } else {
                        self.1
                            .iter()
                            .find(|i| i.id == q.id())
                            .unwrap()
                            .expected
                            .clone()
                    };
                    let probabilities: BTreeMap<String, f64> = options
                        .iter()
                        .map(|(k, _)| (k.clone(), if *k == choice { 1.0 } else { 0.0 }))
                        .collect();
                    Answer::Choice {
                        choice,
                        probabilities,
                        confidence: 1.0,
                    }
                })
                .collect();
            Ok(DecisionResponse {
                answers,
                usage: DecisionUsage { input_tokens: None },
            })
        }
    }

    #[test]
    fn fixtures_are_public_well_formed_and_unambiguous() {
        let items = fixtures().unwrap();
        assert!(items.len() >= 30, "{}", items.len());
        let mut ids = std::collections::HashSet::new();
        for item in &items {
            assert!(ids.insert(&item.id), "duplicate id {}", item.id);
            assert!(item.options.len() >= 2, "{}", item.id);
            question(item, false).unwrap();
            question(item, true).unwrap();
        }
    }

    #[tokio::test]
    async fn a_perfect_judge_scores_one_with_no_flips() {
        let items = fixtures().unwrap();
        let report = run(&Scripted("expected", items.clone()), &items)
            .await
            .unwrap();
        assert_eq!(report.calls, items.len() * 2);
        assert_eq!(report.answered, report.calls);
        assert_eq!(report.accuracy, Some(1.0));
        assert_eq!(report.flip_rate, Some(0.0));
        assert!(report.p50_ms.is_some() && report.p95_ms.is_some());
        assert!(report.wrong_items.is_empty());
    }

    /// A judge that always takes the first option sent flips on every item,
    /// because the second call sends the options reversed.
    #[tokio::test]
    async fn an_order_following_judge_flips_on_every_item() {
        let items = fixtures().unwrap();
        let report = run(&Scripted("first", items.clone()), &items)
            .await
            .unwrap();
        assert_eq!(report.paired, items.len());
        assert_eq!(report.flips, items.len());
        assert_eq!(report.flip_rate, Some(1.0));
        // Exactly one of the two orders puts the expected key first, or neither.
        assert!(report.accuracy.unwrap() < 1.0);
    }

    #[tokio::test]
    async fn errors_are_counted_not_scored() {
        let items = fixtures().unwrap();
        let report = run(&Scripted("fail", items.clone()), &items).await.unwrap();
        assert_eq!(report.errors, items.len() * 2);
        assert_eq!(report.answered, 0);
        assert_eq!(report.accuracy, None);
        assert_eq!(report.flip_rate, None);
        assert_eq!(report.p50_ms, None);
        assert_eq!(report.error_samples.len(), 5);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let ms = |v: &[u64]| {
            v.iter()
                .map(|m| Duration::from_millis(*m))
                .collect::<Vec<_>>()
        };
        assert_eq!(percentile(&ms(&[10, 20, 30, 40]), 50), Some(20));
        assert_eq!(percentile(&ms(&[10, 20, 30, 40]), 95), Some(40));
        assert_eq!(percentile(&ms(&[7]), 95), Some(7));
        assert_eq!(percentile(&[], 50), None);
    }

    /// Prints the B4 report for a real local judge. Ignored by default: it
    /// needs a server on loopback (see the module doc).
    #[tokio::test]
    #[ignore]
    async fn live_bench_against_a_local_endpoint() {
        use crate::decision::{
            endpoint_locality, HttpTimeouts, JudgeBilling, SystemOneHttpBackend,
        };
        let endpoint = std::env::var("PERMAGENT_BENCH_JUDGE_ENDPOINT")
            .expect("set PERMAGENT_BENCH_JUDGE_ENDPOINT to a local /v1/systemone URL");
        assert_eq!(
            endpoint_locality(&endpoint),
            DataLocality::Local,
            "the bench runs against local endpoints only"
        );
        let model = std::env::var("PERMAGENT_BENCH_JUDGE_MODEL").unwrap_or_else(|_| "local".into());
        let backend = SystemOneHttpBackend::new(
            endpoint,
            model,
            String::new(),
            HttpTimeouts {
                connect: Duration::from_secs(5),
                total: Duration::from_secs(30),
            },
        )
        .unwrap()
        .with_billing(JudgeBilling::LocalFree);
        let items = fixtures().unwrap();
        let report = run(&backend, &items).await.unwrap();
        println!(
            "B4_REPORT {}",
            serde_json::to_string_pretty(&report).unwrap()
        );
        assert!(
            report.answered > 0,
            "the judge answered nothing: {:?}",
            report.error_samples
        );
    }
}
