// SPDX-License-Identifier: GPL-3.0-or-later

//! Container-matrix integration tests: run the statically linked `lx` inside
//! real distro containers and assert its host detection.
//!
//! `lx` itself is container-free by design — containers appear only here, in
//! the harness, to obtain package managers we don't have on the build host
//! (Alpine's `apk`, Fedora's `dnf`, Arch's `pacman`). The whole file is behind
//! the `container-tests` feature, so `cargo test` on a bare host never needs a
//! container engine.
//!
//! The distro matrix, engine detection, `lx`-binary location and the
//! "run lx in a container" primitive all live in
//! [`lx_lib::containerbench`]; this test just drives the matrix and asserts.
//! The distro build benchmark reuses the same module.
//!
//! Requirements:
//! * a container engine — `$CONTAINER_ENGINE`, else `podman`/`docker` on PATH
//! * an `lx` binary — `$LX_BIN`, else `target/x86_64-unknown-linux-musl/release/lx`,
//!   `target/release/lx`, or `target/debug/lx`. A **musl** build is required to
//!   run inside Alpine; a glibc build is fine for the others.
//!
//! Run (all targets):
//! `cargo test --features container-tests --test container_matrix -- --nocapture`
//!
//! Run one target (useful for a CI matrix):
//! `LX_CONTAINER_TARGETS=alpine cargo test --features container-tests --test container_matrix`
#![cfg(feature = "container-tests")]

use lx_lib::containerbench;

/// Run `lx info --json` inside `image` and parse the resulting [`serde_json::Value`].
fn info_json(engine: &str, image: &str, lx: &std::path::Path) -> Result<serde_json::Value, String> {
    let output = containerbench::run_lx(engine, image, lx, &["info", "--json"])
        .map_err(|e| format!("could not run '{engine}': {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`lx info --json` failed ({}) in {image}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim())
        .map_err(|e| format!("could not parse `lx info --json` output: {e}\n{stdout}"))
}

#[test]
fn lx_runs_and_detects_the_host_format_in_every_distro() {
    let Some(engine) = containerbench::engine() else {
        eprintln!("skipping container tests: no podman/docker on PATH (set CONTAINER_ENGINE)");
        return;
    };
    let Some(lx) = containerbench::lx_binary() else {
        eprintln!(
            "skipping container tests: no lx binary (set LX_BIN, or build --target x86_64-unknown-linux-musl)"
        );
        return;
    };
    let lx = lx.canonicalize().unwrap_or(lx);
    eprintln!("container engine: {engine}; lx: {}", lx.display());

    let mut failures: Vec<String> = Vec::new();
    for t in containerbench::selected_targets() {
        match info_json(&engine, t.image, &lx) {
            Ok(info) => {
                let mgr = info["package_manager"].as_str().unwrap_or("<missing>");
                let fmt = info["package_format"].as_str().unwrap_or("<missing>");
                if mgr != t.package_manager || fmt != t.package_format {
                    failures.push(format!(
                        "{}: expected {}/{} (manager/format), got {mgr}/{fmt}\n{}",
                        t.image, t.package_manager, t.package_format, info
                    ));
                } else {
                    println!("✓ {} → {mgr}/{fmt}", t.image);
                }
            }
            Err(e) => failures.push(format!("{}: {e}", t.image)),
        }
    }
    assert!(
        failures.is_empty(),
        "container matrix failures:\n{}",
        failures.join("\n\n")
    );
}
