use anyhow::{bail, Result};
use clap::Args;
use std::path::PathBuf;

use crate::config::{ArchSpec, PackageConfig};

#[derive(Debug, Clone, Args)]
pub struct ValidateArgs {
    /// Path to package.yaml
    #[arg(default_value = lpt_lib::constants::DEFAULT_CONFIG_FILENAME)]
    pub config: PathBuf,

    /// Version to validate against (defaults to latest release).
    #[arg(short = 'v', long)]
    pub version: Option<String>,
}

pub fn run(args: ValidateArgs, token: Option<&str>) -> Result<()> {
    let cfg = PackageConfig::load(&args.config)?;
    println!(
        "config: OK (package '{}' from {})",
        cfg.package_name, cfg.github_repo
    );

    // Check a structural invariant: manual patterns must not collide.
    if cfg.has_manual_patterns() {
        println!(
            "patterns: OK ({} architectures pinned)",
            cfg.architectures.patterns().len()
        );
    } else if let ArchSpec::List(names) = &cfg.architectures {
        println!(
            "patterns: auto-discovery restricted to {} architecture(s): {}",
            names.len(),
            names.join(", ")
        );
    } else {
        println!("patterns: auto-discovery (architectures key omitted)");
    }

    // Network checks against the source API (github/gitlab).
    let source_name = cfg.effective_source();
    let source = crate::plugins::source::get_source_plugin(&source_name).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported source '{}' (expected one of: {})",
            source_name,
            crate::plugins::source::source_available_names().join(", ")
        )
    })?;
    crate::plugins::source::apply_source_host(source.as_ref(), &cfg);
    let token_for_source = crate::plugins::source::resolve_source_token(source.as_ref(), token);

    let release = match &args.version {
        Some(v) => source.release_by_tag(&cfg.github_repo, v, token_for_source.as_deref(), None)?,
        None => source.latest_release(&cfg.github_repo, token_for_source.as_deref(), None)?,
    };
    println!(
        "release: found '{}' ({} assets)",
        release.tag_name,
        release.assets.len()
    );
    crate::build::warn_if_prerelease_or_draft(&release);

    // Verify every pinned pattern resolves to an actual asset.
    if cfg.has_manual_patterns() {
        let mut missing = Vec::new();
        for (arch, acfg) in cfg.architectures.patterns() {
            let expanded =
                crate::build::expand_version_placeholder(&acfg.release_pattern, &release.tag_name);
            if !release.assets.iter().any(|a| a.name == expanded) {
                missing.push(format!("{arch}: '{expanded}'"));
            }
        }
        if !missing.is_empty() {
            bail!(
                "pinned patterns not found in release '{}':\n  {}",
                release.tag_name,
                missing.join("\n  ")
            );
        }
        println!(
            "assets: OK (all {} pinned patterns resolve)",
            cfg.architectures.patterns().len()
        );
    }

    // Auto-discovery sanity check.
    if !cfg.has_manual_patterns() {
        let matched = crate::discovery::match_assets(&release);
        println!(
            "assets: auto-discovery found {} of {} architectures",
            matched.len(),
            crate::config::DEFAULT_ARCHITECTURES.len()
        );
    }

    println!("\nvalidate: OK");
    Ok(())
}
