//! Integration tests for rollback entry capture and listing (planning.md §7).

use std::fs;

use syswarden::config;
use syswarden::rollback::{RollbackEntry, RollbackStore};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn temp_store(suffix: &str) -> (RollbackStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("syswarden_it_rollback_{suffix}"));
    let _ = fs::remove_dir_all(&dir);
    let mut cfg = config::defaults();
    cfg.rollback.dir = dir.to_string_lossy().to_string();
    let store = RollbackStore::open(&cfg).expect("open rollback store");
    (store, dir)
}

// ---------------------------------------------------------------------------
// record + list round-trip
// ---------------------------------------------------------------------------

#[test]
fn record_and_list_round_trip() {
    let (mut store, dir) = temp_store("rt");

    let e1 = RollbackEntry::new("AdjustNice", "pid=42 comm=stress", true);
    let e2 = RollbackEntry::new("SetCpuWeight", "unit=app.service", false);
    store.record(e1.clone());
    store.record(e2.clone());

    assert_eq!(store.len(), 2);
    assert_eq!(store.list()[0].action_kind, "AdjustNice");
    assert_eq!(store.list()[1].action_kind, "SetCpuWeight");
    assert!(store.list()[0].reversible);
    assert!(!store.list()[1].reversible);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn store_persists_across_reopen() {
    let (mut store, dir) = temp_store("persist");
    store.record(RollbackEntry::new("Observe", "system", true));
    store.record(RollbackEntry::new("Log", "system", true));
    drop(store);

    let mut cfg = config::defaults();
    cfg.rollback.dir = dir.to_string_lossy().to_string();
    let store2 = RollbackStore::open(&cfg).expect("reopen");
    assert_eq!(store2.len(), 2);
    assert_eq!(store2.list()[0].action_kind, "Observe");
    assert_eq!(store2.list()[1].action_kind, "Log");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// apply refuses without valid prior state
// ---------------------------------------------------------------------------

#[test]
fn apply_refuses_scaffolding_entry() {
    let (mut store, dir) = temp_store("apply");
    let entry = RollbackEntry::new("AdjustNice", "pid=1", true);
    let id = entry.id;
    store.record(entry);

    let err = store.apply(id).unwrap_err();
    assert!(
        err.to_string().contains("no captured prior state"),
        "expected prior-state refusal; got: {err}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn apply_returns_error_for_missing_id() {
    let (store, dir) = temp_store("missing");
    let err = store.apply(u64::MAX).unwrap_err();
    assert!(
        err.to_string().contains("no entry with id="),
        "expected missing-id error; got: {err}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn apply_returns_error_for_irreversible_entry() {
    let (mut store, dir) = temp_store("irrev");
    let mut entry = RollbackEntry::new("StopService", "unit=foo.service", false);
    // Give it a non-empty prior_state so it doesn't hit the scaffolding branch first.
    entry = entry.with_prior_state(serde_json::json!({"unit": "foo.service"}));
    let id = entry.id;
    store.record(entry);

    let err = store.apply(id).unwrap_err();
    assert!(
        err.to_string().contains("irreversible"),
        "expected irreversible error; got: {err}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// keep_entries pruning
// ---------------------------------------------------------------------------

#[test]
fn prune_to_keep_entries_on_open() {
    let dir = std::env::temp_dir().join("syswarden_it_rollback_prune");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // Write 6 entries to disk directly.
    let file = dir.join("rollback.jsonl");
    let mut content = String::new();
    for i in 0u64..6 {
        let e = RollbackEntry {
            id: i,
            timestamp: chrono::Utc::now(),
            action_kind: format!("Kind{i}"),
            target: "system".to_string(),
            prior_state: serde_json::Value::Object(serde_json::Map::new()),
            reversible: true,
        };
        content.push_str(&serde_json::to_string(&e).unwrap());
        content.push('\n');
    }
    fs::write(&file, &content).unwrap();

    // Open with keep_entries=3 → oldest 3 pruned.
    let mut cfg = config::defaults();
    cfg.rollback.dir = dir.to_string_lossy().to_string();
    cfg.rollback.keep_entries = 3;
    let store = RollbackStore::open(&cfg).expect("open");

    assert_eq!(store.len(), 3);
    assert_eq!(store.list()[0].id, 3, "oldest should be pruned");
    assert_eq!(store.list()[2].id, 5);

    let _ = fs::remove_dir_all(&dir);
}
