//! Action planner, simulator, and (v0.2+) executor; every path gates through `safety` (architecture.md §5.12, §10).
#![allow(dead_code)]

use std::collections::HashMap;

use crate::profiles::ActionRisk;

// ---------------------------------------------------------------------------
// ActionKind
// ---------------------------------------------------------------------------

/// Concrete action type (architecture.md §15).
///
/// Declaration order matches §10's risk grouping: observe → moderate → aggressive → prohibited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    // Safe — no system state change.
    Observe,
    Log,
    Report,
    Recommend,
    // Moderate — requires profile permission + allowed targets.
    CreateBackup,
    AdjustNice,
    AdjustIonice,
    SetCpuWeight,
    SetIoWeight,
    SetMemoryHigh,
    // Aggressive — requires `allow_aggressive_actions` + specific flags.
    SetMemoryMax,
    RestartService,
    StopService,
    ApplyZram,
    ApplySysctl,
}

// ---------------------------------------------------------------------------
// ActionTarget
// ---------------------------------------------------------------------------

/// What a `PlannedAction` operates on (architecture.md §15).
///
/// Carries identifying information so the safety layer can check protected lists
/// and allowlists without trusting the policy engine's own flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionTarget {
    Process { pid: u32, comm: String },
    Service { unit: String },
    System,
}

// ---------------------------------------------------------------------------
// ActionStatus
// ---------------------------------------------------------------------------

/// Lifecycle state of an action (architecture.md §15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionStatus {
    Planned,
    Simulated,
    Blocked,
    Executed,
    Failed,
    RolledBack,
}

// ---------------------------------------------------------------------------
// PlannedAction / ActionResult
// ---------------------------------------------------------------------------

/// A concrete intended change, ready for safety evaluation and execution (architecture.md §15).
#[derive(Debug, Clone)]
pub struct PlannedAction {
    pub id: u64,
    pub kind: ActionKind,
    pub risk: ActionRisk,
    pub target: ActionTarget,
    pub params: HashMap<String, String>,
    pub explanation: String,
}

/// Outcome of a planned action (architecture.md §15).
#[derive(Debug, Clone)]
pub struct ActionResult {
    pub action_id: u64,
    pub status: ActionStatus,
    pub message: String,
    pub rollback_id: Option<u64>,
}
