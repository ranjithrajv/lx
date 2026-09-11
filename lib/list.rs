use anyhow::Result;
use clap::Args;
use std::collections::HashSet;

use crate::debs;
use crate::manifest::Manifest;

#[derive(Debug, Clone, Args)]
pub struct ListArgs {
    /// Show each package's dependency closure (via `dpkg-query`), not just
    /// its own version/arch/dist.
    #[arg(long)]
    pub tree: bool,
}

/// List packages lx has installed (tracked in its local manifest), cross-
/// checked against dpkg's own record of what's actually on disk.
pub fn run(args: ListArgs) -> Result<()> {
    let manifest = Manifest::load()?;
    if manifest.packages.is_empty() {
        println!("no lx-managed packages installed");
        return Ok(());
    }

    println!(
        "{:<20} {:<20} {:<8} {:<10} {:<8}INSTALLED",
        "PACKAGE", "VERSION", "ARCH", "DIST", "GENS"
    );
    for (name, gens) in &manifest.packages {
        let Some(entry) = gens.last() else { continue };
        let note = match debs::dpkg_installed_version(name) {
            Some(v) if v == entry.version => String::new(),
            Some(v) => format!(" (dpkg reports {v})"),
            None => " (missing from dpkg!)".to_string(),
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
            ancestors.insert(name.clone());
            print_deps(name, 1, &ancestors);
        }
    }
    Ok(())
}

/// Recursively prints `package`'s dependency closure, guarding against
/// cycles via `ancestors` (the chain of packages above this one in the
/// current branch).
fn print_deps(package: &str, depth: usize, ancestors: &HashSet<String>) {
    for dep in debs::dpkg_depends(package) {
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
