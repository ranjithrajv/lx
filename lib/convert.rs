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

#[derive(Debug, Clone, Args)]
pub struct ConvertArgs {
    /// Path to the source package file (.deb, .rpm, .pkg.tar.zst).
    pub input: PathBuf,

    /// Target format: deb, rpm, or arch. Defaults to this host's native
    /// format (auto-detected), so `lx convert foo.rpm` on a deb host → deb.
    #[arg(short = 't', long, value_name = "FORMAT")]
    pub to: Option<String>,

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
    let target = match &args.to {
        Some(to) => to.to_ascii_lowercase(),
        None => match crate::info::native_plugin_format()
            .filter(|format| matches!(*format, "deb" | "rpm" | "arch"))
        {
            Some(format) => {
                println!("--to not given; targeting this host's native format '{format}'");
                format.to_string()
            }
            None => bail!(
                "no --to given and this host's native format is unknown; \
                 pass --to deb, --to rpm, or --to arch"
            ),
        },
    };
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

    // Scan ELF files in the install tree and fill in missing deps.
    let mut meta = scan_and_fill_deps(meta, &install_tree)?;

    // Interpreter-aware relocation: detect ecosystem, move payloads to the
    // target format's canonical lib root, normalize shebangs, ensure the
    // runtime dep, then translate dep names across formats.
    let ecosystem = detect_ecosystem(&install_tree, &meta.package);
    if !ecosystem.is_empty() {
        relocate_install_tree(&install_tree, &ecosystem, &target)?;
        let _ = rewrite_shebangs(&install_tree);
    }
    ensure_interpreter_dep(&mut meta, &ecosystem, &target);

    // Map dependency syntax from source format to target format.
    let mut meta = convert_deps_syntax(meta, source_format, &target);
    meta = convert_dep_names(meta, &ecosystem, &target);

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
    let pkg = rpm::Package::open(input)
        .with_context(|| format!("failed to open '{}'", input.display()))?;
    let name = pkg.metadata.get_name().unwrap_or_default().to_string();
    let version = pkg.metadata.get_version().unwrap_or_default().to_string();
    let release = pkg.metadata.get_release().unwrap_or_default().to_string();
    let version = if release.is_empty() {
        version
    } else {
        format!("{version}-{release}")
    };
    let arch = pkg.metadata.get_arch().unwrap_or_default().to_string();
    let maintainer = pkg.metadata.get_vendor().unwrap_or_default().to_string();
    let description = pkg.metadata.get_summary().unwrap_or_default().to_string();

    let depends = pkg
        .metadata
        .get_requires()
        .map(|deps| {
            deps.iter()
                .map(|d| d.name.as_str())
                .filter(|n| {
                    !n.is_empty()
                        && !n.starts_with('/')
                        && !n.starts_with("rpmlib(")
                        && !n.starts_with("config(")
                        && !n.starts_with("interpreter(")
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let scripts = extract_rpm_scripts(&pkg);

    Ok(SourceMeta {
        package: name,
        version,
        arch,
        maintainer,
        description,
        depends,
        distribution: "el9".to_string(),
        scripts,
    })
}

/// Extract scriptlets from an RPM in-process via the `rpm` crate.
fn extract_rpm_scripts(pkg: &rpm::Package) -> BTreeMap<String, String> {
    let mut scripts = BTreeMap::new();
    let pairs: [(&str, Option<rpm::Scriptlet>); 6] = [
        ("pre", pkg.metadata.get_pre_install_script().ok()),
        ("post", pkg.metadata.get_post_install_script().ok()),
        ("preun", pkg.metadata.get_pre_uninstall_script().ok()),
        ("postun", pkg.metadata.get_post_uninstall_script().ok()),
        ("pretrans", pkg.metadata.get_pre_trans_script().ok()),
        ("posttrans", pkg.metadata.get_post_trans_script().ok()),
    ];
    for (key, s) in pairs {
        if let Some(s) = s {
            let body = s.script.trim().to_string();
            if !body.is_empty() && body != "(none)" {
                scripts.insert(key.to_string(), body);
            }
        }
    }
    scripts
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
    // Collect all `depend` lines (Arch PKGBUILD dependencies).
    let mut depends = Vec::new();
    for line in pkginfo.lines() {
        if let Some((k, v)) = line.split_once(" = ") {
            let k = k.trim();
            let v = v.trim();
            if k == "depend" && !v.is_empty() {
                depends.push(v.to_string());
            } else {
                fields.insert(k, v);
            }
        }
    }
    let get = |k: &str| fields.get(k).copied().unwrap_or("").to_string();
    Ok(SourceMeta {
        package: get("pkgname"),
        version: get("pkgver"),
        arch: get("arch"),
        maintainer: get("packager"),
        description: get("pkgdesc"),
        depends: depends.join(", "),
        distribution: "arch".to_string(),
        scripts: BTreeMap::new(),
    })
}

/// Detect the interpreter ecosystem from install-tree layout + package name.
/// Returns "npm" | "gem" | "cpan" | "python" | "" (unknown/compiled).
fn detect_ecosystem(install_tree: &Path, package: &str) -> String {
    // Name-prefix hints (conventional names per lib/pkgname.rs).
    let p = package.to_ascii_lowercase();
    let mut hint = String::new();
    if p.starts_with("node-") || p.starts_with("nodejs-") {
        hint = "npm".into();
    } else if p.starts_with("ruby-") {
        hint = "gem".into();
    } else if p.ends_with("-perl") || p.starts_with("perl-") || p.starts_with("lib") {
        // lib*-perl (deb) / perl-* (rpm/arch) — only firm if tree confirms.
        hint = "cpan".into();
    } else if p.starts_with("python3-") || p.starts_with("python-") {
        hint = "python".into();
    }
    // Tree-layout confirmation wins over the hint.
    if has_marker(install_tree, "package.json")
        || install_tree.join("usr/share/nodejs").exists()
        || install_tree.join("usr/lib/nodejs").exists()
    {
        return "npm".to_string();
    }
    if has_marker(install_tree, ".gemspec")
        || install_tree.join("usr/lib/ruby").exists()
        || install_tree.join("usr/share/rubygems").exists()
    {
        return "gem".to_string();
    }
    if has_marker(install_tree, ".pm")
        || install_tree.join("usr/share/perl5").exists()
        || install_tree.join("usr/lib/perl5").exists()
    {
        return "cpan".to_string();
    }
    if has_marker(install_tree, "METADATA")
        || has_marker(install_tree, "PKG-INFO")
        || install_tree.join("usr/lib/python3").exists()
        || install_tree.join("usr/lib/python3.12").exists()
    {
        return "python".to_string();
    }
    hint
}

/// True if any file under `root` (depth ≤ 4) ends with `suffix`.
fn has_marker(root: &Path, suffix: &str) -> bool {
    fn walk(dir: &Path, depth: u8, suffix: &str) -> bool {
        if depth > 4 {
            return false;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_file() {
                if path.to_string_lossy().ends_with(suffix) {
                    return true;
                }
            } else if path.is_dir() && walk(&path, depth + 1, suffix) {
                return true;
            }
        }
        false
    }
    walk(root, 0, suffix)
}

/// Canonical library root per ecosystem + target format.
fn lib_root(ecosystem: &str, target: &str) -> Option<String> {
    match (ecosystem, target) {
        ("npm", "deb") => Some("usr/share/nodejs".into()),
        ("npm", _) => Some("usr/lib/nodejs".into()),
        ("gem", _) => Some("usr/lib/ruby/gems".into()),
        ("cpan", "deb") => Some("usr/share/perl5".into()),
        ("cpan", _) => Some("usr/lib/perl5".into()),
        ("python", "deb") => Some("usr/lib/python3/dist-packages".into()),
        ("python", "rpm") => Some("usr/lib/python3/site-packages".into()),
        ("python", _) => Some("usr/lib/python/site-packages".into()),
        _ => None,
    }
}

/// Relocate interpreter payloads to the target format's canonical path.
/// Source trees from another format/ecosystem often carry the wrong prefix
/// (e.g. rpm `usr/lib/python3/site-packages` → deb expects
/// `usr/lib/python3/dist-packages`). Moves the first matching source dir
/// verbatim; compiled binaries elsewhere in the tree are untouched.
fn relocate_install_tree(install_tree: &Path, ecosystem: &str, target: &str) -> Result<()> {
    if ecosystem.is_empty() {
        return Ok(());
    }
    let dest_rel = match lib_root(ecosystem, target) {
        Some(r) => r,
        None => return Ok(()),
    };
    // Candidate source dirs holding the interpreter payload.
    let candidates = match ecosystem {
        "npm" => vec!["usr/share/nodejs", "usr/lib/nodejs", "usr/lib/node_modules"],
        "gem" => vec!["usr/lib/ruby/gems", "usr/share/rubygems", "var/lib/gems"],
        "cpan" => vec![
            "usr/share/perl5",
            "usr/lib/perl5",
            "usr/share/perl",
            "usr/lib/x86_64-linux-gnu/perl5",
        ],
        "python" => vec![
            "usr/lib/python3/dist-packages",
            "usr/lib/python3/site-packages",
            "usr/lib/python/site-packages",
            "usr/lib/python3.12/site-packages",
            "usr/lib/python3.11/site-packages",
        ],
        _ => return Ok(()),
    };
    for cand in candidates {
        if cand == dest_rel {
            return Ok(()); // already canonical
        }
        let src = install_tree.join(cand);
        if src.is_dir() {
            let dest = install_tree.join(&dest_rel);
            fs::create_dir_all(&dest)?;
            for entry in fs::read_dir(&src)? {
                let entry = entry?;
                let to = dest.join(entry.file_name());
                if to.exists() {
                    continue; // never clobber; keep first copy
                }
                fs::rename(entry.path(), &to)
                    .with_context(|| format!("relocating {} → {}", cand, dest_rel))?;
            }
            // Remove now-empty source dir (ignore failure).
            let _ = fs::remove_dir(&src);
            println!("  ↔ relocated {cand} → {dest_rel} ({ecosystem})");
            return Ok(());
        }
    }
    Ok(())
}

/// Normalize `#!` lines to `/usr/bin/env <interp>` so scripts survive
/// prefix moves across formats (deb↔rpm↔arch ship interpreters at the
/// same `/usr/bin` names but payloads may embed versioned paths).
fn rewrite_shebangs(install_tree: &Path) -> Result<usize> {
    fn walk(dir: &Path, count: &mut usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(&path, count);
            } else if path.is_file() && rewrite_one(&path) {
                *count += 1;
            }
        }
    }
    fn rewrite_one(path: &Path) -> bool {
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        if !bytes.starts_with(b"#!") {
            return false;
        }
        let end = bytes
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(bytes.len());
        let line = String::from_utf8_lossy(&bytes[..end]).to_string();
        // Map versioned/absolute interpreter paths to /usr/bin/env form.
        let interp = if line.contains("python") {
            Some("python3")
        } else if line.contains("perl") {
            Some("perl")
        } else if line.contains("ruby") {
            Some("ruby")
        } else if line.contains("node") {
            Some("node")
        } else {
            None
        };
        let Some(interp) = interp else {
            return false;
        };
        let want = format!("#!/usr/bin/env {interp}");
        if line.trim() == want {
            return false;
        }
        let mut out = want.into_bytes();
        out.extend_from_slice(&bytes[end..]);
        if fs::write(path, out).is_ok() {
            return true;
        }
        false
    }
    let mut count = 0;
    walk(install_tree, &mut count);
    if count > 0 {
        println!("  ↔ normalized {count} shebang(s) to /usr/bin/env");
    }
    Ok(count)
}

/// Ensure the interpreter runtime dep for `ecosystem` is present in
/// target-format syntax, translating names (node→nodejs, python→python).
fn ensure_interpreter_dep(meta: &mut SourceMeta, ecosystem: &str, target: &str) {
    if ecosystem.is_empty() {
        return;
    }
    let runtime = match (ecosystem, target) {
        ("npm", "deb") => "nodejs",
        ("npm", _) => "nodejs",
        ("gem", _) => "ruby",
        ("cpan", "deb") => "perl",
        ("cpan", _) => "perl",
        ("python", "deb") => "python3",
        ("python", "rpm") => "python3",
        ("python", _) => "python",
        _ => return,
    };
    let lower = meta.depends.to_ascii_lowercase();
    // Heuristic: runtime already declared if the token appears.
    if lower
        .split([',', '|', '(', ')', ' '])
        .any(|t| t.trim() == runtime)
    {
        return;
    }
    if meta.depends.trim().is_empty() {
        meta.depends = runtime.to_string();
    } else {
        meta.depends = format!("{}, {}", meta.depends.trim(), runtime);
    }
    println!("  ℹ added runtime dep '{runtime}' for {ecosystem}");
}

/// Scan ELF files in the install tree and fill in missing `depends:`.
/// If the source package left deps empty (common for rpm/arch conversions),
/// populate from non-essential sonames resolved via `pkg_owner`.
/// If deps exist, verify completeness and warn on gaps.
fn scan_and_fill_deps(mut meta: SourceMeta, install_tree: &Path) -> Result<SourceMeta> {
    let elf_files = match crate::scandeps::find_elf_files(install_tree) {
        Ok(f) => f,
        Err(_) => return Ok(meta),
    };
    if elf_files.is_empty() {
        return Ok(meta);
    }

    let mut scanned_sonames = std::collections::BTreeSet::new();
    for elf_path in &elf_files {
        let bytes = match fs::read(elf_path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Ok(libs) = lx_lib::elfdeps::needed_libraries(&bytes) {
            for lib in &libs {
                if !lx_lib::elfdeps::is_essential_libc_soname(lib) {
                    scanned_sonames.insert(lib.clone());
                }
            }
        }
    }

    if scanned_sonames.is_empty() {
        return Ok(meta);
    }

    // Resolve non-essential sonames to package names via the host's package manager.
    let mut resolved_pkgs = std::collections::BTreeSet::new();
    for soname in &scanned_sonames {
        if let Some(pkg) = crate::scandeps::pkg_owner(soname) {
            // Strip :arch qualifier and version constraints.
            let pkg = pkg.split(':').next().unwrap_or(&pkg).to_string();
            resolved_pkgs.insert(pkg);
        }
    }

    if meta.depends.trim().is_empty() && !resolved_pkgs.is_empty() {
        // Source had no deps — fill from ELF scanning.
        meta.depends = resolved_pkgs.iter().cloned().collect::<Vec<_>>().join(", ");
        println!("  ℹ filled depends from ELF scanning: {}", meta.depends);
    } else if !meta.depends.is_empty() && !resolved_pkgs.is_empty() {
        // Source had deps — verify completeness and warn on gaps.
        let declared = crate::scandeps::declared_package_names(&meta.depends);
        let missing: Vec<&String> = resolved_pkgs
            .iter()
            .filter(|p| !declared.contains(&p.to_ascii_lowercase()))
            .collect();
        if !missing.is_empty() {
            println!(
                "  ⚠ ELF needs packages not in source depends: {}",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    Ok(meta)
}

/// Translate dependency *names* across formats for interpreter ecosystems.
///
/// `convert_deps_syntax` only rewrites operators; names like `nodejs`
/// (rpm/arch) vs `nodejs`/`node-*` (deb) and `python3-*` vs `python-*`
/// need mapping via `pkgname::conventional_name` heuristics + the
/// `depmap` deb→rpm/arch tables. Unknown names pass through untouched.
fn convert_dep_names(mut meta: SourceMeta, ecosystem: &str, target: &str) -> SourceMeta {
    if meta.depends.trim().is_empty() {
        return meta;
    }
    let eco = if ecosystem.is_empty() {
        infer_ecosystem_from_dep(&meta.depends)
    } else {
        ecosystem.to_string()
    };
    let deps: Vec<String> = meta
        .depends
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|dep| map_dep_name(dep, &eco, target))
        .collect();
    meta.depends = deps.join(", ");
    meta
}

/// Guess ecosystem from dep tokens when tree detection found nothing.
fn infer_ecosystem_from_dep(depends: &str) -> String {
    let l = depends.to_ascii_lowercase();
    if l.contains("nodejs") || l.contains("node-") {
        "npm".into()
    } else if l.contains("ruby") || l.contains("rubygem") {
        "gem".into()
    } else if l.contains("perl") {
        "cpan".into()
    } else if l.contains("python") {
        "python".into()
    } else {
        String::new()
    }
}

fn map_dep_name(dep: &str, ecosystem: &str, target: &str) -> String {
    // Split `name (constraint)` / `name op ver` / bare name.
    let (name, constraint) = split_dep(dep);
    if name.is_empty() {
        return dep.to_string();
    }
    let mapped = translate_name(&name, ecosystem, target);
    match constraint {
        Some(c) => match target {
            "deb" => format!("{mapped} ({c})"),
            "rpm" => format!("{mapped} {c}"),
            "arch" => format!("{mapped}{c}"),
            _ => dep.to_string(),
        },
        None => mapped,
    }
}

fn split_dep(dep: &str) -> (String, Option<String>) {
    let dep = dep.split('|').next().unwrap_or(dep).trim();
    if let Some(idx) = dep.find('(') {
        let name = dep[..idx].trim().to_string();
        let c = dep[idx..]
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim()
            .to_string();
        if c.is_empty() {
            (name, None)
        } else {
            (name, Some(c))
        }
    } else {
        // rpm `name >= ver` / arch `name>=ver` / bare
        for op in &[
            " >= ", " <= ", " > ", " < ", " = ", ">=", "<=", ">", "<", "=",
        ] {
            if let Some(idx) = dep.find(op) {
                let name = dep[..idx].trim().to_string();
                let ver = dep[idx..].trim().to_string();
                return (name, Some(ver));
            }
        }
        (dep.to_string(), None)
    }
}

/// Translate one bare package name into `target` conventions.
fn translate_name(name: &str, ecosystem: &str, target: &str) -> String {
    let l = name.to_ascii_lowercase();
    // Interpreter runtimes.
    if l == "nodejs" || l == "node" || l == "node-js" {
        return "nodejs".to_string();
    }
    if l == "ruby" || l == "ruby-interpreter" {
        return "ruby".to_string();
    }
    if l == "perl" || l == "perl-interpreter" {
        return "perl".to_string();
    }
    if l == "python3" && target == "arch" {
        return "python".to_string();
    }
    if l == "python" && target == "deb" {
        return "python3".to_string();
    }
    // Ecosystem-prefixed names → target conventions via pkgname helpers.
    // Strip the source prefix, re-apply the target prefix.
    if !ecosystem.is_empty() {
        let stripped = strip_eco_prefix(&l, ecosystem);
        if stripped != l {
            return crate::pkgname::conventional_name(ecosystem, stripped, target, None);
        }
        // Already-conventional names from another format (e.g. deb
        // `libfoo-perl` → rpm `perl-Foo`): normalize through conventional_name.
        if matches!(ecosystem, "cpan" | "gem" | "npm" | "python") {
            let base = l
                .trim_start_matches("lib")
                .trim_matches('-')
                .trim_end_matches("-perl")
                .trim_start_matches("perl-")
                .trim_start_matches("ruby-")
                .trim_start_matches("node-")
                .trim_start_matches("nodejs-")
                .trim_start_matches("python3-")
                .trim_start_matches("python-");
            if base != l {
                return crate::pkgname::conventional_name(ecosystem, base, target, None);
            }
        }
    }
    // System-library names via depmap tables (deb→rpm/arch).
    if target == "rpm" || target == "arch" {
        if let Some(mapped) = depmap_lookup(name, target) {
            return mapped;
        }
    }
    name.to_string()
}

fn strip_eco_prefix<'a>(name: &'a str, ecosystem: &str) -> &'a str {
    let prefixes: &[&str] = match ecosystem {
        "npm" => &["nodejs-", "node-"],
        "gem" => &["ruby-"],
        "cpan" => &["lib", "perl-", "lib"],
        "python" => &["python3-", "python-"],
        _ => &[],
    };
    for p in prefixes {
        if let Some(s) = name.strip_prefix(p) {
            if ecosystem == "cpan" {
                return s.trim_end_matches("-perl").trim_end_matches("perl");
            }
            return s;
        }
    }
    name
}

/// Look up deb→rpm/arch system-library translation via depmap's tables.
fn depmap_lookup(name: &str, target: &str) -> Option<String> {
    // depmap::map_dependency maps registry→system; here we need
    // system→system. Reuse its deb→rpm/arch tables indirectly: probe each
    // known deb name — small table, cheap.
    const KNOWN_DEB: &[&str] = &[
        "libssl3",
        "libsqlite3-0",
        "libpq5",
        "libmariadb3",
        "libcurl4",
        "libgd3",
        "libxml2",
        "libvips",
        "libcairo2",
        "zlib1g",
        "libffi8",
        "libgit2-1.7",
        "libicu74",
        "libnss3",
        "libsodium23",
        "libgrpc++1",
        "libonig5",
        "libzip4",
        "libopenblas0",
        "libargon2-1",
        "libmagickwand-6.q16-6",
        "libsass",
        "libyaml-0-2",
    ];
    // Reverse direction: if `name` is already an rpm/arch name, keep it.
    // Only translate exact deb-name hits.
    for deb in KNOWN_DEB {
        if name.eq_ignore_ascii_case(deb) {
            // Reuse depmap by mapping a sentinel registry dep is awkward;
            // duplicate the small table via map_dependency on known probes.
            return Some(system_name_for(deb, target));
        }
    }
    None
}

fn system_name_for(deb: &str, target: &str) -> String {
    match (deb, target) {
        ("libssl3", "rpm") => "openssl-libs".into(),
        ("libssl3", _) => "openssl".into(),
        ("libsqlite3-0", _) => "sqlite".into(),
        ("libpq5", "rpm") => "postgresql-libs".into(),
        ("libpq5", _) => "postgresql-libs".into(),
        ("libmariadb3", "rpm") => "mariadb-connector-c".into(),
        ("libmariadb3", _) => "mariadb-libs".into(),
        ("libcurl4", "rpm") => "libcurl".into(),
        ("libcurl4", _) => "curl".into(),
        ("zlib1g", _) => "zlib".into(),
        ("libffi8", _) => "libffi".into(),
        ("libnss3", _) => "nss".into(),
        ("libsodium23", _) => "libsodium".into(),
        ("libicu74", "rpm") => "libicu".into(),
        ("libicu74", _) => "icu".into(),
        ("libzip4", _) => "libzip".into(),
        ("libyaml-0-2", _) => "libyaml".into(),
        ("libxml2", _) => "libxml2".into(),
        ("libcairo2", _) => "cairo".into(),
        ("libvips", _) => "vips".into(),
        _ => deb.to_string(),
    }
}

/// Convert dependency syntax from source format to target format.
///
/// Arch PKGBUILD uses `name>=version` (no spaces), RPM uses
/// `name >= version` (spaces, >=/>/</<=), and Debian uses
/// `name (>= version)` (parenthesized). This function normalizes
/// every source format into the target format's expected syntax.
fn convert_deps_syntax(mut meta: SourceMeta, source: &str, target: &str) -> SourceMeta {
    if meta.depends.is_empty() || source == target {
        return meta;
    }
    let deps: Vec<String> = meta
        .depends
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|dep| map_dep_syntax(dep, source, target))
        .collect();
    meta.depends = deps.join(", ");
    meta
}

/// Map a single dependency clause from one format's syntax to another.
fn map_dep_syntax(dep: &str, source: &str, target: &str) -> String {
    // Parse into (name, constraint) regardless of source format.
    let (name, constraint) = match source {
        "arch" => parse_arch_dep(dep),
        "rpm" => parse_rpm_dep(dep),
        "deb" => parse_deb_dep(dep),
        _ => (dep.to_string(), None),
    };
    if name.is_empty() {
        return dep.to_string();
    }
    match target {
        "deb" => match constraint {
            Some(c) => format!("{name} ({c})"),
            None => name,
        },
        "rpm" => match constraint {
            Some(c) => format!("{name} {c}"),
            None => name,
        },
        "arch" => match constraint {
            Some(c) => format!("{name}{c}"),
            None => name,
        },
        _ => dep.to_string(),
    }
}

/// Parse an Arch-style dep: `name>=1.0`, `name>1.0`, `name=1.0`, or bare `name`.
fn parse_arch_dep(dep: &str) -> (String, Option<String>) {
    // Split on the first comparison operator.
    for op in &[">=", "<=", ">", "<", "="] {
        if let Some(idx) = dep.find(op) {
            let name = dep[..idx].trim().to_string();
            let ver = dep[idx..].trim().to_string();
            return (name, Some(ver));
        }
    }
    (dep.trim().to_string(), None)
}

/// Parse an RPM-style dep: `name >= 1.0`, `name >= 1.0`, bare `name`.
fn parse_rpm_dep(dep: &str) -> (String, Option<String>) {
    for op in &[" >= ", " <= ", " > ", " < ", " = ", "!=", "="] {
        if let Some(idx) = dep.find(op) {
            let name = dep[..idx].trim().to_string();
            let ver = dep[idx..].trim().to_string();
            return (name, Some(ver));
        }
    }
    (dep.trim().to_string(), None)
}

/// Parse a Debian-style dep: `name (>= 1.0)`, `name`, `name | other`.
fn parse_deb_dep(dep: &str) -> (String, Option<String>) {
    // Take only the first alternative (before '|').
    let dep = dep.split('|').next().unwrap_or(dep).trim();
    if let Some(idx) = dep.find('(') {
        let name = dep[..idx].trim().to_string();
        let constraint = dep[idx..]
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim()
            .to_string();
        if constraint.is_empty() {
            (name, None)
        } else {
            (name, Some(constraint))
        }
    } else {
        (dep.trim().to_string(), None)
    }
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
            crate::rpmarchive::extract(input, dest)?;
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
            checksums: Default::default(),
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
        detected_deps: Vec::new(),
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
        _ => {
            // arch: carry scripts into .INSTALL pre/post hooks instead of dropping.
            if let Some(s) = scripts.get("pre").or_else(|| scripts.get("preinst")) {
                config.scripts.preupgrade_script = s.clone();
            }
            if let Some(s) = scripts.get("post").or_else(|| scripts.get("postinst")) {
                config.scripts.postupgrade_script = s.clone();
            }
            if let Some(s) = scripts.get("preun").or_else(|| scripts.get("prerm")) {
                if config.scripts.preupgrade_script.is_empty() {
                    config.scripts.preupgrade_script = s.clone();
                }
            }
            if let Some(s) = scripts.get("postun").or_else(|| scripts.get("postrm")) {
                if config.scripts.postupgrade_script.is_empty() {
                    config.scripts.postupgrade_script = s.clone();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arch_dep_bare_name() {
        let (name, ver) = parse_arch_dep("glibc");
        assert_eq!(name, "glibc");
        assert!(ver.is_none());
    }

    #[test]
    fn parse_arch_dep_with_version() {
        let (name, ver) = parse_arch_dep("glibc>=2.17");
        assert_eq!(name, "glibc");
        assert_eq!(ver.as_deref(), Some(">=2.17"));
    }

    #[test]
    fn parse_rpm_dep_bare_name() {
        let (name, ver) = parse_rpm_dep("glibc");
        assert_eq!(name, "glibc");
        assert!(ver.is_none());
    }

    #[test]
    fn parse_rpm_dep_with_version() {
        let (name, ver) = parse_rpm_dep("glibc >= 2.17");
        assert_eq!(name, "glibc");
        assert_eq!(ver.as_deref(), Some(">= 2.17"));
    }

    #[test]
    fn parse_deb_dep_bare_name() {
        let (name, ver) = parse_deb_dep("libc6");
        assert_eq!(name, "libc6");
        assert!(ver.is_none());
    }

    #[test]
    fn parse_deb_dep_with_version() {
        let (name, ver) = parse_deb_dep("libc6 (>= 2.31)");
        assert_eq!(name, "libc6");
        assert_eq!(ver.as_deref(), Some(">= 2.31"));
    }

    #[test]
    fn parse_deb_dep_strips_alternatives() {
        let (name, ver) = parse_deb_dep("libssl3 (>= 3.0) | libssl1.1");
        assert_eq!(name, "libssl3");
        assert_eq!(ver.as_deref(), Some(">= 3.0"));
    }

    #[test]
    fn map_dep_arch_to_deb() {
        assert_eq!(
            map_dep_syntax("glibc>=2.17", "arch", "deb"),
            "glibc (>=2.17)"
        );
    }

    #[test]
    fn map_dep_rpm_to_deb() {
        assert_eq!(
            map_dep_syntax("glibc >= 2.17", "rpm", "deb"),
            "glibc (>= 2.17)"
        );
    }

    #[test]
    fn map_dep_deb_to_rpm() {
        assert_eq!(
            map_dep_syntax("libc6 (>= 2.31)", "deb", "rpm"),
            "libc6 >= 2.31"
        );
    }

    #[test]
    fn map_dep_arch_to_rpm() {
        assert_eq!(map_dep_syntax("glibc>=2.17", "arch", "rpm"), "glibc >=2.17");
    }

    #[test]
    fn map_dep_bare_passthrough() {
        assert_eq!(map_dep_syntax("glibc", "arch", "deb"), "glibc");
    }
}
