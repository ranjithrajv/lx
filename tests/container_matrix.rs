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
    let output = containerbench::run_lx(engine, image, lx, &["info", "--json"], None, None)
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

#[test]
fn lx_builds_a_repo_whose_index_the_native_manager_accepts() {
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
        match validate_one_repo(&engine, &lx, t) {
            Ok(()) => println!("✓ {} repo validated by native manager", t.key),
            Err(e) => failures.push(format!("{}: {e:#}", t.key)),
        }
    }
    assert!(
        failures.is_empty(),
        "repo validation failures:\n{}",
        failures.join("\n\n")
    );
}

/// Build a small package inside the target's container, generate a repository
/// index with `lx repo`, then hand that index to the distro's native package
/// manager to confirm it parses and serves the package.
fn validate_one_repo(engine: &str, lx: &Path, target: &containerbench::ContainerTarget) -> Result<()> {
    use std::io::Write;
    let work = std::env::temp_dir().join(format!("lx-repo-{}", target.key));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("payload"))?;
    writeln!(work.join("payload/hello.txt"), "repo validation payload")?;
    // An ELF-ish binary so the build's binary-staging path is exercised too.
    let bin_src = if std::path::Path::new("/bin/true").exists() {
        "/bin/true"
    } else {
        lx.to_str().unwrap_or("")
    };
    if !bin_src.is_empty() {
        let _ = std::fs::copy(bin_src, work.join("payload/mybinary"));
    }
    let work_s = work.to_string_lossy();

    // 1. Build the package in its native format, writing artifacts under /work.
    let build_args: Vec<&str> = vec![
        "lx",
        "build",
        "--from-dir",
        "/payload",
        "--package-name",
        "bench",
        "--version",
        "1.0.0",
        "--format",
        target.package_format,
        "--host",
        "--output",
        "/work",
    ];
    let build = containerbench::run_lx(
        engine,
        target.image,
        lx,
        &build_args,
        Some(&work),
        Some(&work),
    )?;
    if !build.status.success() {
        bail!(
            "lx build failed ({}): {}",
            build.status,
            String::from_utf8_lossy(&build.stderr)
        );
    }

    // 2. Generate the repository index in place.
    let repo = containerbench::run_lx(
        engine,
        target.image,
        lx,
        &["lx", "repo", "/work", "--suite", "test", "--origin", "test"],
        None,
        None,
    )?;
    if !repo.status.success() {
        bail!(
            "lx repo failed ({}): {}",
            repo.status,
            String::from_utf8_lossy(&repo.stderr)
        );
    }

    // 3. Validate the generated index with the native package manager.
    containerbench::validate_repo(target, "/work")
}
