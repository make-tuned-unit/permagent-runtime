//! Native storage health scanner.
//!
//! Deterministic filesystem scanning for cleanup opportunities. No shell
//! commands, no agent involvement in data production. The agent narrates
//! results; it doesn't produce them.

pub mod safety;
pub mod scanner;
pub mod size;

use crate::events;
use scanner::{CategoryStats, ScanFinding, ScanResult};
use std::collections::HashMap;
use uuid::Uuid;

/// Run a full storage scan across all categories.
/// Emits progress events on the event bus for each category.
pub async fn run_scan() -> ScanResult {
    let run_id = Uuid::now_v7().to_string();
    let mut all_findings: Vec<ScanFinding> = Vec::new();
    let mut categories: HashMap<String, CategoryStats> = HashMap::new();
    let mut counter: u32 = 0;

    // Emit scan started
    events::emit(events::PermagentEvent::new(
        events::PermagentEventType::Activity,
        serde_json::json!({
            "channel": "storage_scan",
            "event": { "type": "scan_started", "run_id": &run_id }
        }),
    ));

    // Category 1: Dev caches
    let dev = tokio::task::spawn_blocking(move || {
        let mut c = counter;
        let f = scanner::scan_dev_caches(&mut c);
        (f, c)
    })
    .await
    .unwrap_or_default();
    counter = dev.1;
    emit_category_complete("Dev Caches", &dev.0, &run_id);
    record_category(&mut categories, "dev_cache", &dev.0);
    all_findings.extend(dev.0);

    // Category 2: App caches
    let app = tokio::task::spawn_blocking(move || {
        let mut c = counter;
        let f = scanner::scan_app_caches(&mut c);
        (f, c)
    })
    .await
    .unwrap_or_default();
    counter = app.1;
    emit_category_complete("App Caches", &app.0, &run_id);
    record_category(&mut categories, "app_cache", &app.0);
    all_findings.extend(app.0);

    // Category 3: Xcode DerivedData
    let xcode = tokio::task::spawn_blocking(move || {
        let mut c = counter;
        let f = scanner::scan_xcode_derived(&mut c);
        (f, c)
    })
    .await
    .unwrap_or_default();
    counter = xcode.1;
    emit_category_complete("Build Artifacts", &xcode.0, &run_id);
    record_category(&mut categories, "build_artifact", &xcode.0);
    all_findings.extend(xcode.0);

    // Category 4: Stale downloads
    let stale = tokio::task::spawn_blocking(move || {
        let mut c = counter;
        let f = scanner::scan_stale_downloads(&mut c);
        (f, c)
    })
    .await
    .unwrap_or_default();
    counter = stale.1;
    emit_category_complete("Old Downloads", &stale.0, &run_id);
    record_category(&mut categories, "stale_file", &stale.0);
    all_findings.extend(stale.0);

    // Category 5: Large user files
    let large = tokio::task::spawn_blocking(move || {
        let mut c = counter;
        let f = scanner::scan_large_user_files(&mut c);
        (f, c)
    })
    .await
    .unwrap_or_default();
    let _ = counter; // suppress unused warning
    emit_category_complete("Large User Files", &large.0, &run_id);
    record_category(&mut categories, "large_file", &large.0);
    all_findings.extend(large.0);

    let total_bytes: u64 = all_findings.iter().map(|f| f.size_bytes).sum();

    // Emit scan complete
    events::emit(events::PermagentEvent::new(
        events::PermagentEventType::Activity,
        serde_json::json!({
            "channel": "storage_scan",
            "event": {
                "type": "scan_complete",
                "run_id": &run_id,
                "total_findings": all_findings.len(),
                "total_bytes": total_bytes,
            }
        }),
    ));

    ScanResult {
        run_id,
        findings: all_findings,
        categories,
        total_bytes,
    }
}

fn emit_category_complete(label: &str, findings: &[ScanFinding], run_id: &str) {
    let total: u64 = findings.iter().map(|f| f.size_bytes).sum();
    events::emit(events::PermagentEvent::new(
        events::PermagentEventType::Activity,
        serde_json::json!({
            "channel": "storage_scan",
            "event": {
                "type": "category_complete",
                "run_id": run_id,
                "category": label,
                "count": findings.len(),
                "total_bytes": total,
            }
        }),
    ));
}

fn record_category(map: &mut HashMap<String, CategoryStats>, key: &str, findings: &[ScanFinding]) {
    let total: u64 = findings.iter().map(|f| f.size_bytes).sum();
    map.insert(
        key.to_string(),
        CategoryStats {
            count: findings.len() as u64,
            total_bytes: total,
        },
    );
}
