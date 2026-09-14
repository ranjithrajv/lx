// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx publish` — the whole producer→distributor loop in one command.
//!
//! Builds every requested package format and then generates that format's
//! repository index, so a maintainer's release job is "run `lx publish` and
//! serve the output directory" instead of a hand-written matrix that builds
//! N formats and remembers to run the repo tool N times. Each format gets
//! its own subdirectory (`<output>/<format>/`), which both keeps the
//! artifacts readable and avoids index-filename collisions between
//! backends (apt and opkg both write `Packages`).

use anyhow::{bail, Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::build::{self, BuildArgs};
use crate::repo::{self, RepoArgs};

#[derive(Debug, Clone, Args)]
pub struct PublishArgs {
    /// Path to package.yaml
    #[arg(default_value = lx_lib::constants::DEFAULT_CONFIG_FILENAME)]
    pub config: PathBuf,

    /// Version of the software to build (defaults to the latest release).
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Build version/revision (defaults to "1").
    #[arg(long, default_value = "1")]
    pub build_version: String,

    /// Directory to write packages and per-format repository indexes into.
    /// Each format lands in its own subdirectory.
    #[arg(long, default_value = "dist")]
    pub output: PathBuf,

    /// Comma-separated package formats, or `all`. Default: deb,rpm,arch.
    #[arg(long, default_value = "deb,rpm,arch")]
    pub formats: String,

    /// Origin/Label recorded in the generated repository indexes.
    #[arg(long, default_value = "lx")]
    pub origin: String,

    /// Suite name for single-suite indexes (default: stable).
    #[arg(long, default_value = "stable")]
    pub suite: String,

    /// Sign built packages and the generated indexes with this key
    /// (ASCII-armored secret key; passphrase via `$LX_SIGN_PASSPHRASE`).
    #[arg(long, value_name = "KEY_FILE")]
    pub sign_key: Option<PathBuf>,

    /// Signing key id / fingerprint (`gpg --local-user`).
    #[arg(long, value_name = "KEY_ID")]
    pub sign_key_id: Option<String>,

    /// Emit SPDX SBOM + SLSA-style provenance alongside the packages.
    #[arg(long)]
    pub sbom: bool,

    /// Sign packages with cosign (Sigstore keyless; needs an OIDC token).
    #[arg(long)]
    pub cosign: bool,

    /// Proceed when an upstream asset has no checksum sidecar.
    #[arg(long)]
    pub allow_unverified: bool,

    /// Build only the local/host architecture (native-only), instead of
    /// every architecture the release publishes.
    #[arg(long)]
    pub host: bool,

    /// Build packages but skip repository index generation.
    #[arg(long)]
    pub no_index: bool,

    /// Print the per-format plan without building or indexing.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(args: PublishArgs, token: Option<&str>) -> Result<()> {
    let formats = resolve_formats(&args.formats)?;
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("failed to create '{}'", args.output.display()))?;

    println!(
        "publishing {} format(s) into {}: {}",
        formats.len(),
        args.output.display(),
        formats.join(", ")
    );

    let mut built: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for format in &formats {
        let dir = args.output.join(format);
        println!("\n=== building {format} → {} ===", dir.display());
        let bargs = BuildArgs {
            config: args.config.clone(),
            all: None,
            version: args.version.clone(),
            build_version: args.build_version.clone(),
            architectures: None,
            host: args.host,
            distributions: None,
            output: dir,
            format: Some(format.clone()),
            provider: None,
            no_verify: false,
            allow_unverified: args.allow_unverified,
            lintian: false,
            lintian_fail_on_warnings: false,
            lintian_pedantic: false,
            lintian_suppress: None,
            dry_run: args.dry_run,
            max_parallel: 0,
            pinned_metadata: None,
            cache_dir: None,
            api_cache_dir: None,
            source: false,
            summary: false,
            telemetry: false,
            save_baseline: false,
            progress: false,
            progress_path: None,
            keep: false,
            sign_key: args.sign_key.clone(),
            sign_key_id: args.sign_key_id.clone(),
            sign_method: None,
            local: false,
            from_dir: None,
            from_file: None,
            package_name: None,
            prefix: None,
            overlay: None,
            update_lock: false,
            artifact_cache_dir: None,
            verify: false,
            sandbox: false,
            install_build_deps: false,
            sbom: args.sbom,
            cosign: args.cosign,
            cross_target: None,
            bindep: true,
        };
        match build::run(bargs, token) {
            Ok(()) => built.push(format.clone()),
            Err(e) => {
                eprintln!("✗ {format} build failed: {e:#}");
                failures.push(format.clone());
            }
        }
    }

    if !args.no_index && !args.dry_run {
        for format in &built {
            println!("\n=== indexing {format} ===");
            let rargs = RepoArgs {
                dir: args.output.join(format),
                format: Some(format.clone()),
                suite: args.suite.clone(),
                multi_suite: false,
                components: "main".into(),
                origin: args.origin.clone(),
                sign_key: args.sign_key.clone(),
                sign_key_id: args.sign_key_id.clone(),
            };
            if let Err(e) = repo::run(rargs) {
                // A format can build but carry no indexable artifact (e.g.
                // arch support filters every distribution out); that's a
                // warning, not a publish failure.
                eprintln!("⚠ {format} index skipped: {e:#}");
            }
        }
    }

    if !failures.is_empty() {
        bail!("publish failed for: {}", failures.join(", "));
    }

    println!("\n✓ published {}", built.join(", "));
    Ok(())
}

/// Resolve the `--formats` value into a concrete list, rejecting an empty
/// selection. `all` expands to every registered packager.
fn resolve_formats(value: &str) -> Result<Vec<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("all") {
        return Ok(crate::plugins::packager_names()
            .into_iter()
            .map(str::to_string)
            .collect());
    }
    let list: Vec<String> = v
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() {
        bail!("no formats selected (expected e.g. deb,rpm,arch or all)");
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_comma_separated_formats() {
        assert_eq!(resolve_formats("deb,rpm").unwrap(), vec!["deb", "rpm"]);
        assert_eq!(
            resolve_formats(" deb , arch ").unwrap(),
            vec!["deb", "arch"]
        );
    }

    #[test]
    fn resolves_single_format() {
        assert_eq!(resolve_formats("deb").unwrap(), vec!["deb"]);
    }

    #[test]
    fn resolves_all_to_every_packager() {
        let all = resolve_formats("all").unwrap();
        assert!(all.contains(&"deb".to_string()));
        assert!(all.contains(&"rpm".to_string()));
        assert!(all.contains(&"arch".to_string()));
    }

    #[test]
    fn empty_selection_is_rejected() {
        assert!(resolve_formats("").is_err());
        assert!(resolve_formats(" , ").is_err());
    }
}
