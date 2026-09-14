// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Result};
use clap::Args;

use crate::consumer;
use crate::manifest::Manifest;
use lx_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct UpdateArgs {
    /// Package to check (omit to check every lx-managed package).
    pub package: Option<String>,

    /// Print the available release's notes for each outdated package.
    #[arg(long)]
    pub diff: bool,
}

/// Check every lx-managed package against its latest release, reporting
/// what's outdated without installing anything (the `apt update` half of
/// the update/upgrade split; `lx upgrade` does the install).
pub fn run(args: UpdateArgs, token: Option<&str>) -> Result<()> {
    let manifest = Manifest::load()?;

    let targets: Vec<String> = match &args.package {
        Some(p) => {
            if !manifest.packages.contains_key(p) {
                bail!("'{p}' is not managed by lx (run `lx install {p}` first)");
            }
            vec![p.clone()]
        }
        None => manifest.packages.keys().cloned().collect(),
    };

    if targets.is_empty() {
        println!("no lx-managed packages installed");
        return Ok(());
    }

    let client = GitHubClient::new(token.map(|s| s.to_string()))?;
    let mut outdated = 0;

    for package in &targets {
        let entry = manifest.current(package).unwrap();
        let format = consumer::format_or_host(&entry.format);
        let repo = consumer::repo_name(package);
        let release = match client.latest_release(&consumer::index_org(), &repo) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  ? {package}: {e:#}");
                continue;
            }
        };
        let Some(resolved) =
            consumer::resolve_asset(&release, package, format, &entry.arch, &entry.distribution)
        else {
            eprintln!(
                "  ? {package}: no {} asset for {}/{} in release '{}'",
                format.name(),
                entry.arch,
                entry.distribution,
                release.tag_name
            );
            continue;
        };
        let candidate = resolved.version;
        let installed =
            consumer::installed_version(package, format).unwrap_or_else(|| entry.version.clone());
        match consumer::is_newer(&installed, &candidate, format) {
            Ok(true) => {
                println!("  ↑ {package}: {installed} -> {candidate} (run `lx upgrade {package}`)");
                if args.diff {
                    print_release_notes(&release);
                }
                outdated += 1;
            }
            Ok(false) => println!("  = {package} up to date ({installed})"),
            Err(e) => eprintln!("  ? {package}: {e:#}"),
        }
    }

    if outdated == 0 {
        println!("\nall packages up to date");
    } else {
        println!("\n{outdated} package(s) can be upgraded; run `lx upgrade` to install");
    }
    Ok(())
}

/// Prints the candidate release's own notes as a stand-in changelog,
/// indented under its `↑ package: ...` line.
fn print_release_notes(release: &lx_lib::github::Release) {
    match release.body.as_deref().map(str::trim) {
        Some(body) if !body.is_empty() => {
            for line in body.lines() {
                println!("      {line}");
            }
        }
        _ => println!("      (no release notes)"),
    }
}
