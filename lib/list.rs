// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use clap::{Args, ValueEnum};
use std::collections::HashSet;

use crate::consumer;
use crate::debget::{Catalog, DebGetKind, DebGetPackage};
use crate::index::InstallFormat;
use crate::manifest::Manifest;
use crate::scandeps;

/// Output shape for `lx list --catalog`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum CatalogFormat {
    /// `<name> [ installed ]`, padded (deb-get `list`).
    #[default]
    Table,
    /// Bare names, one per line (deb-get `list --raw`).
    Raw,
    /// Markdown table with website/method icons (deb-get `prettylist`).
    Pretty,
    /// Quoted CSV: name, pretty name, installed version, archs, method,
    /// summary (deb-get `csvlist`).
    Csv,
}

#[derive(Debug, Clone, Args)]
pub struct ListArgs {
    /// Show each package's dependency closure (via the host package
    /// manager), not just its own version/arch/dist.
    #[arg(long)]
    pub tree: bool,

    /// List packages available from the deb-get catalog instead of the
    /// packages lx has installed. Covers deb-get's `list`/`prettylist`/
    /// `csvlist`.
    #[arg(long)]
    pub catalog: bool,

    /// With `--catalog`: only this repo directory (e.g. `01-main`).
    #[arg(long, requires = "catalog")]
    pub repo: Option<String>,

    /// With `--catalog`: include definitions lx cannot use (deb-get's
    /// `--include-unsupported`).
    #[arg(long, requires = "catalog")]
    pub include_unsupported: bool,

    /// With `--catalog`: output shape.
    #[arg(long, value_enum, default_value_t = CatalogFormat::Table, requires = "catalog")]
    pub format: CatalogFormat,

    /// Only packages the host package manager reports as installed.
    #[arg(long, conflicts_with = "not_installed")]
    pub installed: bool,

    /// Only packages the host package manager does *not* report as installed.
    #[arg(long, conflicts_with = "installed")]
    pub not_installed: bool,

    /// Audit the manifest: report lx-managed packages the host no longer has.
    #[arg(long)]
    pub verify: bool,

    /// With `--verify`: drop the stale entries instead of only reporting them.
    #[arg(long, requires = "verify")]
    pub prune: bool,
}

/// List packages lx has installed (tracked in its local manifest), cross-
/// checked against the host package manager's own record of what's actually
/// on disk. With `--catalog`, list the deb-get catalog instead.
pub fn run(args: ListArgs) -> Result<()> {
    if args.verify {
        return verify(&args);
    }
    if args.catalog {
        return catalog(&args);
    }
    manifest_list(&args)
}

/// The deb-get catalog listing (`--catalog`), mapping deb-get's
/// `list`/`prettylist`/`csvlist`.
fn catalog(args: &ListArgs) -> Result<()> {
    let cat = Catalog::load_default();
    if cat.is_empty() {
        println!(
            "no deb-get catalog at {} (run `lx index update`, or set LX_DEBGET_DIR)",
            cat.root.display()
        );
        return Ok(());
    }
    // Per-repo listings (pretty/csv with --repo) show each repo's own copy;
    // the name listing uses the winning definition per name.
    let repos = args.repo.as_deref().map(|r| cat.definitions_in(r));
    let mut pkgs: Vec<&DebGetPackage> = match &repos {
        Some(v) => v.clone(),
        None => cat.packages.values().collect(),
    };
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));

    let (arch, codename) = host_gate();
    if args.format == CatalogFormat::Pretty {
        println!("| Source   | Package Name   | Description   |");
        println!("| :------: | :------------- | :------------ |");
    }
    for p in pkgs {
        if args.format != CatalogFormat::Pretty
            && args.format != CatalogFormat::Csv
            && !args.include_unsupported
            && !(p.is_supported() && p.supports(&arch, codename.as_deref()))
        {
            continue;
        }
        let installed = crate::debs::dpkg_installed_version(&p.name).is_some();
        if args.installed && !installed {
            continue;
        }
        if args.not_installed && installed {
            continue;
        }
        match args.format {
            CatalogFormat::Table => {
                if installed {
                    println!("{:<30} [ installed ]", p.name);
                } else {
                    println!("{}", p.name);
                }
            }
            CatalogFormat::Raw => println!("{}", p.name),
            CatalogFormat::Pretty => println!(
                "| [<img src=\"../.github/{}\" align=\"top\" width=\"20\" />]({}) | `{}` | <i>{}</i> |",
                icon(p.kind),
                p.website,
                p.name,
                p.summary
            ),
            CatalogFormat::Csv => {
                let installed_version =
                    crate::debs::dpkg_installed_version(&p.name).unwrap_or_default();
                println!(
                    "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"",
                    csv(&p.name),
                    csv(&p.pretty_name),
                    csv(&installed_version),
                    csv(&p.archs_supported.join(" ")),
                    p.kind_label(),
                    csv(&p.summary)
                );
            }
        }
    }
    Ok(())
}

/// The lx-managed manifest listing (the default).
fn manifest_list(args: &ListArgs) -> Result<()> {
    let manifest = Manifest::load()?;
    if manifest.is_empty() {
        println!("no lx-managed packages installed");
        return Ok(());
    }

    println!(
        "{:<20} {:<20} {:<8} {:<10} {:<8}INSTALLED",
        "PACKAGE", "VERSION", "ARCH", "DIST", "GENS"
    );
    for (name, gens) in manifest.history() {
        let Some(entry) = gens.last() else { continue };
        let format = consumer::format_or_host(&entry.format);
        let installed = consumer::installed_version(name, format);
        // The manifest lists installs; the filters mirror the catalog ones so
        // the same flags audit both views.
        if args.installed && installed.is_none() {
            continue;
        }
        if args.not_installed && installed.is_some() {
            continue;
        }
        let note = match installed {
            Some(v) if v == entry.version => String::new(),
            Some(v) => format!(" ({} reports {v})", format.name()),
            None => format!(" (missing from {}!)", format.name()),
        };
        println!(
            "{:<20} {:<20} {:<8} {:<10} {:<8}{}{}",
            name,
            entry.version,
            entry.arch,
            entry.distribution,
            gens.len(),
            entry.installed_at,
            note
        );
        if args.tree {
            let mut ancestors = HashSet::new();
            ancestors.insert(name.to_string());
            print_deps(name, 1, &ancestors);
        }
    }
    Ok(())
}

/// `--verify`: report (and with `--prune`, drop) manifest entries the host no
/// longer has. Absorbs deb-get's `fix-installed`.
fn verify(args: &ListArgs) -> Result<()> {
    let mut manifest = Manifest::load()?;
    let mut stale = Vec::new();
    for name in manifest.names() {
        let Some(entry) = manifest.current(name) else {
            continue;
        };
        let format = consumer::format_or_host(&entry.format);
        if consumer::installed_version(name, format).is_none() {
            stale.push(name.to_string());
        }
    }
    if stale.is_empty() {
        println!("every lx-managed package is still installed");
        return Ok(());
    }
    for name in &stale {
        if args.prune {
            manifest.forget(name);
            println!("forgot {name} (no longer installed)");
        } else {
            println!("{name} is recorded but not installed");
        }
    }
    if args.prune {
        manifest.save()?;
    } else {
        println!("re-run with --prune to drop these entries");
    }
    Ok(())
}

/// Recursively prints `package`'s dependency closure, guarding against
/// cycles via `ancestors` (the chain of packages above this one in the
/// current branch).
fn print_deps(package: &str, depth: usize, ancestors: &HashSet<String>) {
    for dep in scandeps::pkg_depends(package) {
        let cyclic = ancestors.contains(&dep);
        println!(
            "{}└─ {}{}",
            "  ".repeat(depth),
            dep,
            if cyclic { " (cycle)" } else { "" }
        );
        if !cyclic {
            let mut next = ancestors.clone();
            next.insert(dep.clone());
            print_deps(&dep, depth + 1, &next);
        }
    }
}

/// The host's deb arch / codename, for deb-get's `ARCHS_SUPPORTED` gate.
fn host_gate() -> (String, Option<String>) {
    let arch = crate::install_pkg::detect_arch(InstallFormat::Deb).unwrap_or_default();
    (arch, crate::debs::detect_dist())
}

/// deb-get's `prettylist` icon, chosen by method.
fn icon(kind: DebGetKind) -> &'static str {
    match kind {
        DebGetKind::Apt => "debian.png",
        DebGetKind::Github => "github.png",
        DebGetKind::Gitlab => "gitlab.png",
        DebGetKind::Ppa => "launchpad.png",
        _ => "direct.png",
    }
}

/// Escape a value for a CSV field (double embedded quotes).
fn csv(s: &str) -> String {
    s.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_escapes_quotes() {
        assert_eq!(csv("a\"b"), "a\"\"b");
        assert_eq!(csv("plain"), "plain");
    }

    #[test]
    fn icon_maps_each_method() {
        assert_eq!(icon(DebGetKind::Apt), "debian.png");
        assert_eq!(icon(DebGetKind::Github), "github.png");
        assert_eq!(icon(DebGetKind::Ppa), "launchpad.png");
        assert_eq!(icon(DebGetKind::Direct), "direct.png");
    }
}
