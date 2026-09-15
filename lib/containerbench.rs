// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared container-harness scaffolding for `lx`'s distro matrix.
//!
//! `lx` is container-free by design; containers appear only in tests and
//! benchmarks that need package managers the build host lacks. This module
//! owns the bits every such harness reuses so they don't diverge:
//!
//! * the distro matrix ([`TARGETS`]) — image, short key, expected package
//!   manager/format, and the default architectures lx builds for there;
//! * detecting the container engine ([`engine`]);
//! * locating the `lx` binary to mount ([`lx_binary`]);
//! * filtering to a subset of targets ([`selected_targets`]);
//! * running an `lx` command inside a container and capturing its output
//!   ([`run_lx`]).
//!
//! Both the functional container matrix
//! ([`crate::tests::container_matrix`]) and the distro build benchmark
//! ([`crate::containerbench::bench`]) build on this — adding a distro or an
//! architecture is a single edit in [`TARGETS`].

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

/// One distro cell of the matrix.
#[derive(Debug, Clone)]
pub struct ContainerTarget {
    /// Container image. CI should pin by digest (`name@sha256:…`) for
    /// reproducibility; the tags here are readable defaults.
    pub image: &'static str,
    /// Substring matched by `$LX_CONTAINER_TARGETS` (e.g. "alpine").
    pub key: &'static str,
    /// Expected `lx info --json` `package_manager`.
    pub package_manager: &'static str,
    /// Expected `lx info --json` `package_format`.
    pub package_format: &'static str,
}

/// The distro matrix. Order is deliberate: put the fastest/lightest images
/// first so a partial run still covers the common cases.
pub const TARGETS: &[ContainerTarget] = &[
    ContainerTarget {
        image: "docker.io/library/alpine:3.20",
        key: "alpine",
        package_manager: "apk",
        package_format: "apk",
    },
    ContainerTarget {
        image: "docker.io/library/debian:bookworm-slim",
        key: "debian",
        package_manager: "apt",
        package_format: "deb",
    },
    ContainerTarget {
        image: "docker.io/library/fedora:latest",
        key: "fedora",
        package_manager: "dnf",
        package_format: "rpm",
    },
    ContainerTarget {
        image: "docker.io/library/archlinux:base",
        key: "arch",
        package_manager: "pacman",
        package_format: "arch",
    },
];

/// The container engine to drive: `$CONTAINER_ENGINE`, else the first of
/// `podman`/`docker` on PATH.
pub fn engine() -> Option<String> {
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

/// Locate the `lx` binary to mount into the containers. A **musl** build is
/// required to run inside Alpine; a glibc build is fine for the others.
pub fn lx_binary() -> Option<PathBuf> {
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

/// Restrict the matrix to `$LX_CONTAINER_TARGETS` (comma-separated keys) when
/// set; otherwise run every target.
pub fn selected_targets() -> Vec<&'static ContainerTarget> {
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

/// Run `lx <args>` inside `image` (mounting `lx` at `/usr/local/bin/lx`) and
/// return the child's output. The engine is whatever [`engine`] returned.
pub fn run_lx(engine: &str, image: &str, lx: &Path, args: &[&str]) -> Result<std::process::Output> {
    let mut cmd = Command::new(engine);
    cmd.args(["run", "--rm", "--user", "root", "-v"]);
    cmd.arg(format!("{}:/usr/local/bin/lx:ro", lx.display()));
    cmd.arg(image);
    cmd.args(args);
    cmd.output()
        .map_err(|e| anyhow::anyhow!("could not run '{engine}': {e}"))
}
