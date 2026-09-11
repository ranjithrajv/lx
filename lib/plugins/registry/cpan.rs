// SPDX-License-Identifier: GPL-3.0-or-later

//! CPAN (Perl) registry source plugin.
//!
//! Fetches a Perl module from CPAN and produces a local directory of files
//! ready for packaging. Uses `cpanm --installdeps` + `ExtUtils::MakeMaker`
//! to build an install tree, then stages it.
//!
//! Note: CPAN modules are typically libraries (not end-user tools). Packaging
//! them as .deb/.rpm is less common than npm/pip/gem, but useful for
//! deployment when a system Perl module version is too old.

use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::registry::{RegistryPayload, RegistrySource};

pub struct CpanRegistrySource;

impl RegistrySource for CpanRegistrySource {
    fn name(&self) -> &'static str {
        "cpan"
    }

    fn description(&self) -> &'static str {
        "CPAN (Perl) — cpanm + build install tree"
    }

    fn required_tools(&self) -> Vec<&'static str> {
        vec!["cpanm", "perl"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let mut spec = package.to_string();
        if !version.trim().is_empty() {
            // cpanm version syntax: MODULE@version
            spec = format!("{}@{}", package, version.trim());
        }

        println!("cpan: fetching {spec}");

        let workdir = tempfile::tempdir().context("failed to create cpan workdir")?;
        let build_dir = workdir.path().join("build");
        std::fs::create_dir_all(&build_dir)?;

        // cpanm installs to a local::lib style directory.
        // --notest: skip tests (we're packaging, not testing).
        // --installdeps: install dependencies so the build succeeds.
        // -L: install to a local directory within our workdir.
        let local_lib = workdir.path().join("perl5");
        let output = Command::new("cpanm")
            .args([
                "--notest",
                "--installdeps",
                "-L",
                &local_lib.to_string_lossy(),
                "--quiet",
                &spec,
            ])
            .output()
            .context("failed to run `cpanm` (is cpanm on PATH?)")?;

        if !output.status.success() {
            bail!("cpanm failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        // Now install the module itself into a DESTDIR-style tree.
        // We use `cpanm` with a custom --install_base or build manually.
        // The simplest approach: build with Makefile.PL/Build.PL, then install.
        let source_dir = workdir.path().join("source");
        std::fs::create_dir_all(&source_dir)?;

        // Download and extract the distribution tarball.
        let download = Command::new("cpanm")
            .args([
                "--notest",
                "--no-man-pages",
                "-L",
                &local_lib.to_string_lossy(),
                "--reinstall",
                "--interactive",
                "no",
                &spec,
            ])
            .output()
            .context("failed to reinstall via cpanm")?;

        if !download.status.success() {
            bail!(
                "cpanm reinstall failed: {}",
                String::from_utf8_lossy(&download.stderr)
            );
        }

        // Find what was installed by looking at the local lib.
        let site_bin = local_lib.join("bin");
        let site_arch = local_lib.join("lib").join("perl5");
        let files_dir = workdir.path().join("package");
        std::fs::create_dir_all(&files_dir)?;

        // Stage binaries from site_bin to usr/bin.
        if site_bin.is_dir() {
            let bin_dest = files_dir.join("usr").join("bin");
            std::fs::create_dir_all(&bin_dest)?;
            crate::plugins::copy_dir_recursive(&site_bin, &bin_dest)?;
        }

        // Stage library files from site_arch to usr/lib/perl5.
        if site_arch.is_dir() {
            let lib_dest = files_dir.join("usr").join("lib").join("perl5");
            std::fs::create_dir_all(&lib_dest)?;
            crate::plugins::copy_dir_recursive(&site_arch, &lib_dest)?;
        }

        // Extract version from the installed module.
        let version = extract_cpan_version(package, &local_lib);

        println!("cpan: staged {package} (version {version})");

        let description = cfg.description.clone();

        Ok(RegistryPayload {
            files_dir,
            resolved_version: version,
            description,
        })
    }
}

/// Try to determine the installed version by checking the perllocal.pod or
/// by querying the module with perl.
fn extract_cpan_version(package: &str, local_lib: &std::path::Path) -> String {
    // Convert Module::Name to Module/Name.pm
    let _module_path = package.replace("::", "/") + ".pm";
    let pod_file = local_lib.join("lib").join("perl5").join("perllocal.pod");

    if pod_file.exists() {
        if let Ok(text) = std::fs::read_to_string(&pod_file) {
            // perllocal.pod contains version info for installed modules.
            for line in text.lines() {
                if line.contains(package) {
                    // Look for VERSION: N.NNN
                    if let Some(v_start) = line.find("VERSION:") {
                        let ver = &line[v_start + 8..];
                        let ver = ver.split_whitespace().next().unwrap_or("").trim();
                        if !ver.is_empty() {
                            return ver.to_string();
                        }
                    }
                }
            }
        }
    }

    // Fallback: query the module directly.
    if let Ok(output) = Command::new("perl")
        .args([
            "-M",
            package,
            "-e",
            "print eval('$'.$package.'::VERSION') || '0.0.0'",
        ])
        .output()
    {
        let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !ver.is_empty() && ver != "0.0.0" {
            return ver;
        }
    }

    "0.0.0".to_string()
}
