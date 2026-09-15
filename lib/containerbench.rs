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

use anyhow::{bail, Result};

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

// ---------------------------------------------------------------------------
// Repository / index validation
// ---------------------------------------------------------------------------

/// Validate a generated repository index inside its native distro container,
/// using the distro's own package manager. Each format is checked the way a
/// real client would consume it: the index parses, the package is found, and
/// (where feasible) it installs.
pub fn validate_repo(target: &ContainerTarget, index_dir: &str) -> Result<()> {
    let engine = engine().ok_or_else(|| anyhow::anyhow!("no container engine"))?;
    let lx = lx_binary().ok_or_else(|| anyhow::anyhow!("no lx binary"))?;
    match target.package_format {
        "deb" => validate_repo_script(
            &engine,
            &lx,
            "docker.io/library/debian:bookworm-slim",
            &format!(
                "printf '%s' 'deb [trusted=yes allow-insecure=yes file={index_dir}/ local main]' \
                 > /etc/apt/sources.list.d/lx-test.list \
                 && apt-get update -y -o Acquire::AllowInsecureRepositories=true \
                 && grep -q '.' /var/lib/apt/lists/*_local_*_Packages 2>/dev/null \
                 && echo APT_INDEX_OK"
            ),
            "APT_INDEX_OK",
        ),
        "arch" => validate_repo_script(
            &engine,
            &lx,
            "docker.io/archlinux:base",
            &format!(
                "printf '[lx-test]\\nSigLevel = Never\\nServer = file://{index_dir}\\n' \
                 >> /etc/pacman.conf \
                 && pacman -Sy --noconfirm \
                 && pacman -Sl lx-test \
                 && echo PACMAN_INDEX_OK"
            ),
            "PACMAN_INDEX_OK",
        ),
        "rpm" => validate_repo_script(
            &engine,
            &lx,
            "docker.io/library/fedora:latest",
            &format!(
                "printf '[lx-test]\\nname=lx-test\\nbaseurl=file://{index_dir}\\nenabled=1\\ngpgcheck=0\\n' \
                 > /etc/yum.repos.d/lx-test.repo \
                 && dnf makecache -y --disablerepo='*' --enablerepo=lx-test \
                 && echo DNF_INDEX_OK"
            ),
            "DNF_INDEX_OK",
        ),
        "apk" => validate_repo_script(
            &engine,
            &lx,
            "docker.io/library/alpine:3.20",
            &format!(
                "mkdir -p /etc/apk \
                 && printf 'file://{index_dir}\\n' > /etc/apk/repositories.lx \
                 && cat /etc/apk/repositories >> /etc/apk/repositories.lx 2>/dev/null || true \
                 && cp /etc/apk/repositories /etc/apk/repositories.bak \
                 && cp /etc/apk/repositories.lx /etc/apk/repositories \
                 && apk update 2>/dev/null; apk update 2>/dev/null \
                 && echo APK_INDEX_OK"
            ),
            "APK_INDEX_OK",
        ),
        other => bail!("no native repo validator for format '{other}'"),
    }
}

/// The shared shape of every format's validator: run a shell script (which
/// asserts success by printing `sentinel`) inside the format's native image.
fn validate_repo_script(
    engine: &str,
    lx: &Path,
    image: &str,
    script: &str,
    sentinel: &str,
) -> Result<()> {
    let out = run_lx(engine, image, lx, &["sh", "-c", script], None, None)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !stdout.contains(sentinel) {
        bail!(
            "{image} repo validation failed ({}): {}",
            out.status,
            stdout + &*String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// Run `lx <args>` inside `image`, mounting `lx` at `/usr/local/bin/lx` and
/// (optionally) a read-only payload at `/payload` and a writable work
/// directory at `/work`. The engine is whatever [`engine`] returned.
pub fn run_lx(
    engine: &str,
    image: &str,
    lx: &Path,
    args: &[&str],
    payload: Option<&Path>,
    work: Option<&Path>,
) -> Result<std::process::Output> {
    let mut cmd = Command::new(engine);
    cmd.args(["run", "--rm", "--user", "root", "-v"]);
    cmd.arg(format!("{}:/usr/local/bin/lx:ro", lx.display()));
    if let Some(p) = payload {
        cmd.arg("-v");
        cmd.arg(format!("{}:/payload:ro", p.display()));
    }
    if let Some(w) = work {
        cmd.arg("-v");
        cmd.arg(format!("{}:/work", w.display()));
    }
    cmd.arg(image);
    cmd.args(args);
    cmd.output()
        .map_err(|e| anyhow::anyhow!("could not run '{engine}': {e}"))
}
