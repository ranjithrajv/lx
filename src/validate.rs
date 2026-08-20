use anyhow::{bail, Result};
use clap::Args;
use std::path::PathBuf;

use crate::config::PackageConfig;
use crate::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct ValidateArgs {
    /// Path to package.yaml
    #[arg(default_value = "package.yaml")]
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
            cfg.architectures.len()
        );
    } else {
        println!("patterns: auto-discovery (architectures key omitted)");
    }

    // Network checks against the GitHub API.
    let client = GitHubClient::new(token.map(|s| s.to_string()))?;
    let (owner, repo) = crate::discovery::split_repo(&cfg.github_repo)?;

    let release = match &args.version {
        Some(v) => client.release(owner, repo, v)?,
        None => client.latest_release(owner, repo)?,
    };
    println!(
        "release: found '{}' ({} assets)",
        release.tag_name,
        release.assets.len()
    );

    // Verify every pinned pattern resolves to an actual asset.
    if cfg.has_manual_patterns() {
        let mut missing = Vec::new();
        for (arch, acfg) in &cfg.architectures {
            let expanded = acfg.release_pattern.replace("{version}", &release.tag_name);
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
            cfg.architectures.len()
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
