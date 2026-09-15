// SPDX-License-Identifier: GPL-3.0-or-later

//! Distro build benchmark: time `lx build --from-dir` inside real containers.
//!
//! Reuses the matrix/engine/`lx`-mounting scaffolding from `lx_lib::containerbench`
//! (the same code behind `tests/container_matrix.rs`), so adding a distro or an
//! architecture is a single edit in `containerbench::TARGETS`. Unlike the
//! host-side `benchmarking/run.sh` (which compares lx against fpm/nfpm on one
//! machine), this measures how `lx build` performs *per distro* — the apk, apt,
//! dnf and pacman paths in their native environments.
//!
//! ```sh
//! cargo build --release --bin lx-containerbench
//! CONTAINER_ENGINE=docker ./target/release/lx-containerbench
//! LX_CONTAINER_TARGETS=alpine,debian ./target/release/lx-containerbench
//! ```

use std::path::Path;
use std::time::Instant;

use lx_lib::containerbench;

fn main() -> anyhow::Result<()> {
    let engine = containerbench::engine().unwrap_or_else(|| {
        eprintln!("no container engine (podman/docker) on PATH; set CONTAINER_ENGINE");
        std::process::exit(2);
    });
    let lx = containerbench::lx_binary().unwrap_or_else(|| {
        eprintln!("no lx binary (set LX_BIN, or build --target x86_64-unknown-linux-musl)");
        std::process::exit(2);
    });
    let lx = lx.canonicalize().unwrap_or_else(|_| lx.clone());

    // A tiny, deterministic payload: one text file and one ELF binary, shared
    // read-only into every container so the measurement is lx, not I/O.
    let payload = std::env::temp_dir().join(format!("lx-cbench-payload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&payload);
    std::fs::create_dir_all(&payload)?;
    std::fs::write(payload.join("hello.txt"), b"benchmark payload\n")?;
    let binary = if Path::new("/bin/true").exists() {
        "/bin/true"
    } else {
        lx.to_str().unwrap_or("")
    };
    if !binary.is_empty() {
        let _ = std::fs::copy(binary, payload.join("mybinary"));
    }

    let runs: usize = std::env::var("CB_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let warmup: usize = std::env::var("CB_WARMUP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    let targets = containerbench::selected_targets();
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let rem = secs % 86400;
    println!(
        "## Distro build benchmark — {days}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    );
    println!();
    println!("- Engine: `{engine}`");
    println!("- Payload: `{}`", payload.display());
    println!(
        "- Runs: {run} per distro ({warmup} warm-up)",
        run = runs,
        warmup = warmup
    );
    println!();
    println!("| Distro | Status | Median (ms) | Min (ms) | Runs |");
    println!("|--------|--------|------------:|---------:|-----:|");

    let mut failed = false;
    for t in targets {
        match bench_target(&engine, &lx, &payload, t, warmup, runs) {
            Ok((median, min)) => {
                println!("| {} | ok | {median} | {min} | {runs} |", t.key);
            }
            Err(e) => {
                println!("| {} | **failed** | — | — | — |", t.key);
                eprintln!("  ⚠ {}: {e:#}", t.key);
                failed = true;
            }
        }
    }

    let _ = std::fs::remove_dir_all(&payload);
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

/// Time `lx build --from-dir` inside a container, building in that distro's
/// native format (from `target.package_format`). Returns `(median_ms, min_ms)`.
/// Uses the shared `containerbench::run_lx` so the docker invocation can't
/// diverge from the test matrix's.
fn bench_target(
    engine: &str,
    lx: &std::path::Path,
    payload: &std::path::Path,
    target: &containerbench::ContainerTarget,
    warmup: usize,
    runs: usize,
) -> anyhow::Result<(u128, u128)> {
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
        "/out",
    ];
    let run_one = || -> anyhow::Result<()> {
        let out =
            containerbench::run_lx(engine, target.image, lx, &build_args, Some(payload), None)?;
        if !out.status.success() {
            return Err(anyhow::anyhow!(
                "lx build failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    };

    for _ in 0..warmup {
        run_one()?; // discard timing
    }

    let mut times: Vec<u128> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        run_one()?;
        times.push(start.elapsed().as_millis());
    }
    times.sort_unstable();
    let median = times[times.len() / 2];
    let min = times[0];
    Ok((median, min))
}
