// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx` developer task runner.
//!
//! This binary replaces the shell scripts that used to live under `utils/`
//! and `benchmarking/`. Invoke it through the `cargo xtask` alias declared in
//! `.cargo/config.toml`:
//!
//! ```text
//! cargo xtask policy         # enforce the Rust-only development policy
//! cargo xtask license-check  # SPDX header on staged .rs files
//! cargo xtask secret-scan    # credentials in the staged diff
//! cargo xtask coverage       # llvm-cov + uncovered-function report
//! cargo xtask func-tests     # end-to-end CLI tests
//! cargo xtask bench          # lx vs. the tools it replaces
//! cargo xtask dev-setup      # install the pre-commit hook dependencies
//! ```

mod bench;
mod coverage;
mod dev;
mod functional;
mod gaps;
mod license;
mod policy;
mod secret;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "lx developer tasks",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enforce the Rust-only development policy (no shell scripts).
    Policy,
    /// Require the SPDX GPL-3.0-or-later header on staged .rs files.
    LicenseCheck,
    /// Scan the staged diff for high-confidence credential patterns.
    SecretScan,
    /// Run llvm-cov, print uncovered functions, and gate on line coverage.
    Coverage {
        /// Minimum overall line coverage percentage.
        #[arg(long, env = "COVERAGE_MIN", default_value_t = 40)]
        min: u64,
    },
    /// Cross-reference covscan JSON with llvm-cov JSON (used by `coverage`).
    CoverageGaps {
        /// covscan output: `{"module.rs": [["name", start, end], ...]}`.
        fns_json: PathBuf,
        /// `cargo llvm-cov --json` output.
        cov_json: PathBuf,
    },
    /// Run the end-to-end CLI functional tests against a release binary.
    FuncTests {
        /// Binary under test (default: target/release/lx).
        #[arg(long, env = "LX")]
        lx: Option<PathBuf>,
        /// Continue past the first failing suite (also FAIL_FAST=0).
        #[arg(long)]
        no_fail_fast: bool,
        /// Run only the named suite(s); repeatable (default: all).
        #[arg(long)]
        suite: Vec<String>,
    },
    /// Benchmark lx against the tools it replaces; prints Markdown.
    Bench {
        /// lx binary under test (default: $PATH, else target/{release,debug}/lx).
        #[arg(long, env = "LX_BIN")]
        lx: Option<PathBuf>,
        /// Timed iterations per tool.
        #[arg(long, env = "RUNS", default_value_t = 5)]
        runs: u32,
        /// Untimed warm-up iterations per tool.
        #[arg(long, env = "WARMUP", default_value_t = 1)]
        warmup: u32,
        /// Debian suite for the single-.deb comparison.
        #[arg(long, env = "BENCH_DIST", default_value = "trixie")]
        dist: String,
    },
    /// Install the developer tools the pre-commit hooks need.
    DevSetup,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Policy => policy::run(),
        Command::LicenseCheck => license::run(),
        Command::SecretScan => secret::run(),
        Command::Coverage { min } => coverage::run(min),
        Command::CoverageGaps { fns_json, cov_json } => gaps::report(&fns_json, &cov_json),
        Command::FuncTests {
            lx,
            no_fail_fast,
            suite,
        } => functional::run(lx, fail_fast(no_fail_fast), suite),
        Command::Bench {
            lx,
            runs,
            warmup,
            dist,
        } => bench::run(lx, runs, warmup, dist),
        Command::DevSetup => dev::run(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// `--no-fail-fast` wins; otherwise `FAIL_FAST=0`/`false` disables it.
fn fail_fast(no_fail_fast: bool) -> bool {
    if no_fail_fast {
        return false;
    }
    std::env::var("FAIL_FAST")
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

/// The repository root (the crate manifest directory), independent of the
/// caller's working directory.
pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Search `PATH` for an executable named `bin`.
pub(crate) fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// True when `path` is a regular file with at least one execute bit set.
pub(crate) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
