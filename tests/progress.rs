// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::progress::*;
use std::path::PathBuf;

fn tmp_file() -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("lx-progress-test-{}-{n}.json", std::process::id()))
}

#[test]
fn tracks_status_and_writes_json() {
    let f = tmp_file();
    let p = Progress::new(3, "v1.0", "eza", f.clone(), false).unwrap();

    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    assert_eq!(v["total_archs"], 3);
    assert_eq!(v["pending"], 3);
    assert_eq!(v["package"], "eza");

    p.set_arch("amd64", "running").unwrap();
    p.finish_arch("amd64", Outcome::Completed).unwrap();
    p.finish_arch("arm64", Outcome::Failed).unwrap();
    p.set_arch("armhf", "running").unwrap();

    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    assert_eq!(v["completed"], 1);
    assert_eq!(v["failed"], 1);
    assert_eq!(v["running"], 1);
    assert_eq!(v["architectures"]["amd64"]["status"], "completed");
    assert_eq!(v["architectures"]["arm64"]["status"], "failed");

    p.cleanup().unwrap();
    assert!(!f.exists());
}

#[test]
fn tty_render_clears_line() {
    let f = tmp_file();
    let p = Progress::new(1, "v1.0", "eza", f.clone(), true).unwrap();
    p.finish_arch("amd64", Outcome::Completed).unwrap();
    p.cleanup().unwrap();
    assert!(!f.exists());
}
