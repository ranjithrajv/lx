// SPDX-License-Identifier: GPL-3.0-or-later

use crate::plugins::PACKAGED_FROM_LINE;
use anyhow::{Context, Result};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Package metadata needed to render the source package templates.
pub struct Pkg {
    pub name: String,
    pub github_repo: String,
    pub description: String,
    pub maintainer: String,
    pub version: String,
    pub build_version: String,
    /// Debian epoch (e.g. "1"), matching the binary package's -- see
    /// `lx_lib::pkgmeta::with_epoch`. Empty means none.
    pub epoch: String,
    pub license_spdx: String,
    /// Dependency-relation fields, matching the binary package's -- a
    /// source package's `debian/control` binary-package stanza needs the
    /// same `Depends:`/etc. a `dpkg-buildpackage` build of this tree would
    /// need to reproduce the shipped binary .deb.
    pub depends: String,
    pub recommends: String,
    pub suggests: String,
    pub conflicts: String,
    pub replaces: String,
    pub provides: String,
    pub breaks: String,
    pub predepends: String,
    pub section: String,
    pub priority: String,
    pub fields: std::collections::HashMap<String, String>,
    /// Unix epoch seconds the release was published, for reproducible
    /// changelog/copyright timestamps -- see
    /// `lx_lib::pkgmeta::reproducible_epoch`. Matching the binary
    /// package's `job.published_at`.
    pub published_at: Option<i64>,
    /// The upstream license actually detected from GitHub, when available
    /// -- preferred over `license_spdx`/generic text in the rendered
    /// copyright, matching the binary package's behavior.
    pub license: Option<lx_lib::github::RepoLicense>,
}

/// Generate Debian source packages (3.0 quilt) for each distribution among
/// the built .debs in `out_dir`, mirroring the action's
/// `build_source_packages`:
///   * architecture-independent, so generated once per dist,
///   * built entirely in-process (`lx_lib::debarchive`) -- no
///     `dpkg-source`/`dpkg-deb` subprocess,
///   * best-effort: on failure, the binary builds still stand.
pub fn generate(out_dir: &Path, pkg: &Pkg) -> Result<()> {
    let dists = unique_dists(out_dir, &pkg.name)?;
    if dists.is_empty() {
        println!(
            "(no source packages: no built .debs found in {})",
            out_dir.display()
        );
        return Ok(());
    }
    println!(
        "Generating Debian source packages for: {}",
        dists.join(", ")
    );

    let debian_version = lx_lib::pkgmeta::strip_upstream_prefix(&pkg.version);
    let workdir = tempfile::tempdir().context("failed to create temp dir")?;
    let mut orig_done = false;

    for dist in &dists {
        match build_source_package(
            out_dir,
            workdir.path(),
            pkg,
            &debian_version,
            dist,
            orig_done,
        ) {
            Ok(created_orig) => orig_done = orig_done || created_orig,
            Err(e) => {
                eprintln!("  ⚠️  source package for {dist} failed: {e:#}");
            }
        }
    }
    Ok(())
}

/// Collect the unique distributions present in the built .debs by parsing
/// `<pkg>_<debian_version>-<build>+<dist>_<arch>.deb` filenames.
fn unique_dists(out_dir: &Path, pkg_name: &str) -> Result<Vec<String>> {
    let mut dists: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(out_dir).context("failed to read output dir")?;
    let prefix = format!("{pkg_name}_");
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(".deb") {
            continue;
        }
        // eza_0.23.5-1+bookworm_amd64.deb -> strip prefix, then <ver>-<build>+<dist>_<arch>
        let stem = name.trim_end_matches(".deb");
        let rest = stem.trim_start_matches(&prefix);
        let rest = rest.rsplit_once('_').map(|(r, _)| r).unwrap_or(rest);
        let dist = rest.rsplit('+').next().unwrap_or_default();
        if !dist.is_empty() && !dists.iter().any(|d| d == dist) {
            dists.push(dist.to_string());
        }
    }
    Ok(dists)
}

/// One `Checksums-*`/`Files` entry: the `.dsc` lists both tarballs, orig
/// always before debian (dpkg-source's own convention, verified against a
/// real `dpkg-source -b` run -- not alphabetical, which would order them
/// the other way).
pub struct DscFile {
    name: String,
    size: u64,
    md5: String,
    sha1: String,
    sha256: String,
}

impl DscFile {
    pub fn from_bytes(name: String, data: &[u8]) -> Self {
        let md5 = format!("{:x}", md5::compute(data));
        let sha1 = {
            let mut h = Sha1::new();
            h.update(data);
            hex::encode(h.finalize())
        };
        let sha256 = {
            let mut h = Sha256::new();
            h.update(data);
            hex::encode(h.finalize())
        };
        DscFile {
            name,
            size: data.len() as u64,
            md5,
            sha1,
            sha256,
        }
    }
}

/// Build one source package for a single distribution. Returns Ok(true)
/// when this call created the shared `.orig.tar.xz`.
fn build_source_package(
    out_dir: &Path,
    workdir: &Path,
    pkg: &Pkg,
    debian_version: &str,
    dist: &str,
    orig_done: bool,
) -> Result<bool> {
    let src_version = format!("{}-{}+{dist}", debian_version, pkg.build_version);
    // Epoch appears in Version: fields (changelog, .dsc) but never in
    // filenames -- Debian policy excludes it there since `:` isn't
    // filename-safe. `src_version` above stays epoch-free for filenames;
    // this is the one used for file *content*.
    let content_version = lx_lib::pkgmeta::with_epoch(&pkg.epoch, &src_version);
    let tree = format!("{}-{src_version}", pkg.name);
    let tree_dir = workdir.join(&tree);
    let orig_name = format!("{}_{debian_version}.orig.tar.xz", pkg.name);
    let debian_tar_name = format!("{}_{src_version}.debian.tar.xz", pkg.name);
    let dsc_name = format!("{}_{src_version}.dsc", pkg.name);

    // Reference .deb: prefer amd64 for this dist, else the first match.
    let ref_deb = find_ref_deb(out_dir, &pkg.name, dist)?
        .ok_or_else(|| anyhow::anyhow!("no .deb for dist {dist}"))?;

    std::fs::create_dir_all(tree_dir.join("debian/source"))
        .context("failed to create debian/source")?;

    // Extract the binary package as the source tree. DEBIAN/ and
    // usr/share/doc are Debian packaging output, not upstream payload, so
    // strip them.
    lx_lib::debarchive::extract(&out_dir.join(&ref_deb), &tree_dir)
        .with_context(|| format!("failed to extract {ref_deb}"))?;
    let _ = std::fs::remove_dir_all(tree_dir.join("DEBIAN"));
    let _ = std::fs::remove_dir_all(tree_dir.join("usr/share/doc"));

    // debian/control (mirrors templates/source/control). The binary-package
    // stanza's relation fields mirror the shipped .deb's -- a
    // `dpkg-buildpackage` build of this tree needs the same Depends/etc. to
    // reproduce it.
    let relations = lx_lib::pkgmeta::Relations {
        depends: pkg.depends.clone(),
        recommends: pkg.recommends.clone(),
        suggests: pkg.suggests.clone(),
        conflicts: pkg.conflicts.clone(),
        replaces: pkg.replaces.clone(),
        provides: pkg.provides.clone(),
        breaks: pkg.breaks.clone(),
        predepends: pkg.predepends.clone(),
    }
    .render();
    let homepage = lx_lib::constants::homepage_for_github(&pkg.github_repo);
    let sec = if pkg.section.trim().is_empty() {
        lx_lib::constants::DEFAULT_SECTION
    } else {
        pkg.section.trim()
    };
    let pri = if pkg.priority.trim().is_empty() {
        lx_lib::constants::DEFAULT_PRIORITY
    } else {
        pkg.priority.trim()
    };
    let extra_fields = crate::plugins::render_extra_fields(&pkg.fields);
    let control = format!(
        "Source: {name}\nSection: {section}\nPriority: {priority}\nMaintainer: {m}\nHomepage: {homepage}\nStandards-Version: {standards}\nBuild-Depends: {build_depends}\n\nPackage: {name}\nArchitecture: any\nDescription: {desc}\n{PACKAGED_FROM_LINE}\n{relations}{extra_fields}",
        name = pkg.name,
        section = sec,
        priority = pri,
        m = pkg.maintainer,
        standards = lx_lib::constants::STANDARDS_VERSION,
        build_depends = lx_lib::constants::DEBHELPER_COMPAT,
        desc = pkg.description,
    );
    std::fs::write(tree_dir.join("debian/control"), control)?;

    // debian/rules (mirrors templates/source/rules).
    std::fs::write(
        tree_dir.join("debian/rules"),
        "#!/usr/bin/make -f\n\nPKG := $(shell grep '^Package:' debian/control | head -1 | awk '{print $$2}')\n\n%:\n\tdh $@\n\noverride_dh_auto_build:\n\noverride_dh_auto_install:\n\tdh_installdirs\n\tcp -a usr/* debian/$(PKG)/\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            tree_dir.join("debian/rules"),
            std::fs::Permissions::from_mode(lx_lib::constants::DIR_MODE),
        )?;
    }

    // debian/changelog: shares the exact renderer (and thus the exact
    // reproducible-builds-aware date derivation) the binary .deb's
    // changelog uses, rather than an independent hardcoded date.
    let changelog = lx_lib::pkgmeta::render_changelog_entry(
        &pkg.name,
        &content_version,
        dist,
        debian_version,
        &pkg.maintainer,
        pkg.published_at,
    );
    std::fs::write(tree_dir.join("debian/changelog"), changelog)?;

    // debian/copyright: same renderer as the binary .deb's, so the source
    // package gets the full upstream license text/year handling instead of
    // a hardcoded year and a simplified, less-complete template.
    let copyright = lx_lib::pkgmeta::render_copyright(
        &pkg.name,
        &pkg.github_repo,
        pkg.license.as_ref(),
        &pkg.license_spdx,
        pkg.published_at,
    );
    std::fs::write(tree_dir.join("debian/copyright"), copyright)?;

    // debian/source/format
    std::fs::write(tree_dir.join("debian/source/format"), "3.0 (quilt)\n")?;

    // The upstream orig tarball is built once and shared across dists. It
    // must contain a top-level <pkg>-<debian_version>/ directory (standard
    // upstream layout).
    //
    // Reproducible-builds hygiene: `lx_lib::debarchive::tar_xz_tree` walks
    // in sorted order and normalizes owner/group -- see its own doc
    // comment. `mtime` respects SOURCE_DATE_EPOCH (the reproducible-builds
    // .org standard) when set, else the release's own publish time, else a
    // fixed epoch -- the same precedence the binary .deb's mtime uses (see
    // `lx_lib::pkgmeta::reproducible_epoch`); previously this only ever
    // consulted SOURCE_DATE_EPOCH and fell straight to a fixed epoch,
    // ignoring `published_at` entirely. (xz itself has no mtime field to
    // normalize, unlike gzip -- only the tar entries carry one.)
    let source_date_epoch = lx_lib::pkgmeta::reproducible_epoch(pkg.published_at);
    let orig_path = workdir.join(&orig_name);
    let created_orig = if !orig_done {
        let upstream_dir = format!("{}-{debian_version}", pkg.name);
        let orig_bytes = lx_lib::debarchive::tar_xz_tree(
            &tree_dir.join("usr"),
            &format!("{upstream_dir}/usr/"),
            source_date_epoch,
        )
        .context("failed to build orig tarball")?;
        std::fs::write(&orig_path, &orig_bytes)?;
        true
    } else {
        false
    };
    let orig_bytes = std::fs::read(&orig_path)
        .with_context(|| format!("failed to read '{}'", orig_path.display()))?;

    // debian.tar.xz: just the debian/ metadata directory -- lx never
    // produces quilt patches, so there's no .pc/ or patches/ to include.
    let debian_bytes =
        lx_lib::debarchive::tar_xz_tree(&tree_dir.join("debian"), "debian/", source_date_epoch)
            .context("failed to build debian.tar.xz")?;
    std::fs::write(workdir.join(&debian_tar_name), &debian_bytes)?;

    let orig_file = DscFile::from_bytes(orig_name.clone(), &orig_bytes);
    let debian_file = DscFile::from_bytes(debian_tar_name.clone(), &debian_bytes);
    let dsc = render_dsc(pkg, &content_version, &orig_file, &debian_file);
    std::fs::write(workdir.join(&dsc_name), &dsc)?;

    std::fs::copy(workdir.join(&dsc_name), out_dir.join(&dsc_name))?;
    std::fs::copy(
        workdir.join(&debian_tar_name),
        out_dir.join(&debian_tar_name),
    )?;
    if created_orig {
        std::fs::copy(&orig_path, out_dir.join(&orig_name))?;
    }
    println!("  ✓ source package: {dsc_name}");
    Ok(created_orig)
}

/// Render a `.dsc` matching `dpkg-source -b`'s own field order and
/// content, verified field-for-field against a real `dpkg-source -b` run
/// (see `docs/decisions/2026-08-20-docker-free-lintian-source.md`).
/// Unsigned, matching the previous behavior (which never
/// signed either).
pub fn render_dsc(pkg: &Pkg, content_version: &str, orig: &DscFile, debian: &DscFile) -> String {
    let homepage = lx_lib::constants::homepage_for_github(&pkg.github_repo);
    let sec = if pkg.section.trim().is_empty() {
        lx_lib::constants::DEFAULT_SECTION
    } else {
        pkg.section.trim()
    };
    let pri = if pkg.priority.trim().is_empty() {
        lx_lib::constants::DEFAULT_PRIORITY
    } else {
        pkg.priority.trim()
    };
    format!(
        "Format: 3.0 (quilt)\n\
          Source: {name}\n\
          Binary: {name}\n\
          Architecture: any\n\
          Version: {content_version}\n\
          Maintainer: {maintainer}\n\
          Homepage: {homepage}\n\
          Standards-Version: {standards}\n\
          Build-Depends: {build_depends}\n\
          Package-List:\n\
          \x20{name} deb {section} {priority} arch=any\n\
          Checksums-Sha1:\n\
          \x20{o_sha1} {o_size} {o_name}\n\
          \x20{d_sha1} {d_size} {d_name}\n\
          Checksums-Sha256:\n\
          \x20{o_sha256} {o_size} {o_name}\n\
          \x20{d_sha256} {d_size} {d_name}\n\
          Files:\n\
          \x20{o_md5} {o_size} {o_name}\n\
          \x20{d_md5} {d_size} {d_name}\n",
        name = pkg.name,
        maintainer = pkg.maintainer,
        homepage = homepage,
        standards = lx_lib::constants::STANDARDS_VERSION,
        build_depends = lx_lib::constants::DEBHELPER_COMPAT,
        section = sec,
        priority = pri,
        o_sha1 = orig.sha1,
        o_size = orig.size,
        o_name = orig.name,
        d_sha1 = debian.sha1,
        d_size = debian.size,
        d_name = debian.name,
        o_sha256 = orig.sha256,
        d_sha256 = debian.sha256,
        o_md5 = orig.md5,
        d_md5 = debian.md5,
    )
}

/// Find a reference .deb for a dist: prefer amd64, else any matching dist.
fn find_ref_deb(out_dir: &Path, pkg_name: &str, dist: &str) -> Result<Option<String>> {
    let mut any: Option<String> = None;
    for entry in std::fs::read_dir(out_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&format!("{pkg_name}_")) || !name.ends_with(".deb") {
            continue;
        }
        let marker = format!("+{dist}_");
        if name.contains(&marker) {
            if name.ends_with(&format!("+{dist}_amd64.deb")) {
                return Ok(Some(name));
            }
            if any.is_none() {
                any = Some(name);
            }
        }
    }
    Ok(any)
}

// ---------------------------------------------------------------------------
// RPM source packages (.src.rpm)
// ---------------------------------------------------------------------------

/// Generate a `.src.rpm` (spec file + source tarball, built entirely
/// in-process via `lx_lib::rpmarchive::build_srpm`) for each distribution
/// among the built `.rpm`s in `out_dir`. Mirrors [`generate`]'s shape: the
/// source tarball content (the extracted upstream payload under `usr/`) is
/// identical across dists and built once, reused across the per-dist spec
/// files -- same rationale as the shared `.orig.tar.xz` above.
pub fn generate_rpm(out_dir: &Path, pkg: &Pkg) -> Result<()> {
    let dists = unique_dists_rpm(out_dir, &pkg.name)?;
    if dists.is_empty() {
        println!(
            "(no rpm source packages: no built .rpm found in {})",
            out_dir.display()
        );
        return Ok(());
    }
    println!("Generating RPM source packages for: {}", dists.join(", "));

    let version = lx_lib::pkgmeta::strip_upstream_prefix(&pkg.version);
    let source_date_epoch = lx_lib::pkgmeta::reproducible_epoch(pkg.published_at);
    let source_name = format!("{}-{version}.tar.xz", pkg.name);
    let workdir = tempfile::tempdir().context("failed to create temp dir")?;
    let mut source_bytes: Option<Vec<u8>> = None;

    for dist in &dists {
        match build_srpm_for_dist(
            out_dir,
            workdir.path(),
            pkg,
            &version,
            dist,
            source_date_epoch,
            &mut SharedSourceTarball {
                name: &source_name,
                bytes: &mut source_bytes,
            },
        ) {
            Ok(()) => {}
            Err(e) => eprintln!("  ⚠️  rpm source package for {dist} failed: {e:#}"),
        }
    }
    Ok(())
}

/// The shared source tarball for a `.src.rpm` build: its `Source0` filename
/// plus a cache slot filled on first use and reused across distributions.
struct SharedSourceTarball<'a> {
    name: &'a str,
    bytes: &'a mut Option<Vec<u8>>,
}

/// Build one `.src.rpm` for a single distribution. `source.bytes` caches the
/// shared source tarball across calls (built on first use).
fn build_srpm_for_dist(
    out_dir: &Path,
    workdir: &Path,
    pkg: &Pkg,
    version: &str,
    dist: &str,
    source_date_epoch: i64,
    source: &mut SharedSourceTarball,
) -> Result<()> {
    let (ref_rpm, release) = find_ref_rpm(out_dir, &pkg.name, version, dist)?
        .ok_or_else(|| anyhow::anyhow!("no .rpm for dist {dist}"))?;

    if source.bytes.is_none() {
        let tree_dir = workdir.join("tree");
        lx_lib::rpmarchive::extract(&out_dir.join(&ref_rpm), &tree_dir)
            .with_context(|| format!("failed to extract {ref_rpm}"))?;
        let _ = std::fs::remove_dir_all(tree_dir.join("usr/share/doc"));
        let bytes =
            lx_lib::debarchive::tar_xz_tree(&tree_dir.join("usr"), "usr/", source_date_epoch)
                .context("failed to build source tarball")?;
        *source.bytes = Some(bytes);
    }
    let source_bytes = source.bytes.as_ref().unwrap();

    let summary = pkg.description.clone();
    let homepage = lx_lib::constants::homepage_for_github(&pkg.github_repo);
    let description = format!(
        "{summary}\nPackaged from the upstream release ({homepage}) for RPM-based distributions."
    );
    let license = if pkg.license_spdx.trim().is_empty() {
        "NOASSERTION"
    } else {
        pkg.license_spdx.trim()
    };
    let meta = lx_lib::rpmarchive::PackageMeta {
        name: &pkg.name,
        version,
        release: &release,
        summary: &summary,
        description: &description,
        license,
        vendor: None,
        packager: Some(&pkg.maintainer),
    };
    let spec_name = format!("{}.spec", pkg.name);
    let spec = render_rpm_spec(
        &meta,
        &pkg.maintainer,
        homepage.as_str(),
        source.name,
        source_date_epoch,
    );

    let srpm_name = format!("{}-{version}-{release}.src.rpm", pkg.name);
    let srpm_path = out_dir.join(&srpm_name);
    lx_lib::rpmarchive::build_srpm(
        &meta,
        (&spec_name, spec.as_bytes()),
        (source.name, source_bytes),
        source_date_epoch,
        &srpm_path,
    )
    .with_context(|| format!("failed to build {srpm_name}"))?;
    println!("  ✓ rpm source package: {srpm_name}");
    Ok(())
}

/// Collect the unique dists present in the built `.rpm`s by parsing
/// `<pkg>-<version>-<build>.<dist>.<arch>.rpm` filenames (see
/// `src/plugins/rpm.rs`'s `release` computation).
fn unique_dists_rpm(out_dir: &Path, pkg_name: &str) -> Result<Vec<String>> {
    let mut dists: Vec<String> = Vec::new();
    let prefix = format!("{pkg_name}-");
    for entry in std::fs::read_dir(out_dir).context("failed to read output dir")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(".rpm") || name.ends_with(".src.rpm") {
            continue;
        }
        let stem = name.trim_end_matches(".rpm");
        let Some((rest, _arch)) = stem.rsplit_once('.') else {
            continue;
        };
        let Some((_, dist)) = rest.rsplit_once('.') else {
            continue;
        };
        if !dists.iter().any(|d| d == dist) {
            dists.push(dist.to_string());
        }
    }
    Ok(dists)
}

/// Find a reference `.rpm` for a dist (prefer x86_64, else any match) and
/// parse its `release` field back out of the filename.
fn find_ref_rpm(
    out_dir: &Path,
    pkg_name: &str,
    version: &str,
    dist: &str,
) -> Result<Option<(String, String)>> {
    let prefix = format!("{pkg_name}-{version}-");
    let mut any: Option<(String, String)> = None;
    for entry in std::fs::read_dir(out_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(".rpm") || name.ends_with(".src.rpm") {
            continue;
        }
        let stem = name.trim_end_matches(".rpm");
        let Some(rest) = stem.strip_prefix(&prefix) else {
            continue;
        };
        // rest = "<release>.<arch>" where release = "<build>.<dist>".
        let Some((release, arch)) = rest.rsplit_once('.') else {
            continue;
        };
        if !release.ends_with(&format!(".{dist}")) {
            continue;
        }
        let release = release.to_string();
        if arch == "x86_64" {
            return Ok(Some((name.clone(), release)));
        }
        if any.is_none() {
            any = Some((name, release));
        }
    }
    Ok(any)
}

/// Render a minimal, valid `.spec` for the source RPM. Mirrors the fidelity
/// of the binary rpm plugin (`src/plugins/rpm.rs`): no `Requires:`/relations
/// rendering, since the binary plugin doesn't wire those either.
fn render_rpm_spec(
    meta: &lx_lib::rpmarchive::PackageMeta,
    maintainer: &str,
    homepage: &str,
    source_name: &str,
    source_date_epoch: i64,
) -> String {
    let name = meta.name;
    let version = meta.version;
    let release = meta.release;
    let changelog_date = jiff::Timestamp::from_second(source_date_epoch)
        .map(|t| t.strftime("%a %b %d %Y").to_string())
        .unwrap_or_default();
    format!(
        "Name: {name}\n\
         Version: {version}\n\
         Release: {release}\n\
         Summary: {summary}\n\
         License: {license}\n\
         URL: {homepage}\n\
         Source0: {source_name}\n\
         \n\
         %description\n\
         {description}\n\
         \n\
         %prep\n\
         %setup -q -c -n {name}-{version}\n\
         \n\
         %build\n\
         # Pre-built upstream binary; nothing to compile.\n\
         \n\
         %install\n\
         rm -rf %{{buildroot}}\n\
         mkdir -p %{{buildroot}}\n\
         cp -a usr %{{buildroot}}/\n\
         \n\
         %files\n\
         /usr/*\n\
         \n\
         %changelog\n\
         * {changelog_date} {maintainer}\n\
         - Packaged {version}-{release} from the upstream release for RPM-based distributions.\n",
        summary = meta.summary,
        license = meta.license,
        description = meta.description,
    )
}

// ---------------------------------------------------------------------------
// Arch Linux PKGBUILD
// ---------------------------------------------------------------------------

/// Generate a `PKGBUILD` (plus its source tarball) from a built
/// `.pkg.tar.zst` in `out_dir`. Unlike deb/rpm, Arch has no compiled
/// "source package" format -- a real Arch source package *is* a `PKGBUILD`
/// text file (see AUR convention), so there is no archive to build here.
pub fn generate_arch(out_dir: &Path, pkg: &Pkg) -> Result<()> {
    let version = lx_lib::pkgmeta::strip_upstream_prefix(&pkg.version);
    let Some((ref_pkg, release)) = find_ref_arch(out_dir, &pkg.name, &version, &pkg.epoch)? else {
        println!(
            "(no arch PKGBUILD: no built .pkg.tar.zst found in {})",
            out_dir.display()
        );
        return Ok(());
    };
    println!("Generating Arch PKGBUILD from {ref_pkg}");

    let source_date_epoch = lx_lib::pkgmeta::reproducible_epoch(pkg.published_at);
    let workdir = tempfile::tempdir().context("failed to create temp dir")?;
    let tree_dir = workdir.path().join("tree");
    lx_lib::archarchive::extract(&out_dir.join(&ref_pkg), &tree_dir)
        .with_context(|| format!("failed to extract {ref_pkg}"))?;
    let _ = std::fs::remove_dir_all(tree_dir.join("usr/share/doc"));

    let source_name = format!("{}-{version}.tar.xz", pkg.name);
    let source_bytes =
        lx_lib::debarchive::tar_xz_tree(&tree_dir.join("usr"), "usr/", source_date_epoch)
            .context("failed to build source tarball")?;
    std::fs::write(out_dir.join(&source_name), &source_bytes)
        .with_context(|| format!("failed to write '{source_name}'"))?;

    let sha256 = {
        let mut h = Sha256::new();
        h.update(&source_bytes);
        hex::encode(h.finalize())
    };
    let homepage = lx_lib::constants::homepage_for_github(&pkg.github_repo);
    let license = if pkg.license_spdx.trim().is_empty() {
        "custom"
    } else {
        pkg.license_spdx.trim()
    };
    let pkgbuild = render_pkgbuild(
        pkg,
        &version,
        &release,
        &homepage,
        license,
        &source_name,
        &sha256,
    );
    let pkgbuild_path = out_dir.join("PKGBUILD");
    std::fs::write(&pkgbuild_path, pkgbuild)
        .with_context(|| format!("failed to write '{}'", pkgbuild_path.display()))?;
    println!("  ✓ PKGBUILD written ({source_name})");
    Ok(())
}

/// Find a reference `.pkg.tar.zst` (prefer x86_64, else any match) and parse
/// its `release` field back out of the filename (see `src/plugins/arch.rs`).
///
/// The filename is `{name}-[{epoch}:]{version}-{release}-{arch}.pkg.tar.zst`,
/// so the epoch (when present) sits between the name and the release.
fn find_ref_arch(
    out_dir: &Path,
    pkg_name: &str,
    version: &str,
    epoch: &str,
) -> Result<Option<(String, String)>> {
    let prefix = format!("{pkg_name}-");
    let version_prefix = if epoch.trim().is_empty() {
        format!("{version}-")
    } else {
        format!("{}:{version}-", epoch.trim())
    };
    let mut any: Option<(String, String)> = None;
    for entry in std::fs::read_dir(out_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(".pkg.tar.zst") {
            continue;
        }
        let stem = name.trim_end_matches(".pkg.tar.zst");
        let Some(rest) = stem.strip_prefix(&prefix) else {
            continue;
        };
        // rest = "[epoch:]{version}-{release}-{arch}".
        let Some(rest) = rest.strip_prefix(&version_prefix) else {
            continue;
        };
        let Some((release, arch)) = rest.rsplit_once('-') else {
            continue;
        };
        let release = release.to_string();
        if arch == "x86_64" {
            return Ok(Some((name.clone(), release)));
        }
        if any.is_none() {
            any = Some((name, release));
        }
    }
    Ok(any)
}

/// Render a `PKGBUILD` matching makepkg's expected field order.
fn render_pkgbuild(
    pkg: &Pkg,
    version: &str,
    release: &str,
    homepage: &str,
    license: &str,
    source_name: &str,
    sha256: &str,
) -> String {
    let epoch = if pkg.epoch.trim().is_empty() {
        String::new()
    } else {
        format!("epoch={}\n", pkg.epoch.trim())
    };
    let translate = crate::archarchive::debian_relations_to_pacman;
    let depends = quote_array(translate(&pkg.depends));
    let mut optdepends = translate(&pkg.recommends);
    optdepends.extend(translate(&pkg.suggests));
    let optdepends = quote_array(optdepends);
    let provides = quote_array(translate(&pkg.provides));
    let mut conflicts = translate(&pkg.conflicts);
    conflicts.extend(translate(&pkg.breaks));
    let conflicts = quote_array(conflicts);
    let replaces = quote_array(translate(&pkg.replaces));

    let mut relations = String::new();
    for (key, value) in [
        ("depends", depends),
        ("optdepends", optdepends),
        ("provides", provides),
        ("conflicts", conflicts),
        ("replaces", replaces),
    ] {
        if !value.is_empty() {
            relations.push_str(&format!("{key}=({value})\n"));
        }
    }

    format!(
        "# Packaged from the upstream release for Arch Linux (generated by lx).\n\
         pkgname={name}\n\
         pkgver={version}\n\
         pkgrel={release}\n\
         {epoch}\
         pkgdesc=\"{desc}\"\n\
         arch=('x86_64' 'aarch64')\n\
         url=\"{homepage}\"\n\
         license=('{license}')\n\
         {relations}\
         source=(\"{source_name}\")\n\
         sha256sums=('{sha256}')\n\
         \n\
         package() {{\n\
         \tcp -a usr \"$pkgdir/\"\n\
         }}\n",
        name = pkg.name,
        desc = pkg.description.replace('"', "\\\""),
    )
}

/// Render translated pacman entries as a space-separated list of
/// single-quoted elements (the inside of a PKGBUILD bash array). Returns an
/// empty string when there are no entries, so the caller can skip the line.
fn quote_array(items: Vec<String>) -> String {
    items
        .into_iter()
        .map(|e| format!("'{}'", e.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}
