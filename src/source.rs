use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

const SRC_IMAGE: &str = "lpt-src";

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
///   * runs `dpkg-source`/`dpkg-deb` inside a Debian container (the host may
///     be non-Debian, e.g. Arch),
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

    check_docker()?;
    ensure_src_image()?;

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

/// Build one source package for a single distribution. Returns Ok(true)
/// when this call created the shared .orig.tar.xz.
#[allow(clippy::too_many_arguments)]
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
    let orig_name = format!("{}_{debian_version}.orig.tar.xz", pkg.name);

    // Reference .deb: prefer amd64 for this dist, else the first match.
    let ref_deb = find_ref_deb(out_dir, &pkg.name, dist)?
        .ok_or_else(|| anyhow::anyhow!("no .deb for dist {dist}"))?;

    std::fs::create_dir_all(tree_dir.join("debian/source"))
        .context("failed to create debian/source")?;

    // Extract the binary package as the source tree (in-container: the host
    // may not have dpkg-deb). DEBIAN/ and usr/share/doc are Debian packaging
    // output, not upstream payload, so strip them here in the same container
    // (the container owns the extracted files, so only it can remove them).
    run_container_cmd(
        out_dir,
        workdir,
        &[
            "bash",
            "-c",
            &format!(
                "set -e; dpkg-deb -x /out/{ref_deb} /work/{tree} && rm -rf /work/{tree}/DEBIAN /work/{tree}/usr/share/doc"
            ),
        ],
    )
    .with_context(|| format!("dpkg-deb -x failed for {ref_deb}"))?;

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
    // upstream layout), so the tree dir name is rewritten in the archive.
    //
    // Reproducible-builds hygiene: plain `tar` reads directory entries in
    // filesystem order, which isn't guaranteed stable across separate
    // extractions into fresh temp directories -- two builds of the exact
    // same input could otherwise produce a differently-ordered (and thus
    // differently-compressed) tarball. --sort=name fixes member order;
    // --mtime/--owner/--group/--numeric-owner strip the extraction
    // timestamp and container UID/GID, which would otherwise vary by
    // build host and build time. --mtime respects SOURCE_DATE_EPOCH (the
    // reproducible-builds.org standard) when set, else a fixed epoch.
    let mut created_orig = false;
    if !orig_done {
        let upstream_dir = format!("{}-{debian_version}", pkg.name);
        let source_date_epoch =
            std::env::var("SOURCE_DATE_EPOCH").unwrap_or_else(|_| "0".to_string());
        run_container_cmd(
            out_dir,
            workdir,
            &[
                "tar",
                "--sort=name",
                &format!("--mtime=@{source_date_epoch}"),
                "--owner=0",
                "--group=0",
                "--numeric-owner",
                "-cJf",
                "/work/orig.tar.xz",
                "--transform",
                &format!("s|^{tree}|{upstream_dir}|"),
                "-C",
                "/work",
                &format!("{tree}/usr"),
            ],
        )
        .context("failed to create orig tarball")?;
        std::fs::copy(workdir.join("orig.tar.xz"), workdir.join(&orig_name))?;
        created_orig = true;
    }

    // dpkg-source -b writes <pkg>_<src_version>.dsc + .debian.tar.xz into
    // the cwd. Run it in the container with the workdir mounted; collect
    // the outputs from the workdir.
    run_container_cmd(
        out_dir,
        workdir,
        &["bash", "-c", &format!("cd /work && dpkg-source -b {tree}")],
    )
    .with_context(|| format!("dpkg-source -b failed for {dist}"))?;

    std::fs::copy(
        workdir.join(format!("{}_{src_version}.dsc", pkg.name)),
        out_dir.join(format!("{}_{src_version}.dsc", pkg.name)),
    )?;
    std::fs::copy(
        workdir.join(format!("{}_{src_version}.debian.tar.xz", pkg.name)),
        out_dir.join(format!("{}_{src_version}.debian.tar.xz", pkg.name)),
    )?;
    if created_orig {
        std::fs::copy(workdir.join(&orig_name), out_dir.join(&orig_name))?;
    }
    println!("  ✓ source package: {}_{src_version}.dsc", pkg.name);
    Ok(created_orig)
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

fn check_docker() -> Result<()> {
    let status = Command::new("docker")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run docker")?;
    if !status.success() {
        bail!("docker is required to generate source packages (the host is non-Debian)");
    }
    Ok(())
}

/// Build a cached image with dpkg-dev installed, if not already present.
fn ensure_src_image() -> Result<()> {
    let inspect = Command::new("docker")
        .args(["image", "inspect", SRC_IMAGE])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to inspect source image")?;
    if inspect.success() {
        return Ok(());
    }

    let dir = tempfile::tempdir().context("failed to create temp dir")?;
    let df = dir.path().join("Dockerfile");
    std::fs::write(
        &df,
        "FROM debian:bookworm\nRUN apt-get update && apt-get install -y --no-install-recommends dpkg-dev xz-utils && rm -rf /var/lib/apt/lists/*\n",
    )
    .context("failed to write source Dockerfile")?;

    let status = Command::new("docker")
        .args([
            "build",
            "-t",
            SRC_IMAGE,
            "-f",
            df.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::null())
        .status()
        .context("failed to build source image")?;
    if !status.success() {
        bail!("failed to build source image ({SRC_IMAGE})");
    }
    Ok(())
}

/// Run a command inside the source image, mounting the real `out_dir` and
/// `workdir` from the host so dpkg-deb/tar/dpkg-source operate on the same
/// files the host wrote.
fn run_container_cmd(out_dir: &Path, workdir: &Path, args: &[&str]) -> Result<()> {
    let out_abs = out_dir
        .canonicalize()
        .unwrap_or_else(|_| out_dir.to_path_buf());
    let work_abs = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--rm",
        "-v",
        &format!("{}:/out", out_abs.display()),
        "-v",
        &format!("{}:/work", work_abs.display()),
    ]);
    cmd.arg(SRC_IMAGE);
    cmd.args(args);
    let status = cmd.status().context("failed to run source container")?;
    if !status.success() {
        bail!(
            "container command failed: docker run {SRC_IMAGE} {}",
            args.join(" ")
        );
    }
    Ok(())
}
