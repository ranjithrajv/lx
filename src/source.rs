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
    pub license_spdx: String,
}

/// Generate Debian source packages (3.0 quilt) for each distribution among
/// the built .debs in `out_dir`, mirroring the action's
/// `build_source_packages`:
///   * architecture-independent, so generated once per dist,
///   * built entirely in-process (`lpt_lib::debarchive`) -- no Docker, no
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

    let debian_version = debian_version_of(&pkg.version);
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

/// `0.23.5` -> `0.23.5`; `v1.2.3` -> `1.2.3` (strips leading non-digits),
/// matching the action's `sed -E 's/^[^0-9]*//'`.
fn debian_version_of(version: &str) -> String {
    version
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .collect()
}

/// One `Checksums-*`/`Files` entry: the `.dsc` lists both tarballs, orig
/// always before debian (dpkg-source's own convention, verified against a
/// real `dpkg-source -b` run -- not alphabetical, which would order them
/// the other way).
struct DscFile {
    name: String,
    size: u64,
    md5: String,
    sha1: String,
    sha256: String,
}

impl DscFile {
    fn from_bytes(name: String, data: &[u8]) -> Self {
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
/// when this call created the shared `.orig.tar.gz`.
fn build_source_package(
    out_dir: &Path,
    workdir: &Path,
    pkg: &Pkg,
    debian_version: &str,
    dist: &str,
    orig_done: bool,
) -> Result<bool> {
    let src_version = format!("{}-{}+{dist}", debian_version, pkg.build_version);
    let tree = format!("{}-{src_version}", pkg.name);
    let tree_dir = workdir.join(&tree);
    let orig_name = format!("{}_{debian_version}.orig.tar.gz", pkg.name);
    let debian_tar_name = format!("{}_{src_version}.debian.tar.gz", pkg.name);
    let dsc_name = format!("{}_{src_version}.dsc", pkg.name);

    // Reference .deb: prefer amd64 for this dist, else the first match.
    let ref_deb = find_ref_deb(out_dir, &pkg.name, dist)?
        .ok_or_else(|| anyhow::anyhow!("no .deb for dist {dist}"))?;

    std::fs::create_dir_all(tree_dir.join("debian/source"))
        .context("failed to create debian/source")?;

    // Extract the binary package as the source tree. DEBIAN/ and
    // usr/share/doc are Debian packaging output, not upstream payload, so
    // strip them.
    lpt_lib::debarchive::extract(&out_dir.join(&ref_deb), &tree_dir)
        .with_context(|| format!("failed to extract {ref_deb}"))?;
    let _ = std::fs::remove_dir_all(tree_dir.join("DEBIAN"));
    let _ = std::fs::remove_dir_all(tree_dir.join("usr/share/doc"));

    // debian/control (mirrors templates/source/control).
    let control = format!(
        "Source: {name}\nSection: utils\nPriority: optional\nMaintainer: {m}\nHomepage: https://github.com/{repo}\nStandards-Version: 4.6.2\nBuild-Depends: debhelper-compat (= 13)\n\nPackage: {name}\nArchitecture: any\nDescription: {desc}\n Packaged from the upstream GitHub release for Debian.\n",
        name = pkg.name,
        m = pkg.maintainer,
        repo = pkg.github_repo,
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
            std::fs::Permissions::from_mode(0o755),
        )?;
    }

    // debian/changelog (mirrors templates/source/changelog).
    let changelog = format!(
        "{name} ({src_version}) {dist}; urgency=medium\n\n  * New upstream release {version}\n\n -- {m}  Mon, 01 Jan 2024 00:00:00 +0000\n",
        name = pkg.name,
        version = debian_version,
        m = pkg.maintainer,
    );
    std::fs::write(tree_dir.join("debian/changelog"), changelog)?;

    // debian/copyright (mirrors templates/output/copyright).
    let year = 2026;
    let copyright = format!(
        "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\nUpstream-Name: {name}\nSource: https://github.com/{repo}\nLicense: {spdx}\n\nFiles: *\nCopyright: {year} {repo}\nLicense: {spdx}\n",
        name = pkg.name,
        repo = pkg.github_repo,
        spdx = pkg.license_spdx,
    );
    std::fs::write(tree_dir.join("debian/copyright"), copyright)?;

    // debian/source/format
    std::fs::write(tree_dir.join("debian/source/format"), "3.0 (quilt)\n")?;

    // The upstream orig tarball is built once and shared across dists. It
    // must contain a top-level <pkg>-<debian_version>/ directory (standard
    // upstream layout).
    //
    // Reproducible-builds hygiene: `lpt_lib::debarchive::tar_gz_tree` walks
    // in sorted order and normalizes mtime/owner/group -- see its own doc
    // comment. `mtime` respects SOURCE_DATE_EPOCH (the reproducible-builds
    // .org standard) when set, else a fixed epoch.
    let source_date_epoch: i64 = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let orig_path = workdir.join(&orig_name);
    let created_orig = if !orig_done {
        let upstream_dir = format!("{}-{debian_version}", pkg.name);
        let orig_bytes = lpt_lib::debarchive::tar_gz_tree(
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

    // debian.tar.gz: just the debian/ metadata directory -- lpt never
    // produces quilt patches, so there's no .pc/ or patches/ to include.
    let debian_bytes =
        lpt_lib::debarchive::tar_gz_tree(&tree_dir.join("debian"), "debian/", source_date_epoch)
            .context("failed to build debian.tar.gz")?;
    std::fs::write(workdir.join(&debian_tar_name), &debian_bytes)?;

    let orig_file = DscFile::from_bytes(orig_name.clone(), &orig_bytes);
    let debian_file = DscFile::from_bytes(debian_tar_name.clone(), &debian_bytes);
    let dsc = render_dsc(pkg, &src_version, &orig_file, &debian_file);
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
/// Unsigned, matching the previous Docker-based behavior (which never
/// signed either).
fn render_dsc(pkg: &Pkg, src_version: &str, orig: &DscFile, debian: &DscFile) -> String {
    format!(
        "Format: 3.0 (quilt)\n\
         Source: {name}\n\
         Binary: {name}\n\
         Architecture: any\n\
         Version: {src_version}\n\
         Maintainer: {maintainer}\n\
         Homepage: https://github.com/{repo}\n\
         Standards-Version: 4.6.2\n\
         Build-Depends: debhelper-compat (= 13)\n\
         Package-List:\n\
         \x20{name} deb utils optional arch=any\n\
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
        repo = pkg.github_repo,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debian_version_strips_leading_non_digits() {
        assert_eq!(debian_version_of("v1.2.3"), "1.2.3");
        assert_eq!(debian_version_of("0.23.5"), "0.23.5");
        assert_eq!(debian_version_of("bun-v1.3.14"), "1.3.14");
    }

    #[test]
    fn dsc_lists_orig_before_debian_and_matches_dpkg_source_field_order() {
        let pkg = Pkg {
            name: "eza".into(),
            github_repo: "eza-community/eza".into(),
            description: "eza, packaged from eza-community/eza".into(),
            maintainer: "latest-debs maintainers <maintainers@latest-debs.org>".into(),
            version: "0.23.5".into(),
            build_version: "1".into(),
            license_spdx: "MIT".into(),
        };
        let orig = DscFile::from_bytes("eza_0.23.5.orig.tar.gz".into(), b"orig-bytes");
        let debian = DscFile::from_bytes(
            "eza_0.23.5-1+bookworm.debian.tar.gz".into(),
            b"debian-bytes",
        );
        let dsc = render_dsc(&pkg, "0.23.5-1+bookworm", &orig, &debian);

        let lines: Vec<&str> = dsc.lines().collect();
        assert_eq!(lines[0], "Format: 3.0 (quilt)");
        assert_eq!(lines[1], "Source: eza");
        assert_eq!(lines[2], "Binary: eza");
        assert_eq!(lines[3], "Architecture: any");
        assert_eq!(lines[4], "Version: 0.23.5-1+bookworm");
        assert!(dsc.contains("Package-List:\n eza deb utils optional arch=any\n"));

        let checksums_idx = dsc.find("Checksums-Sha1:").unwrap();
        let orig_idx = dsc[checksums_idx..].find("orig.tar.gz").unwrap();
        let debian_idx = dsc[checksums_idx..].find("debian.tar.gz").unwrap();
        assert!(orig_idx < debian_idx, "orig must be listed before debian");
    }
}
