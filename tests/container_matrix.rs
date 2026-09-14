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
//!
//! When no engine or no binary is available the test prints why and passes, so
//! it is safe to invoke unconditionally.
#![cfg(feature = "container-tests")]

use std::path::PathBuf;
use std::process::Command;

struct Target {
    /// Container image. CI should pin by digest (`name@sha256:…`) for
    /// reproducibility; the tags here are readable defaults.
    image: &'static str,
    /// Substring matched by `LX_CONTAINER_TARGETS` (e.g. `alpine`).
    key: &'static str,
    /// Expected `lx info --json` `package_manager`.
    package_manager: &'static str,
    /// Expected `lx info --json` `package_format`.
    package_format: &'static str,
}

const TARGETS: &[Target] = &[
    Target {
        image: "docker.io/library/alpine:3.20",
        key: "alpine",
        package_manager: "apk",
        package_format: "apk",
    },
    Target {
        image: "docker.io/library/debian:bookworm-slim",
        key: "debian",
        package_manager: "apt",
        package_format: "deb",
    },
    Target {
        image: "docker.io/library/fedora:latest",
        key: "fedora",
        package_manager: "dnf",
        package_format: "rpm",
    },
    Target {
        image: "docker.io/archlinux:base",
        key: "arch",
        package_manager: "pacman",
        package_format: "arch",
    },
];

/// The container engine to drive: `$CONTAINER_ENGINE`, else the first of
/// `podman`/`docker` on PATH.
fn engine() -> Option<String> {
    if let Ok(e) = std::env::var("CONTAINER_ENGINE") {
        if !e.trim().is_empty() {
            return Some(e);
        }
    }
    ["podman", "docker"].into_iter().find_map(|e| {
        Command::new(e)
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| e.to_string())
    })
}

/// Locate the `lx` binary to mount into the containers.
fn lx_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LX_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    [
        "target/x86_64-unknown-linux-musl/release/lx",
        "target/release/lx",
        "target/debug/lx",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

fn selected_targets() -> Vec<&'static Target> {
    match std::env::var("LX_CONTAINER_TARGETS") {
        Ok(v) if !v.trim().is_empty() => {
            let wanted: Vec<String> = v.split(',').map(|s| s.trim().to_lowercase()).collect();
            TARGETS
                .iter()
                .filter(|t| wanted.iter().any(|w| t.key.contains(w.as_str())))
                .collect()
        }
        _ => TARGETS.iter().collect(),
    }
}

fn info_json(engine: &str, image: &str, lx: &std::path::Path) -> Result<serde_json::Value, String> {
    let output = Command::new(engine)
        .args([
            "run",
            "--rm",
            "--user",
            "root",
            "-v",
            &format!("{}:/usr/local/bin/lx:ro", lx.display()),
            image,
            "lx",
            "info",
            "--json",
        ])
        .output()
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
    let Some(engine) = engine() else {
        eprintln!("skipping container tests: no podman/docker on PATH (set CONTAINER_ENGINE)");
        return;
    };
    let Some(lx) = lx_binary() else {
        eprintln!(
            "skipping container tests: no lx binary (set LX_BIN, or build --target x86_64-unknown-linux-musl)"
        );
        return;
    };
    let lx = lx.canonicalize().unwrap_or(lx);
    eprintln!("container engine: {engine}; lx: {}", lx.display());

    let mut failures: Vec<String> = Vec::new();
    for t in selected_targets() {
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
