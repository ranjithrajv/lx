use anyhow::{bail, Result};
use clap::Args;

use crate::debs;
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
        let release = match client.latest_release(debs::LATEST_DEBS_ORG, package) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  ? {package}: {e:#}");
                continue;
            }
        };
        let Some(asset) = debs::find_asset(&release, package, &entry.arch, &entry.distribution)
        else {
            eprintln!(
                "  ? {package}: no .deb for {}/{} in release '{}'",
                entry.arch, entry.distribution, release.tag_name
            );
            continue;
        };
        let Some(candidate) = debs::control_version(&asset.name, package, &entry.arch) else {
            continue;
        };
        let installed =
            debs::dpkg_installed_version(package).unwrap_or_else(|| entry.version.clone());
        match debs::is_newer(&installed, &candidate) {
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
