// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx show` — deb-get `show` parity: everything known about one package.

use anyhow::{bail, Result};
use clap::Args;

use crate::debget::{Catalog, DebGetPackage};
use crate::manifest::Manifest;
use crate::scandeps;

#[derive(Debug, Clone, Args)]
pub struct ShowArgs {
    /// Package name (e.g. "eza").
    pub package: String,
}

pub fn run(args: ShowArgs) -> Result<()> {
    let manifest = Manifest::load().unwrap_or_default();
    let entries = manifest.generations(&args.package);
    let installed = scandeps::pkg_installed_version(&args.package);

    // Not managed and not installed: fall back to the deb-get catalog, so
    // `lx show` works for a package that is only *available*.
    if entries.is_none() && installed.is_none() {
        if let Some(p) = Catalog::load_default().get(&args.package).cloned() {
            print_catalog(&p);
            return Ok(());
        }
        bail!(
            "'{}' is unknown: not managed by lx, not installed on this host, \
             and not in the deb-get catalog",
            args.package
        );
    }

    println!("package: {}", args.package);
    let is_installed = installed.is_some();
    match installed {
        Some(ref v) => {
            let mgr = match scandeps::detect_pkg_mgr() {
                Some(scandeps::PkgMgr::Dpkg) => "dpkg",
                Some(scandeps::PkgMgr::Rpm) => "rpm",
                Some(scandeps::PkgMgr::Pacman) => "pacman",
                None => "unknown",
            };
            println!("installed: {v} ({mgr})");
        }
        None => println!("installed: no"),
    }
    match entries {
        Some(hist) => {
            println!("managed: yes ({} record(s))", hist.len());
            for (i, e) in hist.iter().enumerate() {
                println!(
                    "  [{}] version={} tag={} arch={} suite={} format={} asset={} at={}",
                    i, e.version, e.tag, e.arch, e.distribution, e.format, e.asset, e.installed_at
                );
            }
        }
        None => println!("managed: no"),
    }
    let deps = scandeps::pkg_depends(&args.package);
    if !deps.is_empty() {
        println!("depends: {}", deps.join(", "));
    }

    // ELF dependency scan: show what the binary actually needs at runtime
    // and compare against declared deps.
    if is_installed {
        print_elf_needs(&args.package, &deps);
    }

    println!(
        "source: {}/{}",
        crate::consumer::index_org(),
        crate::consumer::repo_name(&args.package)
    );
    Ok(())
}

/// Print one deb-get catalog definition (the `lx show` fallback for a
/// package that is available but not installed).
fn print_catalog(p: &DebGetPackage) {
    println!("package: {}", p.name);
    println!("available: yes (deb-get catalog, repo {})", p.repo);
    println!("method: {}", p.kind_label());
    if !p.pretty_name.is_empty() {
        println!("name: {}", p.pretty_name);
    }
    if let Some(v) = p.defver {
        println!("definition: v{v}");
    }
    if !p.summary.is_empty() {
        println!("summary: {}", p.summary);
    }
    if !p.website.is_empty() {
        println!("website: {}", p.website);
    }
    if !p.archs_supported.is_empty() {
        println!("archs: {}", p.archs_supported.join(" "));
    }
    if !p.codenames_supported.is_empty() {
        println!("codenames: {}", p.codenames_supported.join(" "));
    }
    for (label, value) in [
        ("apt repo", p.apt_repo_url.as_deref()),
        ("apt list", p.apt_list_name.as_deref()),
        ("ppa", p.ppa.as_deref()),
        ("repo", p.forge_repo.as_deref()),
        ("url", p.url.as_deref()),
    ] {
        if let Some(v) = value {
            println!("{label}: {v}");
        }
    }
    if p.has_post_download {
        println!("hook: post_download (not executed by lx)");
    }
    if !p.is_supported() {
        println!("lx notes: {}", p.unsupported.join("; "));
    }
}

/// Scan the installed package's ELF files and display runtime library
/// needs, comparing against declared deps.
fn print_elf_needs(package: &str, declared_deps: &[String]) {
    let files = scandeps::pkg_files(package);
    if files.is_empty() {
        return;
    }

    let mut elf_sonames = std::collections::BTreeSet::new();
    let mut scanned_any = false;

    for file in &files {
        let path = std::path::Path::new(file);
        if !path.is_file() || !crate::filemeta::is_elf(path).unwrap_or(false) {
            continue;
        }
        scanned_any = true;
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Ok(libs) = lx_lib::elfdeps::needed_libraries(&bytes) {
            for lib in &libs {
                if !lx_lib::elfdeps::is_essential_libc_soname(lib) {
                    elf_sonames.insert(lib.clone());
                }
            }
        }
    }

    if !scanned_any || elf_sonames.is_empty() {
        return;
    }

    // Resolve sonames to package names. A package's own libraries resolve back
    // to itself (kwallet ships libKF6WalletBackend.so.6, which it owns); that's
    // a self-provided soname, not a missing dependency, so skip it.
    let mut elf_pkgs = std::collections::BTreeSet::new();
    let mut unresolved = Vec::new();
    for soname in &elf_sonames {
        match scandeps::pkg_owner(soname) {
            Some(pkg) => {
                let pkg = pkg.split(':').next().unwrap_or(&pkg).to_string();
                if pkg.eq_ignore_ascii_case(package) {
                    continue;
                }
                elf_pkgs.insert(pkg);
            }
            None => {
                unresolved.push(soname.as_str());
            }
        }
    }

    let declared_set: std::collections::HashSet<String> = declared_deps
        .iter()
        .map(|d| d.to_ascii_lowercase())
        .collect();

    let missing: Vec<&String> = elf_pkgs
        .iter()
        .filter(|p| !declared_set.contains(&p.to_ascii_lowercase()))
        .collect();
    let unnecessary: Vec<&String> = declared_deps
        .iter()
        .filter(|d| !elf_pkgs.iter().any(|p| p.eq_ignore_ascii_case(d)))
        .collect();

    println!("\nelf_needs: {}", {
        let mut names: Vec<&str> = elf_pkgs.iter().map(String::as_str).collect();
        names.sort();
        names.join(", ")
    });

    if !missing.is_empty() {
        println!(
            "  ⚠ not declared in depends: {}",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !unnecessary.is_empty() {
        println!(
            "  ℹ declared but not in ELF needs: {}",
            unnecessary
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !unresolved.is_empty() {
        println!(
            "  ℹ sonames not resolved locally: {}",
            unresolved.join(", ")
        );
    }
}
