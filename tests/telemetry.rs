use lpt_lib::telemetry::*;
use serde_json::json;
use std::path::PathBuf;

fn tmp_dir() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().to_path_buf();
    (d, p)
}

#[test]
fn disabled_is_noop() {
    let (_guard, dir) = tmp_dir();
    let t = Telemetry::with_dir(false, dir);
    t.init().unwrap();
    t.record_stage("build_initialization").unwrap();
    t.record_failure("docker_build", "boom", 1).unwrap();
    t.finalize(5).unwrap();
    assert_eq!(t.summary_json(), json!({}));
    assert!(!t.dir.join("metrics.json").exists());
}

#[test]
fn enabled_writes_logs_and_metrics() {
    let (_guard, dir) = tmp_dir();
    let t = Telemetry::with_dir(true, dir.clone());
    t.init().unwrap();
    t.record_stage("build_initialization").unwrap();
    t.record_stage_complete("architecture_amd64", "success")
        .unwrap();
    t.record_failure("architecture_build", "failed", 1).unwrap();
    t.finalize(7).unwrap();

    let stages = std::fs::read_to_string(dir.join("stages.log")).unwrap();
    assert!(stages.contains("Stage: build_initialization"));
    assert!(stages.contains("Stage complete: architecture_amd64 (success)"));

    let failures = std::fs::read_to_string(dir.join("failures.log")).unwrap();
    assert!(failures.contains("architecture_build - failed (exit code: 1)"));

    let s = t.summary_json();
    assert_eq!(s["telemetry_enabled"], true);
    assert_eq!(s["build_duration_seconds"], 7);
    assert_eq!(s["build_completed"], true);
    assert!(s["build_start_time"].as_u64().unwrap() > 0);
    assert!(s["build_end_time"].as_u64().unwrap() >= s["build_start_time"].as_u64().unwrap());
}
