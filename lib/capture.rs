// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx capture` — build a package from the output of an install command.
//!
//! The `checkinstall` flow: run a command (e.g. `make install`) with
//! `$DESTDIR` pointing at a staging tree, then package that tree exactly as
//! it landed. Unlike `--from-dir`, the captured layout is preserved
//! (`usr/…`, `etc/…`), because the tree is staged through the `prefix: "/"`
//! path — the same "you supply a tree" mode `--from-dir --prefix /` uses.
//!
//! Dependencies are auto-populated from the captured ELFs via the shared
//! shlibdeps core, so a Makefile-only project produces a package with real
//! relations and no `package.yaml`.

use anyhow::{bail, Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};

use crate::debs::detect_dist;

#[derive(Debug, Clone, Args)]
pub struct CaptureArgs {
    /// Package name for the built package.
    #[arg(long)]
    pub name: String,

    /// Version for the built package.
    #[arg(long)]
    pub version: String,

    /// Output directory for the built packages.
    #[arg(long, default_value = "dist")]
    pub output: PathBuf,

    /// Package format (default: the host's native format).
    #[arg(long)]
    pub format: Option<String>,

    /// Where the command's `$DESTDIR` points (default: a temporary tree).
    #[arg(long)]
    pub destdir: Option<PathBuf>,

    /// Exclude paths matching this glob from the captured tree (repeatable;
    /// e.g. `--exclude "tmp/*"`). Matched against the path relative to the
    /// stage, with or without a leading `/`.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Package description.
    #[arg(long)]
    pub description: Option<String>,

    /// License (SPDX identifier).
    #[arg(long)]
    pub license: Option<String>,

    /// The install command to run and capture. Quote it as one argument:
    /// `lx capture --name foo --version 1.0 'make install'`.
    #[arg(required = true)]
    pub command: String,
}

/// Run the install command into a staging tree, then package that tree.
pub fn run(args: CaptureArgs) -> Result<()> {
    let tmp = tempfile::tempdir().context("creating capture workdir")?;
    let stage = match &args.destdir {
        Some(dir) => dir.clone(),
        None => tmp.path().join("root"),
    };
    std::fs::create_dir_all(&stage)
        .with_context(|| format!("creating capture stage {}", stage.display()))?;

    println!("capturing: {} (DESTDIR={})", args.command, stage.display());
    let status = std::process::Command::new("sh")
        .args(["-c", &args.command])
        .env("DESTDIR", &stage)
        .status()
        .with_context(|| format!("running capture command: {}", args.command))?;
    if !status.success() {
        bail!("capture command failed: {}", args.command);
    }

    apply_excludes(&stage, &args.exclude)?;

    // Auto-populate `depends:` from the captured ELFs, using the same core as
    // the build pipeline. A tree with no ELF files (e.g. a script-only
    // install) gets no dependency rather than the C-runtime fallback.
    let has_elf = crate::scandeps::find_elf_files(&stage)
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let depends = if has_elf {
        crate::scandeps::compute_depends_from_dir(&stage, "", false).0
    } else {
        String::new()
    };

    let cfg_path = tmp.path().join("capture.yaml");
    let cfg = CaptureConfig {
        package_name: args.name.clone(),
        version: args.version.clone(),
        description: args.description.clone().unwrap_or_default(),
        license_spdx: args.license.clone().unwrap_or_default(),
        depends,
    };
    std::fs::write(
        &cfg_path,
        serde_yaml::to_string(&cfg).context("rendering capture config")?,
    )
    .with_context(|| format!("writing {}", cfg_path.display()))?;

    let format = args
        .format
        .clone()
        .unwrap_or_else(|| crate::index::detect_host_format().name().to_string());
    // Route through the "you supply files" path (`--from-dir`) with a `/`
    // prefix: that preserves the captured tree as-is and uses the relaxed
    // local validation (no `github_repo` needed).
    let bargs = crate::build::BuildArgs {
        config: cfg_path,
        output: args.output.clone(),
        format: Some(format),
        host: true,
        // One package for the host's own suite, like `checkinstall`.
        distributions: detect_dist().filter(|d| !d.is_empty()),
        from_dir: Some(stage.clone()),
        prefix: Some("/".to_string()),
        ..Default::default()
    };
    crate::build::run(bargs, None)
}

/// The subset of `package.yaml` the capture writes. Field names are
/// `PackageConfig` keys so the file parses (it uses `deny_unknown_fields`).
#[derive(serde::Serialize)]
struct CaptureConfig {
    package_name: String,
    version: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    license_spdx: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    depends: String,
}

/// Remove every entry under `root` whose stage-relative path matches one of
/// `exclude`. A matching directory is removed whole (its children are not
/// walked).
fn apply_excludes(root: &Path, exclude: &[String]) -> Result<()> {
    if exclude.is_empty() {
        return Ok(());
    }
    let patterns: Vec<glob::Pattern> = exclude
        .iter()
        .filter_map(|g| glob::Pattern::new(g).ok())
        .collect();
    if patterns.is_empty() {
        return Ok(());
    }
    walk_excludes(root, root, &patterns)
}

fn walk_excludes(dir: &Path, root: &Path, patterns: &[glob::Pattern]) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        let rel = path.strip_prefix(root).unwrap_or(&path);
        let absolute = format!("/{}", rel.to_string_lossy());
        let matches = patterns
            .iter()
            .any(|p| p.matches_path(rel) || p.matches_path(Path::new(&absolute)));
        if matches {
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
            println!("  excluded {}", rel.display());
        } else if path.is_dir() {
            walk_excludes(&path, root, patterns)?;
        }
    }
    Ok(())
}
