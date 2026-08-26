use anyhow::{bail, Result};
use clap::Args;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::config::PackageConfig;

#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Directory to write package.yaml into (defaults to current dir).
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Start from a bundled debian-multiarch-builder template
    /// (e.g. "rust/eza", "go/hugo") instead of the interactive wizard:
    /// writes templates/<name>.yaml verbatim, then stops. Pass an unknown
    /// name to list the available templates.
    #[arg(long)]
    pub template: Option<String>,
}

/// Bundled starter configs, ported verbatim from
/// ranjithrajv/debian-multiarch-builder's templates/ directory. The legacy
/// keys they use (summary/license/vendor/dependencies/download_pattern/
/// architecture_map) are all honored by `PackageConfig::apply_legacy_compat`.
pub const EMBEDDED_TEMPLATES: &[(&str, &str)] = &[
    ("rust/eza", include_str!("../templates/rust/eza.yaml")),
    ("rust/bat", include_str!("../templates/rust/bat.yaml")),
    (
        "rust/generic",
        include_str!("../templates/rust/generic.yaml"),
    ),
    (
        "rust/ripgrep",
        include_str!("../templates/rust/ripgrep.yaml"),
    ),
    ("go/hugo", include_str!("../templates/go/hugo.yaml")),
    ("go/kubectl", include_str!("../templates/go/kubectl.yaml")),
    ("go/generic", include_str!("../templates/go/generic.yaml")),
    ("c/neovim", include_str!("../templates/c/neovim.yaml")),
    ("c/generic", include_str!("../templates/c/generic.yaml")),
    (
        "nodejs/generic",
        include_str!("../templates/nodejs/generic.yaml"),
    ),
    (
        "python/generic",
        include_str!("../templates/python/generic.yaml"),
    ),
    (
        "ruby/generic",
        include_str!("../templates/ruby/generic.yaml"),
    ),
];

fn list_templates() {
    println!("Available --template names:");
    for (name, _) in EMBEDDED_TEMPLATES {
        println!("  {name}");
    }
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
    // --template: write a bundled starter config verbatim (the action's
    // "Option 3: Use a Template" workflow) and stop — no prompts.
    if let Some(name) = &args.template {
        let body = match EMBEDDED_TEMPLATES.iter().find(|(n, _)| n == name) {
            Some((_, body)) => *body,
            None => {
                eprintln!("Unknown template '{name}'");
                list_templates();
                bail!("unknown template '{name}'");
            }
        };
        let output = args
            .output
            .unwrap_or_else(|| PathBuf::from(lpt_lib::constants::DEFAULT_CONFIG_FILENAME));
        if output.exists() {
            bail!(
                "'{}' already exists; remove it or pass --output",
                output.display()
            );
        }
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&output, body)?;
        println!(
            "Wrote template '{name}' to {} — edit it, then run `lpt validate {}`.",
            output.display(),
            output.display()
        );
        return Ok(());
    }

    println!("lpt init — interactive setup wizard\n");

    let package_name = prompt("Package name", "mytool");
    let source = prompt(
        "Source provider (github/gitlab/gitea/forgejo/bitbucket/gerrit)",
        "github",
    )
    .trim()
    .to_ascii_lowercase();
    let source = if source.is_empty() {
        "github".to_string()
    } else {
        source
    };
    let repo_prompt = match source.as_str() {
        "gitlab" => "GitLab repo (owner/repo)",
        "gitea" => "Gitea repo (owner/repo)",
        "forgejo" => "Forgejo repo (owner/repo)",
        "bitbucket" => "Bitbucket repo (workspace/repo)",
        "gerrit" => "Gerrit project (owner/repo)",
        _ => "GitHub repo (owner/repo)",
    };
    let default_repo = "owner/mytool";
    let github_repo = prompt(repo_prompt, default_repo);

    let mut cfg = PackageConfig {
        package_name,
        github_repo,
        source: source.clone(),
        ..PackageConfig::default()
    };
    // Optional host for self-hosted instances
    match source.as_str() {
        "gitlab" => {
            let host = prompt("GitLab host (blank for gitlab.com)", "gitlab.com");
            if host != "gitlab.com" && !host.trim().is_empty() {
                cfg.gitlab_host = Some(host.trim().to_string());
            }
        }
        "gitea" => {
            let host = prompt("Gitea host (blank for codeberg.org)", "codeberg.org");
            if host != "codeberg.org" && !host.trim().is_empty() {
                cfg.gitea_host = Some(host.trim().to_string());
            }
        }
        "forgejo" => {
            let host = prompt("Forgejo host (blank for codeberg.org)", "codeberg.org");
            if host != "codeberg.org" && !host.trim().is_empty() {
                cfg.forgejo_host = Some(host.trim().to_string());
            }
        }
        "bitbucket" => {
            let host = prompt("Bitbucket host (blank for bitbucket.org)", "bitbucket.org");
            if host != "bitbucket.org" && !host.trim().is_empty() {
                cfg.bitbucket_host = Some(host.trim().to_string());
            }
        }
        "gerrit" => {
            let default_host = lpt_lib::constants::DEFAULT_GERRIT_HOST;
            let host = prompt(
                &format!("Gerrit host (blank for {default_host})"),
                default_host,
            );
            if host != default_host && !host.trim().is_empty() {
                cfg.gerrit_host = Some(host.trim().to_string());
            }
        }
        _ => {}
    }

    // Offer auto-discovery to build the pattern map without hand-typing.
    let auto = prompt_yes(
        "Auto-discover release assets from the latest release?",
        true,
    );
    if auto {
        let src_plugin = crate::plugins::source::get_source_plugin(&source).ok_or_else(|| {
            anyhow::anyhow!(
                "unsupported source '{}' (expected one of: {})",
                source,
                crate::plugins::source::source_available_names().join(", ")
            )
        })?;
        match src_plugin.latest_release(&cfg.github_repo, None, None) {
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
        "bullseye,bookworm,trixie,forky,sid",
    );
    if !dists.trim().is_empty() {
        cfg.debian_distributions = dists.split(',').map(|s| s.trim().to_string()).collect();
    }

    let output = args
        .output
        .unwrap_or_else(|| PathBuf::from(lpt_lib::constants::DEFAULT_CONFIG_FILENAME));
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
