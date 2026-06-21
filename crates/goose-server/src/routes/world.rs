//! World View strata read route.
//!
//! Endpoints:
//!   GET /api/world/strata?slices=N  — depth-stratified memory history
//!
//! Backs the "Carved Cave" descent: the cave depth is a deterministic function
//! of real memory count, and each chamber is an equal TIME-SLICE of the lived
//! history (oldest deepest → newest at the verge). This route is the single
//! source of truth for that — a read-only COUNT of real rows in Brain's
//! memory.db, no fabrication. On any read failure it returns honest zeros /
//! empty slices (mirrors the `.unwrap_or(0)` discipline in
//! [`super::henry_status`]).
//!
//! created_at in memory.db is stored in two coexisting formats (SQLite
//! `datetime('now')` → "YYYY-MM-DD HH:MM:SS" AND RFC3339). String comparison
//! across those is unsafe, so all slicing is done over epoch seconds via
//! `strftime('%s', created_at)`, which parses both.

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::state::AppState;

/// Query: number of time-slices (chambers). Clamped to 1..=6 by the curve.
#[derive(Debug, Deserialize)]
pub struct StrataQuery {
    #[serde(default = "default_slices")]
    pub slices: u32,
}

fn default_slices() -> u32 {
    1
}

#[derive(Debug, Serialize)]
pub struct Slice {
    /// Inclusive start of this slice's date range (epoch-derived, RFC3339).
    pub start: String,
    /// Exclusive end of this slice's date range (RFC3339).
    pub end: String,
    pub memory_count: usize,
    pub described_count: usize,
}

#[derive(Debug, Serialize)]
pub struct StrataResponse {
    pub total_memories: usize,
    pub described_count: usize,
    /// RFC3339 of the oldest memory, or null on an empty Brain.
    pub first_memory_at: Option<String>,
    /// N entries, oldest first. Empty on read failure / empty Brain.
    pub slices: Vec<Slice>,
}

impl StrataResponse {
    /// Honest-empty payload: returned on any read failure or 0-memory Brain.
    fn empty(total: usize, described: usize) -> Self {
        StrataResponse {
            total_memories: total,
            described_count: described,
            first_memory_at: None,
            slices: Vec::new(),
        }
    }
}

/// Split [start_secs, end_secs] into `n` contiguous, non-overlapping epoch-second
/// ranges. Returns `n` (start, end) pairs covering the whole span, oldest first.
/// The final range's end is the span end (so the newest memory at `end` lands in
/// the last slice via an inclusive upper bound at the caller).
fn slice_boundaries(start_secs: i64, end_secs: i64, n: u32) -> Vec<(i64, i64)> {
    let n = n.max(1) as i64;
    // Degenerate span (single instant or inverted): n slices all covering the instant.
    if end_secs <= start_secs {
        return vec![(start_secs, end_secs); n as usize];
    }
    let span = end_secs - start_secs;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let s = start_secs + (span * i) / n;
        let e = start_secs + (span * (i + 1)) / n;
        out.push((s, e));
    }
    out
}

/// Whether the `memories` table currently has a `description` column.
/// Old DBs predate the migration (knob 2: degrade to described_count=0).
fn has_description_column(conn: &rusqlite::Connection) -> bool {
    let mut found = false;
    if let Ok(mut stmt) = conn.prepare("PRAGMA table_info(memories)") {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) {
            for name in rows.flatten() {
                if name == "description" {
                    found = true;
                    break;
                }
            }
        }
    }
    found
}

async fn world_strata(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<StrataQuery>,
) -> Json<StrataResponse> {
    let n = params.slices.clamp(1, 6);
    Json(query_strata(n))
}

/// Core read. Mirrors `henry_status::query_brain_memory_stats`: open the
/// read-only Brain conn, COUNT real rows, never error out — zeros on failure.
fn query_strata(n: u32) -> StrataResponse {
    let conn = match crate::brain_ops::read_only_brain_conn() {
        Ok(c) => c,
        Err(_) => return StrataResponse::empty(0, 0),
    };

    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap_or(0);

    let has_desc = has_description_column(&conn);
    let described_total: usize = if has_desc {
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE description IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    } else {
        0
    };

    if total == 0 {
        return StrataResponse::empty(0, described_total);
    }

    // Oldest + newest as epoch seconds (format-agnostic via strftime).
    // MIN/MAX may be NULL on a malformed table — guard with Option.
    let first_secs: Option<i64> = conn
        .query_row(
            "SELECT CAST(strftime('%s', MIN(created_at)) AS INTEGER) FROM memories",
            [],
            |r| r.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten();
    let last_secs: Option<i64> = conn
        .query_row(
            "SELECT CAST(strftime('%s', MAX(created_at)) AS INTEGER) FROM memories",
            [],
            |r| r.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten();

    let (start_secs, end_secs) = match (first_secs, last_secs) {
        (Some(s), Some(e)) => (s, e),
        _ => return StrataResponse::empty(total, described_total),
    };

    let first_memory_at = secs_to_rfc3339(start_secs);

    let boundaries = slice_boundaries(start_secs, end_secs, n);
    let last_idx = boundaries.len().saturating_sub(1);
    let mut slices = Vec::with_capacity(boundaries.len());

    for (i, (s, e)) in boundaries.iter().enumerate() {
        // Inclusive lower bound; exclusive upper EXCEPT the final slice, which
        // is inclusive so the newest memory (created_at == end_secs) is counted.
        let count: usize = if i == last_idx {
            conn.query_row(
                "SELECT COUNT(*) FROM memories \
                 WHERE CAST(strftime('%s', created_at) AS INTEGER) >= ?1 \
                   AND CAST(strftime('%s', created_at) AS INTEGER) <= ?2",
                rusqlite::params![s, e],
                |r| r.get(0),
            )
            .unwrap_or(0)
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM memories \
                 WHERE CAST(strftime('%s', created_at) AS INTEGER) >= ?1 \
                   AND CAST(strftime('%s', created_at) AS INTEGER) < ?2",
                rusqlite::params![s, e],
                |r| r.get(0),
            )
            .unwrap_or(0)
        };

        let described: usize = if has_desc {
            let upper_op = if i == last_idx { "<=" } else { "<" };
            let sql = format!(
                "SELECT COUNT(*) FROM memories \
                 WHERE description IS NOT NULL \
                   AND CAST(strftime('%s', created_at) AS INTEGER) >= ?1 \
                   AND CAST(strftime('%s', created_at) AS INTEGER) {upper_op} ?2"
            );
            conn.query_row(&sql, rusqlite::params![s, e], |r| r.get(0))
                .unwrap_or(0)
        } else {
            0
        };

        slices.push(Slice {
            start: secs_to_rfc3339(*s),
            end: secs_to_rfc3339(*e),
            memory_count: count,
            described_count: described,
        });
    }

    StrataResponse {
        total_memories: total,
        described_count: described_total,
        first_memory_at: Some(first_memory_at),
        slices,
    }
}

/// Epoch seconds → RFC3339 (UTC). Falls back to the epoch on an out-of-range
/// value rather than panicking.
fn secs_to_rfc3339(secs: i64) -> String {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(secs, 0)
        .single()
        .unwrap_or_else(|| chrono::Utc.timestamp_opt(0, 0).unwrap())
        .to_rfc3339()
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/world/strata", get(world_strata))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_are_contiguous_non_overlapping_and_cover_span() {
        // A known 5-slice split over [1000, 6000].
        let start = 1000i64;
        let end = 6000i64;
        let n = 5u32;
        let b = slice_boundaries(start, end, n);

        assert_eq!(b.len(), n as usize, "exactly N slices");
        // First starts at span start, last ends at span end.
        assert_eq!(
            b.first().unwrap().0,
            start,
            "first slice starts at span start"
        );
        assert_eq!(b.last().unwrap().1, end, "last slice ends at span end");

        // Contiguous + non-overlapping: each slice's end == next slice's start.
        for w in b.windows(2) {
            assert_eq!(w[0].1, w[1].0, "slice end meets next slice start exactly");
            assert!(w[0].0 < w[0].1, "each slice spans forward");
        }
        // Full coverage: sum of slice widths == total span.
        let covered: i64 = b.iter().map(|(s, e)| e - s).sum();
        assert_eq!(covered, end - start, "slices cover the whole span");
    }

    #[test]
    fn single_slice_covers_entire_span() {
        let b = slice_boundaries(0, 100, 1);
        assert_eq!(b, vec![(0, 100)]);
    }

    #[test]
    fn degenerate_span_does_not_panic_or_divide_by_zero() {
        // Empty/near-empty Brain: first == last (single instant).
        let b = slice_boundaries(500, 500, 3);
        assert_eq!(b.len(), 3);
        for (s, e) in b {
            assert_eq!(s, 500);
            assert_eq!(e, 500);
        }
        // Inverted is also tolerated (returns the instant), never panics.
        let b2 = slice_boundaries(900, 100, 2);
        assert_eq!(b2.len(), 2);
    }

    #[test]
    fn boundaries_are_monotonic_for_uneven_division() {
        // 7 over a span not divisible by 7 — still monotonic, still covers.
        let b = slice_boundaries(0, 1457, 7);
        assert_eq!(b.len(), 7);
        assert_eq!(b.first().unwrap().0, 0);
        assert_eq!(b.last().unwrap().1, 1457);
        for w in b.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
    }
}
