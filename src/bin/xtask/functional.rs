// SPDX-License-Identifier: GPL-3.0-or-later

//! `func-tests` — end-to-end CLI tests, ported from the former
//! `utils/functional-tests/*.sh` scripts.
//!
//! Each suite builds its own fixtures in a temp directory, runs the release
//! `lx` binary, and prints per-check `PASS`/`FAIL` lines. Suites stop at the
//! first failure unless `--no-fail-fast` (or `FAIL_FAST=0`) is set.
//!
//! Artifact inspection uses `lx`'s in-process readers (`debarchive`,
//! `rpmarchive`, `archarchive`) rather than `rpm2cpio`/`cpio`/`tar`, so the
//! tests stay dependency-light.

use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tempfile::TempDir;

type Suite = fn(&Path) -> Result<usize>;

pub fn run(lx: Option<PathBuf>, fail_fast: bool, only: Vec<String>) -> Result<()> {
    let lx = lx.unwrap_or_else(|| crate::repo_root().join("target/release/lx"));
    if !crate::is_executable(&lx) {
        bail!(
            "release binary missing: {} (run: cargo build --release)",
            lx.display()
        );
    }

    let suites: &[(&str, Suite)] = &[
        ("test-build", test_build),
        ("test-convert", test_convert),
        ("test-info-validate-schema", test_info_validate_schema),
        ("test-repo-deps", test_repo_deps),
        ("test-catalog", test_catalog),
    ];
    let selected: Vec<&(&str, Suite)> = suites
        .iter()
        .filter(|(name, _)| only.is_empty() || only.iter().any(|wanted| wanted == name))
        .collect();
    if selected.is_empty() {
        bail!("no suite matches {:?}", only);
    }

    let mut failed: Vec<&str> = Vec::new();
    for (name, suite) in &selected {
        println!();
        println!("========== {name} ==========");
        let passed = match suite(&lx) {
            Ok(failures) => failures == 0,
            Err(err) => {
                eprintln!("  ERROR: {err:#}");
                false
            }
        };
        if passed {
            println!("[ok] {name}");
        } else {
            failed.push(name);
            if fail_fast {
                break;
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!("ALL FUNCTIONAL TESTS PASSED ({} suites)", selected.len());
        Ok(())
    } else {
        println!("FAILURES: {}", failed.join(" "));
        bail!("{} functional suite(s) failed", failed.len());
    }
}

/// Per-suite PASS/FAIL accounting.
struct Checks {
    label: &'static str,
    failures: usize,
}

impl Checks {
    fn new(label: &'static str) -> Self {
        Self { label, failures: 0 }
    }

    fn pass(&self, msg: &str) {
        println!("  PASS: {msg}");
    }

    fn bad(&mut self, msg: &str) {
        println!("  FAIL: {msg}");
        self.failures += 1;
    }

    /// Print the suite verdict and return its failure count.
    fn finish(self) -> usize {
        if self.failures == 0 {
            println!("{}: ALL PASS", self.label);
        } else {
            println!("{}: FAILURES", self.label);
        }
        self.failures
    }
}

fn temp_dir() -> Result<TempDir> {
    tempfile::tempdir().context("creating temp dir")
}

/// Stage an ELF the packagers can introspect: `/bin/true` when present, else
/// the `lx` binary itself.
fn stage_binary_payload(dest: &Path, lx: &Path) -> Result<()> {
    let source = if Path::new("/bin/true").is_file() {
        PathBuf::from("/bin/true")
    } else {
        lx.to_path_buf()
    };
    fs::copy(&source, dest)
        .with_context(|| format!("copying {} to {}", source.display(), dest.display()))?;
    Ok(())
}

/// Files in `dir` whose name ends with `suffix`, sorted.
fn list_files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(suffix))
        })
        .collect();
    files.sort();
    files
}

fn count_files(dir: &Path, suffix: &str) -> usize {
    list_files(dir, suffix).len()
}

/// SHA-256 of the first `suffix` file in `dir`, if any.
fn sha256_of(dir: &Path, suffix: &str) -> Result<Option<String>> {
    match list_files(dir, suffix).first() {
        Some(path) => Ok(Some(lx_lib::checksum::sha256_file(path)?)),
        None => Ok(None),
    }
}

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
                return Some(path);
            }
        }
    }
    None
}

/// `lx convert <input> -t <target> -o <out>` → first artifact with `suffix`.
fn convert(lx: &Path, input: &Path, target: &str, out: &Path, suffix: &str) -> Option<PathBuf> {
    let ok = Command::new(lx)
        .arg("convert")
        .arg(input)
        .arg("-t")
        .arg(target)
        .arg("-o")
        .arg(out)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !ok {
        return None;
    }
    list_files(out, suffix).into_iter().next()
}

/// Best-effort reachability probe: TCP connect to `github.com:443`.
fn forge_reachable() -> bool {
    let Ok(addrs) = ("github.com", 443).to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(8)).is_ok())
}

fn test_build(lx: &Path) -> Result<usize> {
    let tmp = temp_dir()?;
    let payload = tmp.path().join("payload");
    fs::create_dir_all(&payload)?;
    fs::write(payload.join("hello.txt"), "hello\n")?;
    stage_binary_payload(&payload.join("mybinary"), lx)?;

    let mut checks = Checks::new("BUILD");

    println!("[build] offline payload matrix (host arch, --prefix)");
    for (format, suffix) in [
        ("deb", ".deb"),
        ("rpm", ".rpm"),
        ("arch", ".pkg.tar.zst"),
        ("apk", ".apk"),
        ("ipk", ".ipk"),
    ] {
        let out = tmp.path().join(format!("out-{format}"));
        fs::create_dir_all(&out)?;
        let status = Command::new(lx)
            .arg("build")
            .arg("--from-dir")
            .arg(&payload)
            .args([
                "--prefix",
                "/usr/local/share/ft",
                "--package-name",
                "ft",
                "--version",
                "1.0.0",
                "--format",
                format,
                "--host",
                "--distributions",
                "default",
            ])
            .arg("--output")
            .arg(&out)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `lx build`")?;
        if status.success() {
            let count = count_files(&out, suffix);
            if count >= 1 {
                checks.pass(&format!("{format} -> {count} artifact(s)"));
            } else {
                checks.bad(&format!("{format}: no {suffix} files"));
            }
        } else {
            checks.bad(&format!("{format}: build failed"));
        }
    }

    println!("[build] reproducibility (deb, two builds, same hash)");
    let o1 = tmp.path().join("r1");
    let o2 = tmp.path().join("r2");
    fs::create_dir_all(&o1)?;
    fs::create_dir_all(&o2)?;
    for out in [&o1, &o2] {
        Command::new(lx)
            .arg("build")
            .arg("--from-dir")
            .arg(&payload)
            .args([
                "--prefix",
                "/usr/local/share/ft",
                "--package-name",
                "ft",
                "--version",
                "1.0.0",
                "--format",
                "deb",
                "--host",
            ])
            .arg("--output")
            .arg(out)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `lx build`")?;
    }
    match (sha256_of(&o1, ".deb")?, sha256_of(&o2, ".deb")?) {
        (Some(a), Some(b)) if a == b => checks.pass(&format!("reproducible ({a})")),
        (a, b) => checks.bad(&format!("hash differs: {a:?} vs {b:?}")),
    }

    Ok(checks.finish())
}

fn test_convert(lx: &Path) -> Result<usize> {
    let tmp = temp_dir()?;
    let payload = tmp.path().join("payload");
    fs::create_dir_all(&payload)?;
    stage_binary_payload(&payload.join("mybinary"), lx)?;

    let mut checks = Checks::new("CONVERT");

    let src_out = tmp.path().join("src");
    let status = Command::new(lx)
        .arg("build")
        .arg("--from-dir")
        .arg(&payload)
        .args([
            "--package-name",
            "ft",
            "--version",
            "3.0.0",
            "--format",
            "deb",
            "--host",
            "--distributions",
            "trixie",
        ])
        .arg("--output")
        .arg(&src_out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running `lx build`")?;
    if !status.success() {
        checks.bad("source deb build failed");
        return Ok(checks.finish());
    }
    let Some(src_deb) = list_files(&src_out, ".deb").into_iter().next() else {
        checks.bad("source deb missing");
        return Ok(checks.finish());
    };

    println!("[convert] round trip: deb -> rpm -> arch -> deb");
    let Some(d2r) = convert(lx, &src_deb, "rpm", &tmp.path().join("d2r"), ".rpm") else {
        checks.bad("deb->rpm");
        return Ok(checks.finish());
    };
    checks.pass("deb->rpm");
    let Some(r2a) = convert(lx, &d2r, "arch", &tmp.path().join("r2a"), ".pkg.tar.zst") else {
        checks.bad("rpm->arch");
        return Ok(checks.finish());
    };
    checks.pass("rpm->arch");
    let Some(a2d) = convert(lx, &r2a, "deb", &tmp.path().join("a2d"), ".deb") else {
        checks.bad("arch->deb");
        return Ok(checks.finish());
    };
    checks.pass("arch->deb");

    println!("[convert] payload survives each hop");
    let x = tmp.path().join("x-deb-rpm");
    match lx_lib::rpmarchive::extract(&d2r, &x) {
        Ok(()) if find_file(&x, "mybinary").is_some() => checks.pass("deb->rpm payload"),
        Ok(()) => checks.bad("deb->rpm: mybinary missing"),
        Err(err) => checks.bad(&format!("deb->rpm payload: {err:#}")),
    }
    let x = tmp.path().join("x-rpm-arch");
    match lx_lib::archarchive::extract(&r2a, &x) {
        Ok(()) if find_file(&x, "mybinary").is_some() => checks.pass("rpm->arch payload"),
        Ok(()) => checks.bad("rpm->arch: mybinary missing"),
        Err(err) => checks.bad(&format!("rpm->arch payload: {err:#}")),
    }
    let x = tmp.path().join("x-arch-deb");
    match lx_lib::debarchive::extract(&a2d, &x) {
        Ok(()) if find_file(&x, "mybinary").is_some() => checks.pass("arch->deb payload"),
        Ok(()) => checks.bad("arch->deb: mybinary missing"),
        Err(err) => checks.bad(&format!("arch->deb payload: {err:#}")),
    }

    println!("[convert] idempotency (deb->rpm twice, same hash)");
    let i1 = tmp.path().join("i1");
    let i2 = tmp.path().join("i2");
    let _ = convert(lx, &src_deb, "rpm", &i1, ".rpm");
    let _ = convert(lx, &src_deb, "rpm", &i2, ".rpm");
    match (sha256_of(&i1, ".rpm")?, sha256_of(&i2, ".rpm")?) {
        (Some(a), Some(b)) if a == b => checks.pass(&format!("idempotent ({a})")),
        (a, b) => checks.bad(&format!("differs: {a:?} vs {b:?}")),
    }

    println!("[convert] --dry-run produces no artifact");
    let dr = tmp.path().join("dr");
    let _ = Command::new(lx)
        .arg("convert")
        .arg(&src_deb)
        .arg("--dry-run")
        .args(["-t", "rpm"])
        .arg("-o")
        .arg(&dr)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if count_files(&dr, ".rpm") == 0 {
        checks.pass("dry-run: no artifact");
    } else {
        checks.bad("dry-run emitted .rpm");
    }

    println!("[convert] --to default = host format");
    let def = tmp.path().join("def");
    let _ = Command::new(lx)
        .arg("convert")
        .arg(&src_deb)
        .arg("-o")
        .arg(&def)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if count_files(&def, "") >= 1 {
        checks.pass("--to default");
    } else {
        checks.bad("--to default produced nothing");
    }

    Ok(checks.finish())
}

fn test_info_validate_schema(lx: &Path) -> Result<usize> {
    let tmp = temp_dir()?;
    let mut checks = Checks::new("INFO/VALIDATE/SCHEMA");

    println!("[info] host detection (json)");
    match Command::new(lx).args(["info", "--json"]).output() {
        Ok(output) if output.status.success() => {
            match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                Ok(value) => {
                    let manager = value
                        .get("package_manager")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let format = value
                        .get("package_format")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !manager.is_empty() && !format.is_empty() {
                        checks.pass(&format!(
                            "info reports manager='{manager}' format='{format}'"
                        ));
                    } else {
                        checks.bad("info: missing package_manager/package_format");
                    }
                }
                Err(err) => checks.bad(&format!("info --json: invalid JSON: {err}")),
            }
        }
        _ => checks.bad("info --json failed"),
    }

    println!("[validate] valid package.yaml passes, invalid fails");
    let good = tmp.path().join("good.yaml");
    let bad = tmp.path().join("bad.yaml");
    fs::write(
        &good,
        "package_name: demo\ngithub_repo: eza-community/eza\nversion: 1.0.0\n\
         description: d\nmaintainer: t <t@e>\nlicense_spdx: MIT\n",
    )?;
    fs::write(
        &bad,
        "github_repo: this-repo-does-not-exist-xyz/qqq\nversion: 1.0.0\n\
         description: d\nmaintainer: t <t@e>\n",
    )?;
    // `validate` does a live forge reachability check, so the "valid"
    // fixture must be a real, reachable repo.
    if !forge_reachable() {
        println!("  SKIP: validate (no network to github.com)");
    } else {
        let good_ok = Command::new(lx)
            .arg("validate")
            .arg(&good)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if good_ok {
            checks.pass("valid config accepted");
        } else {
            checks.bad("valid config rejected");
        }
        let bad_ok = Command::new(lx)
            .arg("validate")
            .arg(&bad)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if bad_ok {
            checks.bad("invalid config accepted");
        } else {
            checks.pass("invalid config rejected");
        }
    }

    println!("[schema] JSON schema generation is valid JSON");
    match Command::new(lx).arg("schema").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            if !text.contains("\"type\"") {
                checks.bad("schema missing 'type'");
            } else if serde_json::from_str::<serde_json::Value>(&text).is_ok() {
                checks.pass("schema emits valid JSON");
            } else {
                checks.bad("schema is not valid JSON");
            }
        }
        _ => checks.bad("schema command failed"),
    }

    Ok(checks.finish())
}

fn test_repo_deps(lx: &Path) -> Result<usize> {
    let tmp = temp_dir()?;
    let payload = tmp.path().join("payload");
    fs::create_dir_all(&payload)?;
    stage_binary_payload(&payload.join("mybinary"), lx)?;
    let mut checks = Checks::new("REPO/DEPS");

    println!("[repo] build an apt repository index from local .debs");
    let debs = tmp.path().join("debs");
    fs::create_dir_all(&debs)?;
    let status = Command::new(lx)
        .arg("build")
        .arg("--from-dir")
        .arg(&payload)
        .args([
            "--package-name",
            "ft",
            "--version",
            "1.0.0",
            "--format",
            "deb",
            "--host",
            "--distributions",
            "trixie",
        ])
        .arg("--output")
        .arg(&debs)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running `lx build`")?;
    if !status.success() {
        checks.bad("source .deb build failed");
        return Ok(checks.finish());
    }
    if list_files(&debs, ".deb").is_empty() {
        checks.bad("no .debs produced");
        return Ok(checks.finish());
    }

    if !forge_reachable() {
        println!("  SKIP: repo (no network to github.com)");
    } else {
        let ok = Command::new(lx)
            .arg("repo")
            .arg(&debs)
            .args(["--format", "deb", "--suite", "trixie", "--origin", "test"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            let packages = debs.join("Packages");
            if packages.is_file() && debs.join("Packages.gz").is_file() {
                checks.pass("repo index (Packages, Packages.gz present)");
            } else {
                checks.bad("repo: Packages/Packages.gz missing");
            }
            if fs::read_to_string(&packages)
                .map(|text| text.contains("Package: ft"))
                .unwrap_or(false)
            {
                checks.pass("repo lists 'ft'");
            } else {
                checks.bad("repo: 'ft' not in Packages");
            }
        } else {
            checks.bad("lx repo failed");
        }
    }

    println!("[deps scan] ELF dependency scanning via a real forge release");
    if !forge_reachable() {
        println!("  SKIP: deps scan (no network to github.com)");
    } else {
        match Command::new(lx)
            .args(["deps", "scan", "https://github.com/eza-community/eza"])
            .output()
        {
            Ok(output)
                if output.status.success()
                    && !String::from_utf8_lossy(&output.stdout).trim().is_empty() =>
            {
                checks.pass("deps scan produced output");
            }
            Ok(_) => checks.bad("deps scan: no output"),
            Err(err) => checks.bad(&format!("deps scan failed: {err}")),
        }
    }

    Ok(checks.finish())
}

/// The deb-get catalog environment (`LX_DEBGET_DIR`, XDG dirs) for a suite.
struct CatalogEnv {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
}

impl CatalogEnv {
    fn command(&self, lx: &Path) -> Command {
        let mut command = Command::new(lx);
        command
            .env("LX_DEBGET_DIR", &self.root)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_DATA_HOME", &self.data);
        command
    }
}

fn capture_stdout(command: &mut Command) -> Option<String> {
    command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

fn has_line(text: &str, needle: &str) -> bool {
    text.lines().any(|line| line == needle)
}

fn test_catalog(lx: &Path) -> Result<usize> {
    let tmp = temp_dir()?;
    let mut checks = Checks::new("CATALOG");
    let env = CatalogEnv {
        root: tmp.path().join("catalog"),
        config: tmp.path().join("config"),
        data: tmp.path().join("data"),
    };
    fs::create_dir_all(env.root.join("01-main.d"))?;
    fs::create_dir_all(env.root.join("99-local.d"))?;
    fs::write(
        env.root.join("01-main.d/alpha"),
        "DEFVER=1\nARCHS_SUPPORTED=\"all\"\n\
         URL=\"https://example.com/alpha_1.0_amd64.deb\"\nPRETTY_NAME=\"Alpha\"\n\
         WEBSITE=\"https://example.com\"\nSUMMARY=\"Alpha package.\"\n",
    )?;
    // Same name in 99-local: the override must win for the name listing/show.
    fs::write(
        env.root.join("99-local.d/alpha"),
        "DEFVER=1\nARCHS_SUPPORTED=\"all\"\n\
         URL=\"https://local.example.com/alpha_2.0_amd64.deb\"\nPRETTY_NAME=\"Alpha (local)\"\n\
         WEBSITE=\"https://example.com\"\nSUMMARY=\"Alpha override.\"\n",
    )?;
    // Wrong arch: hidden by default, shown with --include-unsupported.
    fs::write(
        env.root.join("01-main.d/beta"),
        "DEFVER=1\nARCHS_SUPPORTED=\"riscv64\"\n\
         URL=\"https://example.com/beta_1.0_riscv64.deb\"\nPRETTY_NAME=\"Beta\"\n\
         WEBSITE=\"https://example.com\"\nSUMMARY=\"Beta package.\"\n",
    )?;
    // No method: unsupported, hidden by default.
    fs::write(
        env.root.join("01-main.d/gamma"),
        "DEFVER=1\nPRETTY_NAME=\"Gamma\"\nWEBSITE=\"https://example.com\"\n\
         SUMMARY=\"Gamma package.\"\n",
    )?;
    // GitHub method (resolved by lx, never by executing the definition).
    fs::write(
        env.root.join("01-main.d/gh"),
        "DEFVER=1\nget_github_releases \"cli/cli\" \"latest\"\nPRETTY_NAME=\"GitHub CLI\"\n\
         WEBSITE=\"https://cli.github.com/\"\nSUMMARY=\"GitHub CLI.\"\n",
    )?;

    println!("[catalog] lx list --catalog honours the host gate and --include-unsupported");
    let listed = capture_stdout(
        env.command(lx)
            .args(["list", "--catalog", "--format", "raw"]),
    )
    .unwrap_or_default();
    if has_line(&listed, "alpha") {
        checks.pass("alpha listed");
    } else {
        checks.bad(&format!("alpha missing: {listed}"));
    }
    if has_line(&listed, "gh") {
        checks.pass("gh listed");
    } else {
        checks.bad(&format!("gh missing: {listed}"));
    }
    if has_line(&listed, "beta") {
        checks.bad("beta should be arch-gated out");
    } else {
        checks.pass("beta gated out");
    }
    if has_line(&listed, "gamma") {
        checks.bad("gamma should be unsupported-out");
    } else {
        checks.pass("gamma gated out");
    }
    let all = capture_stdout(env.command(lx).args([
        "list",
        "--catalog",
        "--format",
        "raw",
        "--include-unsupported",
    ]))
    .unwrap_or_default();
    if has_line(&all, "beta") {
        checks.pass("beta with --include-unsupported");
    } else {
        checks.bad("beta still missing");
    }
    if has_line(&all, "gamma") {
        checks.pass("gamma with --include-unsupported");
    } else {
        checks.bad("gamma still missing");
    }

    println!("[catalog] 99-local overrides 01-main by name");
    let shown = capture_stdout(env.command(lx).args(["show", "alpha"])).unwrap_or_default();
    if shown.contains("repo 99-local") {
        checks.pass("override repo");
    } else {
        checks.bad("override repo not reported");
    }
    if shown.contains("alpha_2.0_amd64.deb") {
        checks.pass("override URL");
    } else {
        checks.bad("override URL not reported");
    }
    let main_csv = capture_stdout(env.command(lx).args([
        "list",
        "--catalog",
        "--repo",
        "01-main",
        "--format",
        "csv",
    ]))
    .unwrap_or_default();
    if main_csv.contains("\"alpha\",\"Alpha\"") {
        checks.pass("01-main copy kept");
    } else {
        checks.bad("01-main alpha copy missing");
    }

    println!("[catalog] lx show reports the method and unsupported notes");
    let gh = capture_stdout(env.command(lx).args(["show", "gh"])).unwrap_or_default();
    if gh.contains("method: github") {
        checks.pass("github method");
    } else {
        checks.bad("github method not reported");
    }
    let gamma = capture_stdout(env.command(lx).args(["show", "gamma"])).unwrap_or_default();
    if gamma.contains("lx notes:") {
        checks.pass("unsupported note");
    } else {
        checks.bad("unsupported note missing");
    }
    let unknown_shown = env
        .command(lx)
        .args(["show", "chrome-does-not-exist"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if unknown_shown {
        checks.bad("unknown package shown");
    } else {
        checks.pass("unknown package refused");
    }

    println!("[catalog] --format csv is six quoted columns");
    let row = main_csv
        .lines()
        .find(|line| line.starts_with("\"gh\""))
        .unwrap_or("");
    let columns = row.matches("\",\"").count() + 1;
    if columns == 6 {
        checks.pass("6 columns");
    } else {
        checks.bad(&format!("expected 6 columns, got {columns}: {row}"));
    }

    println!("[catalog] --format pretty emits a markdown table");
    let pretty = capture_stdout(
        env.command(lx)
            .args(["list", "--catalog", "--format", "pretty"]),
    )
    .unwrap_or_default();
    if pretty
        .lines()
        .take(2)
        .any(|line| line.starts_with("| Source"))
    {
        checks.pass("header");
    } else {
        checks.bad("pretty header missing");
    }
    if pretty.contains("`gh`") {
        checks.pass("row");
    } else {
        checks.bad("pretty row missing");
    }

    println!("[catalog] --installed filters by host install state");
    // Nothing is installed here, so the installed listing must be empty.
    let installed = capture_stdout(env.command(lx).args(["list", "--catalog", "--installed"]))
        .unwrap_or_default();
    let count = installed.lines().count();
    if count == 0 {
        checks.pass("empty installed list");
    } else {
        checks.bad(&format!("installed list not empty ({count})"));
    }

    println!("[catalog] lx list --verify audits the manifest");
    let verify = capture_stdout(env.command(lx).args(["list", "--verify"])).unwrap_or_default();
    if verify.contains("every lx-managed package is still installed") {
        checks.pass("clean manifest verifies");
    } else {
        checks.bad("verify output unexpected");
    }

    println!("[catalog] lx index clean reports cached index data");
    let clean = env
        .command(lx)
        .args(["index", "clean", "--dry-run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if clean {
        checks.pass("clean --dry-run runs");
    } else {
        checks.bad("clean failed");
    }

    println!("[catalog] the debget read index serves the catalog");
    let _ = env
        .command(lx)
        .args(["index", "add", "debget", "--kind", "debget"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let search = capture_stdout(
        env.command(lx)
            .args(["index", "search", "--repo", "debget", "alpha"]),
    )
    .unwrap_or_default();
    if search.contains("Alpha override.") {
        checks.pass("index search");
    } else {
        checks.bad("index search missing hit");
    }
    let index_list = capture_stdout(env.command(lx).args(["index", "list"])).unwrap_or_default();
    if index_list.lines().any(|line| line.starts_with("debget")) {
        checks.pass("index registered");
    } else {
        checks.bad("debget index not registered");
    }

    Ok(checks.finish())
}
