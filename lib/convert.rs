// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx convert` — convert a built package from one format to another.
//!
//! Reads a `.deb`, `.rpm`, or `.pkg.tar.zst` produced by `lx build` (or any
//! source) and re-packages it as a different format. This is the fpm-style
//! format conversion (`-s rpm -t deb`) that lx otherwise lacks — instead of
//! converting archives byte-for-byte (which loses metadata), it reads the
//! control fields from the source, extracts the install tree, and rebuilds
//! natively in the target format via the plugin pipeline.

use anyhow::{bail, Context, Result};
use clap::Args;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Args)]
pub struct ConvertArgs {
    /// Path to the source package file (.deb, .rpm, .pkg.tar.zst).
    pub input: PathBuf,

    /// Target format: deb, rpm, or arch.
    #[arg(short = 't', long, value_name = "FORMAT")]
    pub to: String,

    /// Output directory for the converted package.
    #[arg(short = 'o', long, default_value = "dist")]
    pub output: PathBuf,

    /// Override the package name (default: read from source).
    #[arg(long, value_name = "NAME")]
    pub package_name: Option<String>,

    /// Override the package version (default: read from source).
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Override the architecture (default: read from source).
    #[arg(long, value_name = "ARCH")]
    pub arch: Option<String>,

    /// Override the distribution/suite (default: read from source).
    #[arg(long, value_name = "DIST")]
    pub distribution: Option<String>,

    /// Build revision / iteration (default: "1").
    #[arg(long, default_value = "1")]
    pub build_version: String,

    /// Print what would be done without converting.
    #[arg(long)]
    pub dry_run: bool,
}

/// Metadata extracted from a source package — enough to rebuild in any format.
#[derive(Debug, Default)]
struct SourceMeta {
    package: String,
    version: String,
    arch: String,
    maintainer: String,
    description: String,
    depends: String,
    distribution: String,
    /// Maintainer scripts extracted from the source package, keyed by script
    /// name (e.g. "preinst", "postinst", "prerm", "postrm" for deb;
    /// "pre", "post", "preun", "postun" for rpm).
    scripts: std::collections::BTreeMap<String, String>,
}

pub fn run(args: ConvertArgs) -> Result<()> {
    if !args.input.exists() {
        bail!("input '{}' does not exist", args.input.display());
    }
    let target = args.to.to_ascii_lowercase();
    if !["deb", "rpm", "arch"].contains(&target.as_str()) {
        bail!(
            "unsupported target format '{}' (expected deb, rpm, or arch)",
            target
        );
    }

    // Detect source format from extension.
    let input_ext = args
        .input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let source_format = if input_ext == "deb" {
        "deb"
    } else if input_ext == "rpm" {
        "rpm"
    } else if input_ext == "zst" || args.input.to_string_lossy().contains(".pkg.tar") {
        "arch"
    } else {
        bail!(
            "cannot detect source format from '{}': expected .deb, .rpm, or .pkg.tar.zst",
            args.input.display()
        )
    };

    if source_format == target {
        bail!(
            "source and target format are both '{}' — nothing to convert",
            target
        );
    }

    println!(
        "converting {} ({}) → {}",
        args.input.display(),
        source_format,
        target
    );

    // Extract metadata + install tree from the source.
    let tmp = tempfile::tempdir().context("failed to create temp dir")?;
    let meta = extract_meta(source_format, &args.input, tmp.path())?;
    let install_tree = tmp.path().join("install_tree");
    extract_install_tree(source_format, &args.input, &install_tree)?;

    // Apply overrides. Bind resolved values first so we don't partially
    // move `args` (which is borrowed later by `build_target`).
    let package_name = args.package_name.clone().unwrap_or(meta.package);
    let version = args.version.clone().unwrap_or(meta.version);
    let arch = args.arch.clone().unwrap_or(meta.arch);
    let distribution = args.distribution.clone().unwrap_or(meta.distribution);
    let meta = SourceMeta {
        package: package_name,
        version,
        arch,
        distribution,
        ..meta
    };

    if meta.package.is_empty() {
        bail!("no package name found (use --package-name to override)");
    }
    if meta.version.is_empty() {
        bail!("no version found (use --version to override)");
    }
    if meta.arch.is_empty() {
        bail!("no architecture found (use --arch to override)");
    }

    if args.dry_run {
        println!(
            "would convert: {} {}-{} ({}) → {}",
            meta.package, meta.version, args.build_version, meta.arch, target
        );
        println!("  install tree: {}", install_tree.display());
        return Ok(());
    }

    fs::create_dir_all(&args.output)?;

    // Build the target package from the extracted install tree.
    let built = build_target(&meta, &install_tree, &target, &args)?;

    let final_name = format!(
        "{}_{}-{}+{}_{}.{}",
        meta.package,
        meta.version,
        args.build_version,
        meta.distribution,
        meta.arch,
        ext_for(&target)
    );
    let final_path = args.output.join(&final_name);
    fs::copy(&built, &final_path)
        .with_context(|| format!("copying {} to {}", built.display(), final_path.display()))?;

    println!("✓ converted: {}", final_path.display());
    Ok(())
}

fn ext_for(format: &str) -> &str {
    match format {
        "deb" => "deb",
        "rpm" => "rpm",
        "arch" => "pkg.tar.zst",
        _ => "bin",
    }
}

/// Read control metadata from the source package.
fn extract_meta(format: &str, input: &Path, tmp: &Path) -> Result<SourceMeta> {
    match format {
        "deb" => extract_deb_meta(input, tmp),
        "rpm" => extract_rpm_meta(input),
        "arch" => extract_arch_meta(input, tmp),
        _ => bail!("unknown source format '{format}'"),
    }
}

fn extract_deb_meta(input: &Path, tmp: &Path) -> Result<SourceMeta> {
    let ctrl = crate::repo::read_control(input)?;
    let get = |k: &str| ctrl.get(k).cloned().unwrap_or_default();
    let scripts = extract_deb_scripts(input, tmp)?;
    Ok(SourceMeta {
        package: get("Package"),
        version: get("Version"),
        arch: get("Architecture"),
        maintainer: get("Maintainer"),
        description: get("Description"),
        depends: get("Depends"),
        distribution: infer_dist_from_version(&get("Version")),
        scripts,
    })
}

/// Extract maintainer scripts (preinst/postinst/prerm/postrm) from a .deb.
fn extract_deb_scripts(input: &Path, _tmp: &Path) -> Result<BTreeMap<String, String>> {
    let mut scripts = BTreeMap::new();
    let file = std::fs::File::open(input)?;
    let mut archive = ar::Archive::new(file);
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry?;
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if !name.starts_with("control.tar") {
            continue;
        }
        let mut data = Vec::new();
        std::io::copy(&mut entry, &mut data)?;
        let text: Vec<u8> = if name.ends_with(".gz") {
            let mut gz = flate2::read::GzDecoder::new(data.as_slice());
            let mut out = Vec::new();
            std::io::copy(&mut gz, &mut out)?;
            out
        } else if name.ends_with(".xz") {
            let mut out = Vec::new();
            let mut dec = lzma_rust2::XzReader::new(data.as_slice(), true);
            std::io::copy(&mut dec, &mut out)?;
            out
        } else {
            data
        };
        let mut ar = tar::Archive::new(text.as_slice());
        for member in ar.entries()? {
            let mut member = member?;
            let fname = member
                .path()?
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            match fname.as_str() {
                "preinst" | "postinst" | "prerm" | "postrm" | "config" | "templates" => {
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut member, &mut s)?;
                    scripts.insert(fname, s);
                }
                _ => {}
            }
        }
        break;
    }
    Ok(scripts)
}

fn extract_rpm_meta(input: &Path) -> Result<SourceMeta> {
    let name = Command::new("rpm")
        .args(["-qp", "--queryformat", "%{NAME}", &input.to_string_lossy()])
        .output()
        .context("rpm not found — install rpm to convert from .rpm")?;
    let version = Command::new("rpm")
        .args([
            "-qp",
            "--queryformat",
            "%{VERSION}-%{RELEASE}",
            &input.to_string_lossy(),
        ])
        .output()
        .context("rpm query failed")?;
    let arch = Command::new("rpm")
        .args(["-qp", "--queryformat", "%{ARCH}", &input.to_string_lossy()])
        .output()
        .context("rpm query failed")?;
    let maintainer = Command::new("rpm")
        .args([
            "-qp",
            "--queryformat",
            "%{VENDOR}",
            &input.to_string_lossy(),
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let description = Command::new("rpm")
        .args([
            "-qp",
            "--queryformat",
            "%{SUMMARY}",
            &input.to_string_lossy(),
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    if !name.status.success() {
        bail!("rpm query failed for '{}'", input.display());
    }

    let scripts = extract_rpm_scripts(input)?;

    Ok(SourceMeta {
        package: String::from_utf8_lossy(&name.stdout).trim().to_string(),
        version: String::from_utf8_lossy(&version.stdout).trim().to_string(),
        arch: String::from_utf8_lossy(&arch.stdout).trim().to_string(),
        maintainer: maintainer.trim().to_string(),
        description: description.trim().to_string(),
        depends: String::new(),
        distribution: "el9".to_string(),
        scripts,
    })
}

/// Extract scriptlets from an RPM via `rpm -qp --queryformat`.
fn extract_rpm_scripts(input: &Path) -> Result<BTreeMap<String, String>> {
    let mut scripts = BTreeMap::new();
    for (tag, key) in [
        ("%{PREIN}", "pre"),
        ("%{POSTIN}", "post"),
        ("%{PREUN}", "preun"),
        ("%{POSTUN}", "postun"),
        ("%{PRETRANS}", "pretrans"),
        ("%{POSTTRANS}", "posttrans"),
        ("%{VERIFYSCRIPT}", "verify"),
    ] {
        let out = Command::new("rpm")
            .args(["-qp", "--queryformat", tag, &input.to_string_lossy()])
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                let body = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !body.is_empty() && body != "(none)" {
                    scripts.insert(key.to_string(), body);
                }
            }
        }
    }
    Ok(scripts)
}

fn extract_arch_meta(input: &Path, _tmp: &Path) -> Result<SourceMeta> {
    // Extract .PKGINFO from the zstd-compressed tar.
    let data = fs::read(input)?;
    let decompressed = zstd::bulk::decompress(&data, 64 * 1024 * 1024)
        .context("zstd decompress failed for .pkg.tar.zst")?;
    let mut archive = tar::Archive::new(decompressed.as_slice());
    let mut pkginfo = String::new();
    for entry in archive.entries().context("reading arch archive")? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path.file_name().and_then(|n| n.to_str()) == Some(".PKGINFO") {
            std::io::Read::read_to_string(&mut entry, &mut pkginfo)?;
            break;
        }
    }
    let mut fields = BTreeMap::new();
    for line in pkginfo.lines() {
        if let Some((k, v)) = line.split_once(" = ") {
            fields.insert(k.trim(), v.trim());
        }
    }
    let get = |k: &str| fields.get(k).copied().unwrap_or("").to_string();
    Ok(SourceMeta {
        package: get("pkgname"),
        version: get("pkgver"),
        arch: get("arch"),
        maintainer: get("packager"),
        description: get("pkgdesc"),
        depends: String::new(),
        distribution: "arch".to_string(),
        scripts: BTreeMap::new(),
    })
}

/// Extract the install tree (files that go into the package) to a directory.
fn extract_install_tree(format: &str, input: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    match format {
        "deb" => {
            // Extract data.tar.* member.
            let file = fs::File::open(input)?;
            let mut archive = ar::Archive::new(file);
            while let Some(entry) = archive.next_entry() {
                let mut entry = entry?;
                let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
                if !name.starts_with("data.tar") {
                    continue;
                }
                let mut data = Vec::new();
                std::io::copy(&mut entry, &mut data)?;
                let text: Vec<u8> = if name.ends_with(".gz") {
                    let mut gz = flate2::read::GzDecoder::new(data.as_slice());
                    let mut out = Vec::new();
                    std::io::copy(&mut gz, &mut out)?;
                    out
                } else if name.ends_with(".xz") {
                    let mut out = Vec::new();
                    let mut dec = lzma_rust2::XzReader::new(data.as_slice(), true);
                    std::io::copy(&mut dec, &mut out)?;
                    out
                } else if name.ends_with(".zst") || name.ends_with(".zstd") {
                    zstd::bulk::decompress(&data, 64 * 1024 * 1024)
                        .context("zstd decompress data.tar")?
                } else {
                    data
                };
                let mut ar = tar::Archive::new(text.as_slice());
                ar.unpack(dest).with_context(|| {
                    format!("failed to extract data.tar from '{}'", input.display())
                })?;
                break;
            }
        }
        "rpm" => {
            // rpm2cpio + cpio extracts the payload.
            let cpio = Command::new("rpm2cpio")
                .arg(input)
                .output()
                .context("rpm2cpio not found — install rpm2cpio to convert from .rpm")?;
            if !cpio.status.success() {
                bail!("rpm2cpio failed for '{}'", input.display());
            }
            let mut cpio_proc = std::process::Command::new("cpio")
                .args(["-idm", "--no-absolute-filenames"])
                .current_dir(dest)
                .stdin(std::process::Stdio::piped())
                .spawn()
                .context("cpio not found — install cpio to convert from .rpm")?;
            {
                use std::io::Write;
                cpio_proc
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(&cpio.stdout)
                    .context("writing to cpio stdin")?;
            }
            let status = cpio_proc.wait()?;
            if !status.success() {
                bail!("cpio extract failed for '{}'", input.display());
            }
        }
        "arch" => {
            let data = fs::read(input)?;
            let decompressed = zstd::bulk::decompress(&data, 64 * 1024 * 1024)
                .context("zstd decompress failed")?;
            let mut archive = tar::Archive::new(decompressed.as_slice());
            archive.unpack(dest).context("extracting .pkg.tar.zst")?;
        }
        _ => bail!("unknown source format '{format}'"),
    }
    Ok(())
}

/// Build the target package from the extracted install tree.
fn build_target(
    meta: &SourceMeta,
    install_tree: &Path,
    target_format: &str,
    args: &ConvertArgs,
) -> Result<PathBuf> {
    let plugin = crate::plugins::get_packager(target_format)
        .ok_or_else(|| anyhow::anyhow!("unknown target plugin '{target_format}'"))?;

    let tmp = tempfile::tempdir()?;
    let staging_root = tmp.path().join("staging");
    fs::create_dir_all(&staging_root)?;

    // Stage the install tree: copy everything under install_tree into staging.
    // For deb/rpm, files under usr/bin etc. are already laid out correctly.
    copy_dir_recursive(install_tree, &staging_root)?;

    // Build a minimal config for the plugin, carrying over maintainer scripts
    // from the source package (mapped to the target format's expected names).
    let mut config = crate::config::PackageConfig {
        package_name: meta.package.clone(),
        version: meta.version.clone(),
        description: meta.description.clone(),
        maintainer: meta.maintainer.clone(),
        depends: meta.depends.clone(),
        package_format: target_format.to_string(),
        ..Default::default()
    };
    apply_scripts_to_config(&mut config, &meta.scripts, target_format);

    let job = crate::build::ResolvedJob {
        dist: if meta.distribution.is_empty() {
            match target_format {
                "deb" => "bookworm".to_string(),
                "rpm" => "el9".to_string(),
                _ => "arch".to_string(),
            }
        } else {
            meta.distribution.clone()
        },
        arch: meta.arch.clone(),
        asset: lx_lib::github::Asset {
            name: args.input.to_string_lossy().to_string(),
            size: None,
            browser_download_url: String::new(),
        },
        tag: meta.version.clone(),
        published_at: None,
    };

    let ctx = crate::plugins::BuildContext {
        cfg: &config,
        job: &job,
        binary_dir: install_tree,
        staging_root: &staging_root,
        license: None,
        debian_version: &meta.version,
        build_version: &args.build_version,
        mtime: jiff::Timestamp::now().as_second(),
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
    };

    plugin.build(&ctx)
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let target = dest.join(name);
        if path.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .with_context(|| format!("copying {} to {}", path.display(), target.display()))?;
        }
    }
    Ok(())
}

/// Infer distribution from a Debian version string (e.g. "1.0+bookworm" → "bookworm").
fn infer_dist_from_version(version: &str) -> String {
    version
        .split_once('+')
        .map(|(_, after)| after.to_string())
        .unwrap_or_else(|| "bookworm".to_string())
}

/// Apply extracted source-package scripts to the target config, mapping
/// source-format script names to the target format's expected names.
///
/// Source deb: preinst, postinst, prerm, postrm
/// Source rpm: pre, post, preun, postun, pretrans, posttrans, verify
/// Target deb: preinstall, postinstall, preremove, postremove
/// Target rpm: preinstall (=pre), postinstall (=post), preremove (=preun),
///             postremove (=postun), pretrans, posttrans, verify
fn apply_scripts_to_config(
    config: &mut crate::config::PackageConfig,
    scripts: &BTreeMap<String, String>,
    target_format: &str,
) {
    if scripts.is_empty() {
        return;
    }

    match target_format {
        "deb" => {
            // Map source script names to deb config field names.
            // deb source → deb target: preinst→preinstall, etc.
            // rpm source → deb target: pre→preinstall, preun→preremove, etc.
            if let Some(s) = scripts.get("preinst").or_else(|| scripts.get("pre")) {
                config.scripts.preinstall = s.clone();
            }
            if let Some(s) = scripts.get("postinst").or_else(|| scripts.get("post")) {
                config.scripts.postinstall = s.clone();
            }
            if let Some(s) = scripts.get("prerm").or_else(|| scripts.get("preun")) {
                config.scripts.preremove = s.clone();
            }
            if let Some(s) = scripts.get("postrm").or_else(|| scripts.get("postun")) {
                config.scripts.postremove = s.clone();
            }
            if let Some(s) = scripts.get("pretrans") {
                config.scripts.pretrans = s.clone();
            }
            if let Some(s) = scripts.get("posttrans") {
                config.scripts.posttrans = s.clone();
            }
            if let Some(s) = scripts.get("verify") {
                config.scripts.verify = s.clone();
            }
        }
        "rpm" => {
            // Map source script names to rpm config field names.
            // rpm source → rpm target: pre→preinstall, etc.
            // deb source → rpm target: preinst→preinstall, etc.
            if let Some(s) = scripts.get("pre").or_else(|| scripts.get("preinst")) {
                config.scripts.preinstall = s.clone();
            }
            if let Some(s) = scripts.get("post").or_else(|| scripts.get("postinst")) {
                config.scripts.postinstall = s.clone();
            }
            if let Some(s) = scripts.get("preun").or_else(|| scripts.get("prerm")) {
                config.scripts.preremove = s.clone();
            }
            if let Some(s) = scripts.get("postun").or_else(|| scripts.get("postrm")) {
                config.scripts.postremove = s.clone();
            }
            if let Some(s) = scripts.get("pretrans") {
                config.scripts.pretrans = s.clone();
            }
            if let Some(s) = scripts.get("posttrans") {
                config.scripts.posttrans = s.clone();
            }
            if let Some(s) = scripts.get("verify") {
                config.scripts.verify = s.clone();
            }
        }
        _ => {} // arch: no script carry-over (arch uses .INSTALL, set separately)
    }
}
