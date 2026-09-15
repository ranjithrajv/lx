// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::consumer;
use crate::debs;
use crate::index::detect_host_format;
use crate::install_pkg;
use crate::manifest::Manifest;
use lx_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct InstallArgs {
    /// Package name (e.g. "eza"). The host's own repositories are probed
    /// first (native-first, unmanaged); otherwise the enabled indexes are
    /// tried and the latest-debs org (`LX_INDEX_ORG`) is the fallback.
    pub package: String,

    /// Native package format to install: `deb`, `rpm`, or `arch`. Defaults
    /// to the host's own package manager.
    #[arg(long)]
    pub format: Option<String>,

    /// Version/tag to install (defaults to the latest release).
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Target architecture (defaults to the host's, per format).
    #[arg(long)]
    pub arch: Option<String>,

    /// Target distribution/suite (defaults to the host's, per format).
    #[arg(long)]
    pub distribution: Option<String>,

    /// Download the package into this directory instead of installing it.
    #[arg(long)]
    pub download_only: Option<PathBuf>,

    /// Skip checksum verification against the release's sidecar file (not
    /// recommended).
    #[arg(long)]
    pub no_verify: bool,

    /// Proceed when the release has no sidecar checksum to verify against,
    /// instead of failing the install. Most releases don't publish a
    /// checksum sidecar, so without this the default is to refuse to
    /// install an unverified package rather than silently warn and continue.
    #[arg(long)]
    pub allow_unverified: bool,

    /// Reinstall. For an lx-managed package, re-install its recorded version
    /// (unless `--version` is given); otherwise force a reinstall even when
    /// the host manager already reports the resolved version installed.
    #[arg(long)]
    pub reinstall: bool,

    /// Skip the install confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Resolve the package from this enabled index (see `lx index list`) —
    /// prebuilt-first, with a build-from-recipe fallback. Without it, the
    /// enabled indexes are tried first and the latest-debs org is the
    /// fallback.
    #[arg(long)]
    pub source: Option<String>,

    /// Build from an index recipe, skipping the org and any prebuilt asset
    /// (index-only; always the host's native format).
    #[arg(long)]
    pub build: bool,

    /// When building from a recipe (AUR/LX community), install its missing
    /// host build dependencies before compiling, via the host package
    /// manager.
    #[arg(long)]
    pub install_build_deps: bool,
}

/// Mirrors clap's defaults so callers can use `..Default::default()`.
impl Default for InstallArgs {
    fn default() -> Self {
        Self {
            package: String::new(),
            format: None,
            version: None,
            arch: None,
            distribution: None,
            download_only: None,
            no_verify: false,
            allow_unverified: false,
            reinstall: false,
            yes: false,
            source: None,
            build: false,
            install_build_deps: false,
        }
    }
}

/// `--reinstall` without an explicit `--version` re-installs the version lx
/// recorded for this package (the former top-level `lx reinstall` behavior):
/// fill in the manifest's version/format/arch/distribution unless the user
/// overrode them. Not managed / no recorded version falls through to a
/// normal forced reinstall of the latest release.
fn apply_reinstall_defaults(mut args: InstallArgs, manifest: &Manifest) -> InstallArgs {
    if !args.reinstall {
        return args;
    }
    let Some(entry) = manifest.current(&args.package) else {
        return args;
    };
    if args.version.is_none() {
        if entry.version.trim().is_empty() {
            return args;
        }
        println!("reinstalling {} {}", args.package, entry.version);
        args.version = Some(entry.version.clone());
    }
    if args.format.is_none() && !entry.format.trim().is_empty() {
        args.format = Some(entry.format.clone());
    }
    if args.arch.is_none() && !entry.arch.trim().is_empty() {
        args.arch = Some(entry.arch.clone());
    }
    if args.distribution.is_none() && !entry.distribution.trim().is_empty() {
        args.distribution = Some(entry.distribution.clone());
    }
    args
}

pub fn run(args: InstallArgs, token: Option<&str>) -> Result<()> {
    // Decide the source order from the *user's* flags, before `reinstall`
    // fills format/arch/distribution from the manifest.
    let prefer_indexes = prefer_indexes(&args);
    if args.build && !prefer_indexes {
        bail!(
            "--build installs from a recipe, which is always the host's native \
             format; drop --format/--arch/--distribution"
        );
    }
    // Whether the user actually asked for a version, before `--reinstall`
    // fills in the recorded one. Only an explicit `--version` is a meaningful
    // tag for a build-from-recipe index (AUR has no tags).
    let explicit_version = args.version.is_some();

    let manifest = Manifest::load().unwrap_or_default();
    let args = apply_reinstall_defaults(args, &manifest);

    // `--source`: skip everything else and install from that index.
    if let Some(source) = args.source.clone() {
        return crate::index::install_from_active(
            &args.package,
            Some(&source),
            index_opts(&args, explicit_version),
        );
    }

    // Native-first: a package the host's own repositories carry is installed
    // by the host manager (`pacman -S`/`apt-get install`/`dnf install`/`apk
    // add`), not fetched from a forge release or built from an AUR recipe.
    // Such an install is deliberately *not* recorded in the lx manifest — the
    // native manager owns repository packages. Skipped for an explicit
    // source/build/target request or a download-only run, and for a package lx
    // already manages (whose recorded source should win).
    if native_first_applies(&args, &manifest, prefer_indexes) {
        let format = detect_host_format();
        if let Some(version) = consumer::native_repo_version(&args.package, format) {
            println!(
                "{} {version} is available from the host repositories",
                args.package
            );
            return consumer::install_native(&args.package, format, args.yes);
        }
        println!(
            "{} is not in the host repositories; falling back to the enabled \
             package indexes, then the latest-debs org",
            args.package
        );
    }

    // Enabled indexes are the default source; the latest-debs org is the
    // fallback. `--build` is index-only, so it never falls back to the org.
    if args.build || prefer_indexes {
        match crate::index::any_active_has(&args.package) {
            Ok(true) => {
                return crate::index::install_from_active(
                    &args.package,
                    None,
                    index_opts(&args, explicit_version),
                );
            }
            Ok(false) if args.build => bail!(
                "no enabled index has '{}'; --build needs a recipe \
                 (`lx index search {}` to check)",
                args.package,
                args.package
            ),
            Ok(false) => {}
            Err(e) if args.build => {
                return Err(e).with_context(|| "index lookup failed for --build");
            }
            Err(e) => eprintln!(
                "⚠ index lookup failed: {e:#}; trying {}",
                consumer::index_org()
            ),
        }
    }

    install_from_org(&args, token)
}

/// True when the enabled indexes should be tried before the latest-debs org.
///
/// An explicit `--format`/`--arch`/`--distribution` is an org-only request
/// (index installs are always the host's native format), so those go straight
/// to the org. Reinstall-filled defaults do not count — this is evaluated
/// before [`apply_reinstall_defaults`].
fn prefer_indexes(args: &InstallArgs) -> bool {
    args.format.is_none() && args.arch.is_none() && args.distribution.is_none()
}

/// True when `lx install` should probe the host's own repositories before the
/// enabled indexes / latest-debs org.
///
/// Only the plain default path qualifies: an explicit `--source`, `--build`,
/// `--format`/`--arch`/`--distribution`, `--version`, or `--download-only`
/// all name a different source of truth. A package lx already manages keeps
/// its recorded source rather than being silently handed to the host manager.
/// `index_preferred` is [`prefer_indexes`] evaluated on the user's original
/// flags (before `--reinstall` fills them from the manifest).
fn native_first_applies(args: &InstallArgs, manifest: &Manifest, index_preferred: bool) -> bool {
    index_preferred
        && !args.build
        && args.source.is_none()
        && args.version.is_none()
        && args.download_only.is_none()
        && manifest.current(&args.package).is_none()
}

/// Map the consumer install flags onto the shared index install options.
///
/// `explicit_version` distinguishes a user-supplied `--version` from one
/// filled in by `--reinstall`. A build-from-recipe index (AUR) has no tags,
/// so forwarding a reinstall-recorded version would make it refuse with
/// "AUR has no prebuilt tags".
fn index_opts(args: &InstallArgs, explicit_version: bool) -> crate::index::InstallOpts {
    crate::index::InstallOpts {
        tag: if explicit_version {
            args.version.clone()
        } else {
            None
        },
        build: args.build,
        no_verify: args.no_verify,
        allow_unverified: args.allow_unverified,
        yes: args.yes,
        download_only: args.download_only.clone(),
        install_build_deps: args.install_build_deps,
    }
}

/// The `latest-debs` org path: resolve `<package>-debian`'s release, match the
/// host asset, download, verify, install, and record.
fn install_from_org(args: &InstallArgs, token: Option<&str>) -> Result<()> {
    let format = match &args.format {
        Some(f) => consumer::parse_format(f)?,
        None => detect_host_format(),
    };
    let org = consumer::index_org();
    let repo = consumer::repo_name(&args.package);

    let client = GitHubClient::new(token.map(|s| s.to_string()))?;

    let release = match &args.version {
        Some(v) => match client.release_by_tag(&org, &repo, v) {
            Ok(r) => r,
            Err(e) => {
                debs::suggest_versions(&client, &org, &repo, v);
                return Err(e);
            }
        },
        None => client.latest_release(&org, &repo).with_context(|| {
            format!(
                "no releases found for '{org}/{repo}'. Is '{}' published under \
                 https://github.com/orgs/{org}/repositories ?",
                args.package
            )
        })?,
    };

    let arch = match &args.arch {
        Some(a) => a.clone(),
        None => install_pkg::detect_arch(format)?,
    };
    let dist = match &args.distribution {
        Some(d) => d.clone(),
        None => consumer::host_dist(format).unwrap_or_default(),
    };

    let resolved = consumer::resolve_asset(&release, &args.package, format, &arch, &dist)
        .ok_or_else(|| {
            anyhow!(
                "no {} asset for {arch}/{dist} in release '{}'. Available:\n  {}",
                format.name(),
                release.tag_name,
                release
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            )
        })?;
    if resolved.musl_fallback {
        println!("  (no {dist}-specific build; using musl-static binary — runs on any Linux)");
    }
    let asset = resolved.asset;
    let control_version = resolved.version;

    if args.download_only.is_none() && !args.reinstall {
        if let Some(installed) = consumer::installed_version(&args.package, format) {
            if installed == control_version {
                println!(
                    "{} is already at {control_version}; nothing to do (use --reinstall to force)",
                    args.package
                );
                return Ok(());
            }
        }
    }

    let target = if dist.is_empty() {
        format.name().to_string()
    } else {
        dist.clone()
    };
    println!(
        "Found {} ({}) for {arch}/{target}",
        asset.name,
        debs::human_size(asset.size.unwrap_or(0))
    );

    let dest_dir = args
        .download_only
        .clone()
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("failed to create '{}'", dest_dir.display()))?;
    let dest = dest_dir.join(&asset.name);
    println!("  ↓ downloading {}", asset.name);
    debs::download(&client, asset, &dest)?;

    if !args.no_verify {
        debs::verify_sidecar_or_require_flag(&client, asset, &dest, args.allow_unverified)?;
    }

    if args.download_only.is_some() {
        println!("Downloaded to {}", dest.display());
        return Ok(());
    }

    consumer::install_and_record(
        &dest,
        &asset.name,
        &args.package,
        format,
        control_version,
        arch,
        dist,
        release.tag_name.clone(),
        args.yes,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::PackageEntry;

    fn recorded() -> PackageEntry {
        PackageEntry {
            version: "1.2.3-1+trixie".into(),
            arch: "amd64".into(),
            distribution: "trixie".into(),
            asset: "eza_1.2.3-1+trixie_amd64.deb".into(),
            tag: "v1.2.3".into(),
            installed_at: "t".into(),
            format: "deb".into(),
        }
    }

    fn args(reinstall: bool) -> InstallArgs {
        InstallArgs {
            package: "eza".into(),
            format: None,
            version: None,
            arch: None,
            distribution: None,
            download_only: None,
            no_verify: false,
            allow_unverified: false,
            reinstall,
            yes: true,
            source: None,
            build: false,
            install_build_deps: false,
        }
    }

    fn manifest_with_eza() -> Manifest {
        let mut m = Manifest::default();
        m.record("eza", recorded());
        m
    }

    #[test]
    fn reinstall_fills_recorded_version_and_target() {
        let got = apply_reinstall_defaults(args(true), &manifest_with_eza());
        assert_eq!(got.version.as_deref(), Some("1.2.3-1+trixie"));
        assert_eq!(got.format.as_deref(), Some("deb"));
        assert_eq!(got.arch.as_deref(), Some("amd64"));
        assert_eq!(got.distribution.as_deref(), Some("trixie"));
    }

    #[test]
    fn reinstall_explicit_version_and_format_win() {
        let mut a = args(true);
        a.version = Some("9.9.9".into());
        a.format = Some("rpm".into());
        let got = apply_reinstall_defaults(a, &manifest_with_eza());
        assert_eq!(got.version.as_deref(), Some("9.9.9"));
        assert_eq!(got.format.as_deref(), Some("rpm"));
        // Unset fields are still filled from the manifest.
        assert_eq!(got.arch.as_deref(), Some("amd64"));
    }

    #[test]
    fn reinstall_of_unmanaged_package_is_a_noop() {
        let got = apply_reinstall_defaults(args(true), &Manifest::default());
        assert!(got.version.is_none() && got.format.is_none());
    }

    #[test]
    fn without_reinstall_flag_recorded_version_is_ignored() {
        let got = apply_reinstall_defaults(args(false), &manifest_with_eza());
        assert!(got.version.is_none());
    }

    #[test]
    fn indexes_are_preferred_unless_a_format_target_is_given() {
        assert!(prefer_indexes(&args(false)));

        let mut by_format = args(false);
        by_format.format = Some("rpm".into());
        assert!(!prefer_indexes(&by_format));

        let mut by_arch = args(false);
        by_arch.arch = Some("arm64".into());
        assert!(!prefer_indexes(&by_arch));

        let mut by_dist = args(false);
        by_dist.distribution = Some("trixie".into());
        assert!(!prefer_indexes(&by_dist));
    }

    #[test]
    fn build_stays_on_the_index_path() {
        // `--build` does not itself disable index preference; it forces the
        // index path in `run()` and never falls back to the org.
        let mut build = args(false);
        build.build = true;
        assert!(prefer_indexes(&build));
    }

    #[test]
    fn native_first_applies_to_the_plain_default_path() {
        let m = Manifest::default();
        assert!(native_first_applies(&args(false), &m, true));
    }

    #[test]
    fn native_first_is_skipped_for_an_explicit_request() {
        let m = Manifest::default();

        let mut build = args(false);
        build.build = true;
        assert!(!native_first_applies(&build, &m, true));

        let mut sourced = args(false);
        sourced.source = Some("aur".into());
        assert!(!native_first_applies(&sourced, &m, true));

        let mut versioned = args(false);
        versioned.version = Some("1.0".into());
        assert!(!native_first_applies(&versioned, &m, true));

        let mut download = args(false);
        download.download_only = Some(std::env::temp_dir());
        assert!(!native_first_applies(&download, &m, true));

        // A format/arch/dist target disables index preference; native-first
        // follows the same gate.
        assert!(!native_first_applies(&args(false), &m, false));
    }

    #[test]
    fn native_first_defers_to_an_lx_managed_package() {
        // lx already owns this package from a recorded source, so a plain
        // `lx install` must not silently hand it to the host manager.
        assert!(!native_first_applies(
            &args(false),
            &manifest_with_eza(),
            true
        ));
    }
}
