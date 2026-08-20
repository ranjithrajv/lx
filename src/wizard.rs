use anyhow::{bail, Result};
use clap::Args;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::config::PackageConfig;
use lpt_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Directory to write package.yaml into (defaults to current dir).
    #[arg(long)]
    pub output: Option<PathBuf>,
}

fn prompt(prompt: &str, default: &str) -> String {
    print!("{prompt} [{default}]: ");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line).ok();
    let line = line.trim().to_string();
    if line.is_empty() {
        default.to_string()
    } else {
        line
    }
}

fn prompt_yes(question: &str, default: bool) -> bool {
    let d = if default { "y" } else { "n" };
    let answer = prompt(question, d).to_ascii_lowercase();
    answer.starts_with('y')
}

pub fn run(args: InitArgs) -> Result<()> {
    println!("lpt init — interactive setup wizard\n");

    let package_name = prompt("Package name", "mytool");
    let github_repo = prompt("GitHub repo (owner/repo)", "owner/mytool");

    let mut cfg = PackageConfig {
        package_name,
        github_repo,
        ..PackageConfig::default()
    };

    // Offer auto-discovery to build the pattern map without hand-typing.
    let auto = prompt_yes(
        "Auto-discover release assets from the latest release?",
        true,
    );
    if auto {
        let client = GitHubClient::new(None)?;
        let (owner, repo) = crate::discovery::split_repo(&cfg.github_repo)?;
        match client.latest_release(owner, repo) {
            Ok(release) => {
                let matched = crate::discovery::match_assets(&release);
                if matched.is_empty() {
                    println!("\n  (no assets matched — you'll configure patterns manually)");
                } else {
                    println!("\n  discovered {} architectures:", matched.len());
                    for m in &matched {
                        println!("    {:<8} {}", m.arch, m.asset);
                    }
                    let use_auto = prompt_yes("\nUse these auto-discovered patterns?", true);
                    if use_auto {
                        for m in matched {
                            cfg.architectures.set_pattern(
                                m.arch.clone(),
                                crate::config::ArchConfig {
                                    release_pattern: m.asset,
                                },
                            );
                        }
                    }
                }
            }
            Err(e) => println!("\n  (discovery failed: {e:#})"),
        }
    }

    if cfg.architectures.is_empty() {
        println!("\nNo patterns configured — the config will use auto-discovery at build time.");
    }

    cfg.description = prompt("Description", &cfg.package_name);
    let fmt = prompt("Artifact format (tar.gz/tgz/zip/raw)", "tar.gz");
    cfg.artifact_format = fmt;

    let dists = prompt(
        "Distributions (comma-separated; blank = all)",
        "bookworm,trixie,forky,sid",
    );
    if !dists.trim().is_empty() {
        cfg.debian_distributions = dists.split(',').map(|s| s.trim().to_string()).collect();
    }

    let output = args.output.unwrap_or_else(|| PathBuf::from("package.yaml"));
    if output.exists() {
        let ok = prompt_yes(
            &format!("'{}' already exists — overwrite?", output.display()),
            false,
        );
        if !ok {
            bail!("aborted");
        }
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let yaml = serde_yaml::to_string(&cfg)?;
    std::fs::write(&output, yaml)?;
    println!(
        "\nWrote {} — run `lpt validate {}` to check it.",
        output.display(),
        output.display()
    );
    Ok(())
}
