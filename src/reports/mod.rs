//! History aggregation and report generation for the `report` command (architecture.md §5.19).
#![allow(dead_code)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::history::HistoryRecord;
use crate::pressure::PressureLevel;

// ---------------------------------------------------------------------------
// Report data contracts (architecture.md §5.19 "Report structs")
// ---------------------------------------------------------------------------

/// Number of recorded ticks observed at each pressure level over the window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LevelCounts {
    pub none: usize,
    pub low: usize,
    pub moderate: usize,
    pub high: usize,
    pub critical: usize,
}

/// Aggregated summary of local history over a time window.
///
/// Pure data; rendered by the CLI as human text or `--json`. An empty window
/// produces a zeroed report, never an error (architecture.md §5.19).
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub window_days: u32,
    pub since: DateTime<Utc>,
    pub record_count: usize,
    pub first_timestamp: Option<DateTime<Utc>>,
    pub last_timestamp: Option<DateTime<Utc>>,
    pub level_counts: LevelCounts,
    /// Most frequent pressure level over the window; ties break toward severity.
    pub dominant_level: Option<PressureLevel>,
    pub total_actions: usize,
    pub total_simulated: usize,
    pub total_blocked: usize,
    /// `"{ActionKind:?}:{ActionStatus:?}"` outcome counts, descending by count.
    pub outcome_counts: Vec<(String, usize)>,
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Aggregate `records` (already filtered to the window) into a [`Report`].
///
/// Pure and deterministic — tests construct `HistoryRecord`s directly. The
/// `reports` layer never decides actions (architecture.md §5.19); it only
/// summarizes. Empty input yields an empty report.
#[must_use]
pub fn report(records: &[HistoryRecord], window_days: u32, since: DateTime<Utc>) -> Report {
    let mut counts = LevelCounts::default();
    let mut total_actions = 0usize;
    let mut total_simulated = 0usize;
    let mut total_blocked = 0usize;
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();

    for r in records {
        match r.pressure_level {
            PressureLevel::None => counts.none += 1,
            PressureLevel::Low => counts.low += 1,
            PressureLevel::Moderate => counts.moderate += 1,
            PressureLevel::High => counts.high += 1,
            PressureLevel::Critical => counts.critical += 1,
        }
        total_actions += r.action_count;
        total_simulated += r.simulated_count;
        total_blocked += r.blocked_count;
        for o in &r.outcomes {
            *outcomes.entry(o.clone()).or_default() += 1;
        }
    }

    // Descending by count, then key, for deterministic output.
    let mut outcome_counts: Vec<(String, usize)> = outcomes.into_iter().collect();
    outcome_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    Report {
        window_days,
        since,
        record_count: records.len(),
        first_timestamp: records.first().map(|r| r.timestamp),
        last_timestamp: records.last().map(|r| r.timestamp),
        dominant_level: dominant_level(&counts),
        level_counts: counts,
        total_actions,
        total_simulated,
        total_blocked,
        outcome_counts,
    }
}

/// Level with the highest count. Ties break toward the more severe level
/// (`max_by_key` returns the last maximum; the array is ordered ascending by
/// severity). Returns `None` only when there are no records.
fn dominant_level(c: &LevelCounts) -> Option<PressureLevel> {
    let pairs = [
        (PressureLevel::None, c.none),
        (PressureLevel::Low, c.low),
        (PressureLevel::Moderate, c.moderate),
        (PressureLevel::High, c.high),
        (PressureLevel::Critical, c.critical),
    ];
    pairs
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .max_by_key(|(_, n)| *n)
        .map(|(level, _)| level)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn rec(
        level: PressureLevel,
        actions: usize,
        blocked: usize,
        outcomes: &[&str],
    ) -> HistoryRecord {
        HistoryRecord {
            schema_version: 1,
            timestamp: Utc::now(),
            pressure_level: level,
            psi_summary: "cpu=0.0 mem=0.0 io=0.0".to_string(),
            state: "Idle".to_string(),
            action_count: actions,
            simulated_count: actions,
            blocked_count: blocked,
            outcomes: outcomes.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn empty_history_yields_empty_report() {
        let r = report(&[], 7, Utc::now());
        assert_eq!(r.record_count, 0);
        assert_eq!(r.dominant_level, None);
        assert_eq!(r.level_counts, LevelCounts::default());
        assert_eq!(r.total_actions, 0);
        assert!(r.first_timestamp.is_none());
        assert!(r.outcome_counts.is_empty());
    }

    #[test]
    fn counts_levels_and_sums_actions() {
        let records = [
            rec(PressureLevel::None, 0, 0, &[]),
            rec(PressureLevel::Low, 1, 0, &["AdjustNice:Applied"]),
            rec(PressureLevel::Low, 2, 1, &["SetCpuWeight:Applied"]),
            rec(PressureLevel::High, 3, 2, &[]),
        ];
        let r = report(&records, 1, Utc::now());
        assert_eq!(r.record_count, 4);
        assert_eq!(r.level_counts.none, 1);
        assert_eq!(r.level_counts.low, 2);
        assert_eq!(r.level_counts.high, 1);
        assert_eq!(r.dominant_level, Some(PressureLevel::Low));
        assert_eq!(r.total_actions, 6);
        assert_eq!(r.total_blocked, 3);
    }

    #[test]
    fn dominant_level_ties_break_toward_severity() {
        // None and Critical both appear once; Critical (more severe) wins.
        let records = [
            rec(PressureLevel::None, 0, 0, &[]),
            rec(PressureLevel::Critical, 0, 0, &[]),
        ];
        let r = report(&records, 1, Utc::now());
        assert_eq!(r.dominant_level, Some(PressureLevel::Critical));
    }

    #[test]
    fn outcome_counts_sorted_descending() {
        let records = [
            rec(PressureLevel::Low, 1, 0, &["A:Applied", "B:Blocked"]),
            rec(PressureLevel::Low, 1, 0, &["A:Applied"]),
            rec(PressureLevel::Low, 1, 0, &["A:Applied", "B:Blocked"]),
        ];
        let r = report(&records, 1, Utc::now());
        assert_eq!(
            r.outcome_counts,
            vec![("A:Applied".to_string(), 3), ("B:Blocked".to_string(), 2),]
        );
    }
}
