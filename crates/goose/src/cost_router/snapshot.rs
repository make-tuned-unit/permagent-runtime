//! Routing snapshot — the receipt on a goal card / Build meter.
//!
//! Written when escalation or hold runs. Signals never swap the main loop;
//! this is what the UI and the agent read.

use serde::{Deserialize, Serialize};

use super::tool_signals::ToolTranscriptSignals;

pub const ROUTING_SNAPSHOT_KEY: &str = "routing_snapshot";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RoutingSnapshot {
    /// One prose sentence. Empty ⇒ UI hides the line.
    pub note: String,
    #[serde(default)]
    pub signals: ToolTranscriptSignals,
}

impl RoutingSnapshot {
    pub fn from_signals(signals: &ToolTranscriptSignals, extra: Option<&str>) -> Self {
        let note = extra
            .map(str::to_string)
            .or_else(|| signals.prose())
            .unwrap_or_default();
        Self {
            note,
            signals: signals.clone(),
        }
    }

    pub fn from_metadata(meta: &serde_json::Map<String, serde_json::Value>) -> Option<Self> {
        meta.get(ROUTING_SNAPSHOT_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn write_into(&self, meta: &mut serde_json::Map<String, serde_json::Value>) {
        if let Ok(v) = serde_json::to_value(self) {
            meta.insert(ROUTING_SNAPSHOT_KEY.to_string(), v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let snap = RoutingSnapshot::from_signals(
            &ToolTranscriptSignals {
                spinning: 0.7,
                ..Default::default()
            },
            None,
        );
        let mut map = serde_json::Map::new();
        snap.write_into(&mut map);
        let back = RoutingSnapshot::from_metadata(&map).unwrap();
        assert!(back.note.contains("stuck"));
    }
}
