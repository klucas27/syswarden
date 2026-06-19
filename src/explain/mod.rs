//! Human-readable `Explanation` builder from decisions, actions, and metrics (architecture.md §5.18, §21).
//!
//! Phase 14 stub: single-line summary. Phase 20 adds structured, user-facing explanations.
#![allow(dead_code)]

use crate::actions::{ActionResult, ActionStatus};
use crate::policy::PolicyDecision;
use crate::pressure::{PressureSnapshot, SystemState};

// ---------------------------------------------------------------------------
// Explanation
// ---------------------------------------------------------------------------

/// Human-readable explanation for one supervision tick (architecture.md §5.18).
pub struct Explanation {
    pub summary: String,
}

// ---------------------------------------------------------------------------
// build
// ---------------------------------------------------------------------------

/// Build a one-line [`Explanation`] from one tick's outputs.
#[must_use]
pub fn build(
    state: SystemState,
    pressure: &PressureSnapshot,
    decision: &PolicyDecision,
    results: &[ActionResult],
) -> Explanation {
    let simulated = results
        .iter()
        .filter(|r| matches!(r.status, ActionStatus::Simulated))
        .count();
    let blocked = results
        .iter()
        .filter(|r| matches!(r.status, ActionStatus::Blocked))
        .count();
    let summary = format!(
        "state={state:?} pressure={:?} intent={:?} actions={} (simulated={simulated} blocked={blocked}) — {}",
        pressure.level,
        decision.intent,
        results.len(),
        decision.rationale,
    );
    Explanation { summary }
}
