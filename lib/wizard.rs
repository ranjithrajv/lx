// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Result};
use clap::Args;
use dialoguer::{theme::ColorfulTheme, Confirm, Input};
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

    /// Import from the Arch User Repository: fetch <name>'s PKGBUILD from
    /// the AUR and convert pkgname/pkgver/source/depends into a starter
    /// package.yaml (makedeb-orphan migration path; review the output —
    /// PKGBUILD build() steps become prebuild_steps hints, not code).
    #[arg(long, value_name = "AUR_PKG")]
    pub from_aur: Option<String>,
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

/// Free-text prompt via dialoguer (foss-mind audit 2026-08-27: replaces the
/// hand-rolled print/read_line pair for real TTY handling — Ctrl-C, echo,
/// history). Falls back to `default` on EOF/Ctrl-C/non-TTY stdin so a piped
/// or closed stdin still produces a config, matching the previous
/// read_line-into-empty behaviour.
fn prompt(prompt: &str, default: &str) -> String {
    Input::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default.to_string())
        .interact_text()
        .unwrap_or_else(|_| default.to_string())
}

/// Yes/no prompt; same error policy as [`prompt`] (EOF/Ctrl-C -> default).
fn prompt_yes(question: &str, default: bool) -> bool {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(question)
        .default(default)
        .interact()
        .unwrap_or(default)
}

pub fn run(args: InitArgs) -> Result<()> {
    // --from-aur: convert an AUR PKGBUILD into a starter package.yaml.
    if let Some(name) = &args.from_aur {
        return import_from_aur(name, args.output);
    }
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
            .unwrap_or_else(|| PathBuf::from(lx_lib::constants::DEFAULT_CONFIG_FILENAME));
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
            "Wrote template '{name}' to {} — edit it, then run `lx validate {}`.",
            output.display(),
            output.display()
        );
        return Ok(());
    }

    println!("lx init — interactive setup wizard\n");

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
            let default_host = lx_lib::constants::DEFAULT_GERRIT_HOST;
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
        .unwrap_or_else(|| PathBuf::from(lx_lib::constants::DEFAULT_CONFIG_FILENAME));
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
        "\nWrote {} — run `lx validate {}` to check it.",
        output.display(),
        output.display()
    );
    Ok(())
}

/// Fetch an AUR package's metadata + PKGBUILD and render a starter
/// package.yaml. Field mapping: pkgname -> package_name, pkgver ->
/// version, url host + pkgname -> github_repo guess (review!), depends ->
/// depends (Arch names kept verbatim with a warning — they rarely match
/// Debian names), license -> license_spdx, source=…github… -> release
/// asset hints in comments. PKGBUILD build()/package() bodies are shell,
/// not declarative: they become commented `prebuild_steps` hints, never
/// executed code (unlike makedeb, lx never evals recipes).
fn import_from_aur(name: &str, output: Option<PathBuf>) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct RpcResult {
        #[serde(default, rename = "Name")]
        pkgname: String,
        #[serde(default, rename = "Version")]
        pkgver: String,
        #[serde(default, rename = "Description")]
        description: String,
        #[serde(default, rename = "URL")]
        url: String,
        #[serde(default, rename = "License")]
        license: Vec<String>,
        #[serde(default, rename = "Depends")]
        depends: Vec<String>,
        #[serde(default, rename = "Maintainer")]
        maintainer: String,
    }
    #[derive(serde::Deserialize)]
    struct Rpc {
        #[serde(default)]
        results: Vec<RpcResult>,
    }

    let client = lx_lib::http::new_client()?;
    let meta: Rpc = client
        .get(format!(
            "https://aur.archlinux.org/rpc/?v=5&type=info&arg[]={name}"
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("AUR lookup for '{name}' failed"))?
        .json()
        .with_context(|| format!("parsing AUR metadata for '{name}'"))?;
    let m = meta
        .results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("'{name}' not found in the AUR"))?;

    let pkgbuild: String = client
        .get(format!(
            "https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h={}",
            m.pkgname
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("fetching PKGBUILD for '{}'", m.pkgname))?
        .text()
        .unwrap_or_default();
    let has_build_fn = pkgbuild.contains("build()");
    let src_hint: Option<String> = pkgbuild
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("source="))
        .map(str::to_string);

    let repo_guess = github_guess(&m.url, &m.pkgname);
    let mut yaml = format!(
        "# Imported from AUR package '{}' via `lx init --from-aur` — REVIEW ME.\n",
        m.pkgname
    );
    yaml.push_str(&format!("package_name: {}\n", m.pkgname));
    yaml.push_str(&format!(
        "github_repo: {}   # guessed from AUR url: {}\n",
        repo_guess, m.url
    ));
    if !m.description.is_empty() {
        yaml.push_str(&format!(
            "description: \"{}\"\n",
            m.description.replace('"', "'")
        ));
    }
    if !m.pkgver.is_empty() {
        yaml.push_str(&format!("version: \"{}\"\n", m.pkgver));
    }
    if let Some(l) = m.license.first() {
        yaml.push_str(&format!(
            "license_spdx: {l}   # AUR license field, verify SPDX\n"
        ));
    }
    if !m.depends.is_empty() {
        yaml.push_str(&format!(
            "# WARNING: Arch dependency names kept verbatim — map to Debian names:\ndepends: \"{}\"\n",
            m.depends.join(", ")
        ));
    }
    if !m.maintainer.is_empty() {
        yaml.push_str(&format!("# AUR maintainer: {}\n", m.maintainer));
    }
    if has_build_fn {
        yaml.push_str(
            "# This PKGBUILD compiles from source (build() present). Consider:\n#   build_mode: source\n#   build_system: cmake   # or: custom + build_commands/install_commands\n#   build_suites: [trixie, forky, sid]\n#   architectures: [<this-host-arch>]\n",
        );
    } else {
        yaml.push_str(
            "# Binary repack (no build() in PKGBUILD): pin release assets with\n#   architectures:\n#     amd64:\n#       release_pattern: \"...\"\n",
        );
    }
    if let Some(s) = src_hint {
        yaml.push_str(&format!("# AUR source line (asset-name hint): {s}\n"));
    }

    let output =
        output.unwrap_or_else(|| PathBuf::from(lx_lib::constants::DEFAULT_CONFIG_FILENAME));
    if output.exists() {
        bail!(
            "'{}' already exists; remove it or pass --output",
            output.display()
        );
    }
    std::fs::write(&output, yaml)?;
    println!(
        "Imported AUR '{}' to {} — review guesses, then `lx validate {}`.",
        m.pkgname,
        output.display(),
        output.display()
    );
    Ok(())
}

/// Guess owner/repo from an upstream URL; falls back to a placeholder the
/// user must fix (a wrong guess is louder than a silent one).
fn github_guess(url: &str, pkgname: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    for marker in ["https://github.com/", "http://github.com/"] {
        if let Some(rest) = u.strip_prefix(marker) {
            let rest = rest.trim_end_matches(".git");
            if rest.contains('/') {
                return rest.to_string();
            }
        }
    }
    format!("OWNER/{pkgname}   # FIXME: not a GitHub URL")
}

use anyhow::Context;
