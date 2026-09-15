// SPDX-License-Identifier: GPL-3.0-or-later

//! `bench` — benchmark `lx` against the tools it replaces.
//!
//! Prints a Markdown report to stdout. No network access: every benchmark
//! stages a local payload so runs are comparable and reproducible. See
//! `benchmarking/README.md` for the methodology and ground rules.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{bail, Context, Result};

struct Config {
    lx: PathBuf,
    runs: u32,
    warmup: u32,
    dist: String,
}

pub fn run(lx: Option<PathBuf>, runs: u32, warmup: u32, dist: String) -> Result<()> {
    if runs < 1 {
        bail!("RUNS must be a positive integer (got {runs})");
    }
    let lx = resolve_lx(lx)?;
    if lx.to_string_lossy().contains("/target/debug/") {
        eprintln!(
            "warning: {} is a debug build; results are not release-comparable.",
            lx.display()
        );
    }

    let config = Config {
        lx,
        runs,
        warmup,
        dist,
    };
    print_header(&config);
    bench_pack_deb(&config)?;
    bench_resolve_deps(&config)?;
    bench_repo_index(&config)?;
    print_footer();
    Ok(())
}

fn resolve_lx(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if !crate::is_executable(&path) {
            bail!("lx binary not executable: {}", path.display());
        }
        return Ok(path);
    }
    if let Some(path) = crate::which("lx") {
        return Ok(path);
    }
    let root = crate::repo_root();
    for candidate in [root.join("target/release/lx"), root.join("target/debug/lx")] {
        if crate::is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    bail!("no lx binary found. Build one with `cargo build --release`, or pass --lx/LX_BIN.")
}

/// A command under test: program + args + optional cwd/stdout sink.
#[derive(Clone)]
struct Cmd {
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    stdout_file: Option<PathBuf>,
}

impl Cmd {
    fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            stdout_file: None,
        }
    }

    fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    fn stdout_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.stdout_file = Some(path.into());
        self
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(dir) = &self.cwd {
            command.current_dir(dir);
        }
        command
    }

    /// Run quiet (optionally sinking stdout to a file), for gating and timing.
    fn quiet(&self) -> Command {
        let mut command = self.command();
        match &self.stdout_file {
            Some(path) => match fs::File::create(path) {
                Ok(file) => {
                    command.stdout(file);
                }
                Err(_) => {
                    command.stdout(Stdio::null());
                }
            },
            None => {
                command.stdout(Stdio::null());
            }
        }
        command.stderr(Stdio::null());
        command
    }
}

impl Config {
    fn have(&self, bin: &str) -> bool {
        crate::which(bin).is_some()
    }

    /// Median and min wall-clock ms over `runs` iterations, after `warmup`.
    fn measure(&self, cmd: &Cmd) -> (u128, u128) {
        for _ in 0..self.warmup {
            let _ = cmd.quiet().status();
        }
        let mut times = Vec::with_capacity(self.runs as usize);
        for _ in 0..self.runs {
            let start = Instant::now();
            let _ = cmd.quiet().status();
            times.push(start.elapsed().as_millis());
        }
        times.sort_unstable();
        let min = times[0];
        let median = times[(self.runs as usize).div_ceil(2) - 1];
        (median, min)
    }

    fn run_one(&self, label: &str, required: Option<&str>, cmd: &Cmd, notes: &str) {
        if let Some(bin) = required {
            if !self.have(bin) {
                emit_row(label, "-", "-", "-", &format!("{notes} (not installed)"));
                return;
            }
        }
        let ok = cmd
            .quiet()
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !ok {
            emit_row(
                label,
                "failed",
                "-",
                "0",
                &format!("{notes} (command failed)"),
            );
            return;
        }
        let (median, min) = self.measure(cmd);
        emit_row(
            label,
            &median.to_string(),
            &min.to_string(),
            &self.runs.to_string(),
            notes,
        );
    }
}

fn emit_row(tool: &str, median: &str, min: &str, runs: &str, notes: &str) {
    println!("| {tool} | {median} | {min} | {runs} | {notes} |");
}

fn tool_version(cmd: &Cmd) -> String {
    cmd.command()
        .output()
        .ok()
        .and_then(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines().next().map(|line| line.trim().to_string())
        })
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn print_header(config: &Config) {
    let now = jiff::Timestamp::now();
    let date = now.strftime("%Y-%m-%d");
    let iso = now.strftime("%Y-%m-%dT%H:%M:%SZ");
    let arch = dpkg_arch();
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);

    println!("## Run {date} — {}", std::env::consts::ARCH);
    println!();
    println!("- Date:        {iso}");
    println!("- Host:        {}", host_line());
    println!("- Architecture: {arch}");
    println!("- CPUs:        {cpus}");
    println!(
        "- Timed runs:  {} per tool ({} warm-up)",
        config.runs, config.warmup
    );
    println!(
        "- Bench dist:  {} (single-suite deb comparison)",
        config.dist
    );
    println!(
        "- lx:          {}",
        tool_version(&Cmd::new(config.lx.clone()).arg("--version"))
    );
    println!("- Reference tools:");
    for tool in [
        "dpkg-deb",
        "dpkg-shlibdeps",
        "dpkg-scanpackages",
        "apt-ftparchive",
        "fpm",
        "nfpm",
    ] {
        if config.have(tool) {
            println!(
                "  - {tool}: {}",
                tool_version(&Cmd::new(tool).arg("--version"))
            );
        } else {
            println!("  - {tool}: not installed");
        }
    }
    println!();
}

fn print_footer() {
    println!("---");
    println!();
    println!("<!-- Generated by `cargo xtask bench`. Append to benchmarking/results.md to");
    println!("keep a record, and note anything unusual (thermal throttling, a busy machine,");
    println!("a non-release lx build). -->");
}

fn host_line() -> String {
    let release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|text| text.trim().to_string())
        .unwrap_or_default();
    format!(
        "{} {release} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

fn dpkg_arch() -> String {
    Command::new("dpkg")
        .arg("--print-architecture")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|arch| !arch.is_empty())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn write_executable(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content)?;
    set_executable(path)
}

fn has_dpkg_shlib_db() -> bool {
    let Ok(entries) = fs::read_dir("/var/lib/dpkg/info") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.ends_with(".symbols") || name.ends_with(".shlibs")
    })
}

fn bench_pack_deb(config: &Config) -> Result<()> {
    let work = tempfile::Builder::new()
        .prefix("lx-bench-deb.")
        .tempdir()
        .context("creating bench temp dir")?;
    let payload = work.path().join("payload");
    fs::create_dir_all(&payload)?;
    write_executable(&payload.join("bench"), "#!/bin/sh\necho bench\n")?;
    let ls = if Path::new("/usr/bin/ls").is_file() {
        "/usr/bin/ls"
    } else {
        "/bin/ls"
    };
    fs::copy(ls, payload.join("tool")).context("staging ls")?;
    fs::write(payload.join("data.txt"), "bench data\n")?;

    let arch = dpkg_arch();
    let root = work.path().join("root");
    fs::create_dir_all(root.join("DEBIAN"))?;
    fs::create_dir_all(root.join("usr/bin"))?;
    fs::copy(payload.join("bench"), root.join("usr/bin/bench"))?;
    fs::copy(payload.join("tool"), root.join("usr/bin/tool"))?;
    fs::write(
        root.join("DEBIAN/control"),
        format!(
            "Package: bench-pkg\nVersion: 1.0.0\nArchitecture: {arch}\n\
             Maintainer: Bench <bench@example.com>\nDescription: benchmark payload\n"
        ),
    )?;

    let nfpm_yaml = work.path().join("nfpm.yaml");
    fs::write(
        &nfpm_yaml,
        format!(
            "name: bench-pkg\narch: {arch}\nversion: 1.0.0\ncontents:\n\
             \x20 - src: {bench}\n    dst: /usr/bin/bench\n\
             \x20 - src: {tool}\n    dst: /usr/bin/tool\n",
            bench = payload.join("bench").display(),
            tool = payload.join("tool").display(),
        ),
    )?;
    let lx_out = work.path().join("lx-out");
    let nfpm_out = work.path().join("nfpm-out");
    fs::create_dir_all(&lx_out)?;
    fs::create_dir_all(&nfpm_out)?;

    println!("### Pack a .deb from a staged tree");
    println!();
    println!("Input: one payload directory (a script, a real ELF, and a data file),");
    println!("staged fresh per run. `lx` is compared against the reference packagers that");
    println!("take the same \"here are files\" input.");
    println!();
    println!(
        "`lx` is restricted to one distribution (`--distributions \"{}\"`) so",
        config.dist
    );
    println!("the row is one `.deb` vs. one `.deb`; without it `lx` builds the payload once");
    println!("per Debian suite.");
    println!();
    println!("| Tool | Median (ms) | Min (ms) | Runs | Notes |");
    println!("|---|---:|---:|---:|---|");

    config.run_one(
        "lx build --from-dir",
        None,
        &Cmd::new(config.lx.clone())
            .args(["build", "--from-dir"])
            .arg(&payload)
            .args([
                "--package-name",
                "bench-pkg",
                "--version",
                "1.0.0",
                "--format",
                "deb",
                "--host",
                "--distributions",
                config.dist.as_str(),
            ])
            .arg("--output")
            .arg(&lx_out),
        "also generates control/changelog/copyright + infers ELF deps",
    );
    config.run_one(
        "dpkg-deb --build",
        Some("dpkg-deb"),
        &Cmd::new("dpkg-deb")
            .arg("--build")
            .arg(&root)
            .arg(work.path().join("dpkg.deb")),
        "builds the .deb only",
    );
    config.run_one(
        "fpm -s dir -t deb",
        Some("fpm"),
        &Cmd::new("fpm")
            .args(["-s", "dir", "-t", "deb", "-n", "bench-pkg", "-v", "1.0.0"])
            .arg("-C")
            .arg(&payload)
            .args(["--prefix", "/usr/bin", "."])
            .arg("--package")
            .arg(work.path().join("fpm.deb")),
        "feature-parity reference packager",
    );
    config.run_one(
        "nfpm pkg -p deb",
        Some("nfpm"),
        &Cmd::new("nfpm")
            .args(["pkg", "-f"])
            .arg(&nfpm_yaml)
            .args(["-p", "deb", "--target"])
            .arg(&nfpm_out),
        "feature-parity reference packager",
    );
    println!();
    Ok(())
}

fn bench_resolve_deps(config: &Config) -> Result<()> {
    let work = tempfile::Builder::new()
        .prefix("lx-bench-deps.")
        .tempdir()
        .context("creating bench temp dir")?;
    let elf = if Path::new("/usr/bin/ls").is_file() {
        PathBuf::from("/usr/bin/ls")
    } else {
        PathBuf::from("/bin/ls")
    };
    let pkgdir = work.path().join("pkg");
    fs::create_dir_all(pkgdir.join("debian"))?;
    fs::write(
        pkgdir.join("debian/control"),
        "Source: bench-pkg\nSection: utils\nPriority: optional\n\
         Maintainer: Bench <bench@example.com>\nStandards-Version: 4.6.0\n\n\
         Package: bench-pkg\nArchitecture: any\nDepends: ${shlibs:Depends}\n\
         Description: benchmark payload\n benchmark payload\n",
    )?;

    if !has_dpkg_shlib_db() {
        println!("### Resolve shared-library dependencies");
        println!();
        println!("Skipped: this host has no dpkg `symbols`/`shlibs` database (the Debian");
        println!("files under `/var/lib/dpkg/info/`), which both `lx deps resolve` and");
        println!("`dpkg-shlibdeps` read. Run on a Debian/Ubuntu host to record this row.");
        println!();
        return Ok(());
    }

    println!("### Resolve shared-library dependencies");
    println!();
    println!(
        "Input: `{}`. `lx deps resolve` is a command-level drop-in for",
        elf.display()
    );
    println!("`dpkg-shlibdeps`; both read the local dpkg symbols/shlibs databases.");
    println!();
    println!("| Tool | Median (ms) | Min (ms) | Runs | Notes |");
    println!("|---|---:|---:|---:|---|");

    config.run_one(
        "lx deps resolve",
        None,
        &Cmd::new(config.lx.clone())
            .args(["deps", "resolve", "-O"])
            .arg(&elf),
        "command drop-in for dpkg-shlibdeps",
    );
    config.run_one(
        "dpkg-shlibdeps -O",
        Some("dpkg-shlibdeps"),
        &Cmd::new("dpkg-shlibdeps").arg("-O").arg(&elf).cwd(&pkgdir),
        "reference implementation",
    );
    println!();
    Ok(())
}

fn bench_repo_index(config: &Config) -> Result<()> {
    if !config.have("dpkg-deb") {
        println!("### Build an apt repository index");
        println!();
        println!("Skipped: dpkg-deb is needed to stage .debs.");
        println!();
        return Ok(());
    }

    let work = tempfile::Builder::new()
        .prefix("lx-bench-repo.")
        .tempdir()
        .context("creating bench temp dir")?;
    let repo = work.path().join("repo");
    fs::create_dir_all(&repo)?;
    let arch = dpkg_arch();
    for n in 1..=3 {
        let root = work.path().join(format!("root{n}"));
        fs::create_dir_all(root.join("DEBIAN"))?;
        fs::create_dir_all(root.join("usr/bin"))?;
        write_executable(
            &root.join(format!("usr/bin/bench{n}")),
            &format!("#!/bin/sh\necho bench{n}\n"),
        )?;
        fs::write(
            root.join("DEBIAN/control"),
            format!(
                "Package: bench-pkg-{n}\nVersion: 1.0.{n}\nArchitecture: {arch}\n\
                 Maintainer: Bench <bench@example.com>\nDescription: benchmark payload {n}\n"
            ),
        )?;
        let deb = repo.join(format!("bench-pkg-{n}_1.0.{n}_{arch}.deb"));
        let _ = Command::new("dpkg-deb")
            .arg("--build")
            .arg(&root)
            .arg(&deb)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    println!("### Build an apt repository index");
    println!();
    println!("Input: a directory of three `.deb`s. `lx repo` writes an apt-consumable");
    println!("index; the reference tools write the `Packages` file itself.");
    println!();
    println!("`lx` does more here: it gzips `Packages` and writes a `Release` file with");
    println!("MD5/SHA1/SHA256 checksums. The reference rows time the `Packages` file");
    println!("only, so this row is \"whole `lx repo` vs. whole reference command\".");
    println!();
    println!("| Tool | Median (ms) | Min (ms) | Runs | Notes |");
    println!("|---|---:|---:|---:|---|");

    config.run_one(
        "lx repo",
        None,
        &Cmd::new(config.lx.clone())
            .args(["repo", "--format", "deb"])
            .arg(&repo),
        "writes Packages + Packages.gz + Release",
    );
    config.run_one(
        "dpkg-scanpackages",
        Some("dpkg-scanpackages"),
        &Cmd::new("dpkg-scanpackages")
            .arg(&repo)
            .arg("/dev/null")
            .stdout_file(work.path().join("Packages.dpkg")),
        "uncompressed Packages only",
    );
    config.run_one(
        "apt-ftparchive packages",
        Some("apt-ftparchive"),
        &Cmd::new("apt-ftparchive")
            .arg("packages")
            .arg(&repo)
            .stdout_file(work.path().join("Packages.apt")),
        "uncompressed Packages only",
    );
    println!();
    Ok(())
}
