//! Process-wide cost-routing counters, emitted once by the headless exit sink.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static DISPATCHES: AtomicU64 = AtomicU64::new(0);
static CHEAPEST_CAPABLE: AtomicU64 = AtomicU64::new(0);
static PACK_HONORED: AtomicU64 = AtomicU64::new(0);
static PLANNER: Mutex<String> = Mutex::new(String::new());

pub fn record(area: &str, key: &str) {
    if area != "cost_routing" {
        return;
    }
    let counter = match key {
        "dispatches" => &DISPATCHES,
        "cheapest_capable" => &CHEAPEST_CAPABLE,
        "pack_honored" => &PACK_HONORED,
        _ => return,
    };
    counter.fetch_add(1, Ordering::Relaxed);
    // Lane R17a: cost_routing is one of the four RECEIPT CONTRACT v1 lines
    // the receipts sidecar mirrors — refresh it on every counter change too,
    // not just the other three areas (`crate::receipts::write_sidecar` is a
    // no-op unless `PERMAGENT_RECEIPTS_SIDECAR` is set for this run).
    crate::receipts::write_sidecar();
}

pub fn set_planner(provider: &str, model: &str) {
    {
        let mut planner = PLANNER.lock().unwrap_or_else(|e| e.into_inner());
        *planner = format!("{provider}/{model}");
    }
    crate::receipts::write_sidecar();
}

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub dispatches: u64,
    pub cheapest_capable: u64,
    pub pack_honored: u64,
    pub planner: String,
}

pub fn snapshot() -> Snapshot {
    let planner = PLANNER.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Snapshot {
        dispatches: DISPATCHES.load(Ordering::Relaxed),
        cheapest_capable: CHEAPEST_CAPABLE.load(Ordering::Relaxed),
        pack_honored: PACK_HONORED.load(Ordering::Relaxed),
        planner: if planner.is_empty() {
            "unknown/unknown".into()
        } else {
            planner
        },
    }
}

impl Snapshot {
    pub fn format_line(&self) -> String {
        format!(
            "receipt cost_routing dispatches={} cheapest_capable={} pack_honored={} planner={}",
            self.dispatches, self.cheapest_capable, self.pack_honored, self.planner
        )
    }
}

pub fn format_line() -> String {
    snapshot().format_line()
}

#[cfg(test)]
mod tests {
    use super::Snapshot;

    #[test]
    fn routing_receipt_format_contract() {
        let receipt = Snapshot {
            dispatches: 3,
            cheapest_capable: 2,
            pack_honored: 3,
            planner: "provider/model".into(),
        };
        assert_eq!(
            receipt.format_line(),
            "receipt cost_routing dispatches=3 cheapest_capable=2 pack_honored=3 planner=provider/model"
        );
    }
}
