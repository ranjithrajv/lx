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
    recommends: String,
    suggests: String,
    conflicts: String,
    replaces: String,
    provides: String,
    breaks: String,
    predepends: String,
    /// Debian `Section` / RPM `Group` (best effort), carried so a converted
    /// package lands in the right section instead of defaulting.
    section: String,
    /// Debian `Priority` (empty means the target default).
    priority: String,
    /// Package epoch, re-emitted in the target's own field (`<epoch>:` in a
    /// deb Version, the RPM/Arch epoch tag).
    epoch: String,
    distribution: String,
    /// Maintainer scripts extracted from the source package, keyed by script
    /// name (e.g. "preinst", "postinst", "prerm", "postrm" for deb;
    /// "pre", "post", "preun", "postun" for rpm).
    scripts: std::collections::BTreeMap<String, String>,
    /// Config files (`/etc/...`) carrying config semantics, as
    /// `(absolute path, noreplace)`. Re-registered in the target as a deb
    /// conffile, an RPM `%config`/`%config(noreplace)`, or a pacman `backup`.
    conffiles: Vec<(String, bool)>,
    /// Maintainer triggers found in the source (deb `triggers` lines, RPM
    /// `%trigger*` conditions/scriptlets).
    triggers: Vec<Trigger>,
}

/// A maintainer trigger found in a source package.
#[derive(Debug, Clone)]
struct Trigger {
    /// deb: `interest` / `interest-await` / `interest-noawait` / `activate` /
    /// `activate-await` / `activate-noawait`. rpm: `triggerin` / `triggerun` /
    /// `triggerpostun` / `triggerprein`.
    kind: String,
    /// deb: the trigger name/path; rpm: the condition package.
    name: String,
    /// rpm trigger scriptlet body (deb triggers carry no script).
    script: String,
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

    // Detect source format from the file name. Requiring the full
    // `.pkg.tar.zst`/`.zstd` suffix avoids misclassifying e.g. a
    // `.pkg.tar.xz` as a zstd arch package.
    let input_name = args
        .input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let input_ext = args
        .input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let source_format = if input_ext == "deb" {
        "deb"
    } else if input_ext == "rpm" {
        "rpm"
    } else if input_name.ends_with(".pkg.tar.zst") || input_name.ends_with(".pkg.tar.zstd") {
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
    let arch = args
        .arch
        .clone()
        .unwrap_or_else(|| normalize_arch(&meta.arch, source_format, &target));
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

    // Build the target package from the extracted install tree. The work
    // dir must outlive the copy below: the plugin writes the built artifact
    // under it, so a `TempDir` owned by `build_target` would be dropped (and
    // the artifact deleted) before we copy it out.
    let built = build_target(&meta, &install_tree, &target, &args, tmp.path())?;

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
    let mut scripts = extract_deb_scripts(input, tmp)?;
    let triggers = parse_deb_triggers(scripts.get("triggers").map(String::as_str).unwrap_or(""));
    // Standard debs carry conffiles in the `DEBIAN/conffiles` control member,
    // not the control field; prefer the member, fall back to the field.
    let member_conffiles = scripts.remove("conffiles").unwrap_or_default();
    let conffiles = if member_conffiles.trim().is_empty() {
        parse_deb_conffiles(&get("Conffiles"))
    } else {
        parse_deb_conffiles(&member_conffiles)
    };
    let (epoch, version) = split_deb_epoch(&get("Version"));
    Ok(SourceMeta {
        package: get("Package"),
        version: version.clone(),
        arch: get("Architecture"),
        maintainer: get("Maintainer"),
        description: get("Description"),
        depends: get("Depends"),
        recommends: get("Recommends"),
        suggests: get("Suggests"),
        conflicts: get("Conflicts"),
        replaces: get("Replaces"),
        provides: get("Provides"),
        breaks: get("Breaks"),
        predepends: get("Pre-Depends"),
        section: get("Section"),
        priority: get("Priority"),
        epoch,
        distribution: infer_dist_from_version(&version),
        scripts,
        conffiles,
        triggers,
    })
}

/// Split a Debian `Version` into (`epoch`, `version`). `1:2.3-1` ->
/// (`1`, `2.3-1`); a version with no numeric epoch is returned unchanged.
fn split_deb_epoch(raw: &str) -> (String, String) {
    match raw.split_once(':') {
        Some((epoch, rest)) if !epoch.is_empty() && epoch.chars().all(|c| c.is_ascii_digit()) => {
            (epoch.to_string(), rest.to_string())
        }
        _ => (String::new(), raw.to_string()),
    }
}

/// Normalize a source package's architecture name into `target`'s
/// convention, canonicalizing through the Debian name (RPM `x86_64` -> deb
/// `amd64` -> pacman `x86_64`). An architecture already in the target's own
/// convention passes through unchanged.
fn normalize_arch(arch: &str, source: &str, target: &str) -> String {
    let deb = if source == "deb" {
        arch.to_string()
    } else {
        from_rpm_arch(arch).to_string()
    };
    match target {
        "deb" => deb,
        "rpm" => lx_lib::constants::to_rpm_arch(&deb).to_string(),
        "arch" => lx_lib::constants::to_pacman_arch(&deb).to_string(),
        _ => arch.to_string(),
    }
}

/// Inverse of [`lx_lib::constants::to_rpm_arch`] for the architectures lx
/// emits (`x86_64` -> `amd64`, `aarch64` -> `arm64`, ...).
fn from_rpm_arch(arch: &str) -> &str {
    match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "armhfp" | "armv7hl" | "armv7h" => "armhf",
        "i686" => "i386",
        "ppc64le" => "ppc64el",
        "loongarch64" => "loong64",
        other => other,
    }
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
                "preinst" | "postinst" | "prerm" | "postrm" | "config" | "templates"
                | "triggers" | "conffiles" => {
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

    let depends = format_rpm_requires(&pkg.metadata.get_requires().unwrap_or_default());
    let provides = format_rpm_provides(&name, &pkg.metadata.get_provides().unwrap_or_default());
    let recommends = format_rpm_requires(&pkg.metadata.get_recommends().unwrap_or_default());
    let conflicts = format_rpm_requires(&pkg.metadata.get_conflicts().unwrap_or_default());
    let replaces = format_rpm_requires(&pkg.metadata.get_obsoletes().unwrap_or_default());
    let section = pkg.metadata.get_group().unwrap_or_default().to_string();
    let epoch = pkg
        .metadata
        .get_epoch()
        .ok()
        .filter(|e| *e > 0)
        .map(|e| e.to_string())
        .unwrap_or_default();

    let scripts = extract_rpm_scripts(&pkg);

    Ok(SourceMeta {
        package: name,
        version,
        arch,
        maintainer,
        description,
        depends,
        provides,
        recommends,
        conflicts,
        replaces,
        section,
        epoch,
        distribution: "el9".to_string(),
        scripts,
        conffiles: rpm_conffiles(&pkg),
        triggers: rpm_triggers(&pkg),
        ..Default::default()
    })
}

/// Format one RPM dependency as `name` or `name op version`.
fn format_rpm_dependency(d: &rpm::Dependency) -> String {
    if d.version.is_empty() {
        return d.name.clone();
    }
    let op = match d.flags {
        f if f.contains(rpm::DependencyFlags::GE) => ">=",
        f if f.contains(rpm::DependencyFlags::LE) => "<=",
        f if f.contains(rpm::DependencyFlags::GREATER) => ">",
        f if f.contains(rpm::DependencyFlags::LESS) => "<",
        _ => "=",
    };
    format!("{} {} {}", d.name, op, d.version)
}

/// Format RPM `Requires`, skipping file/rpmlib/config/interpreter
/// capabilities that have no cross-format meaning, plus trigger-flagged
/// dependencies (lx records RPM triggers as trigger-flagged Requires).
fn format_rpm_requires(deps: &[rpm::Dependency]) -> String {
    deps.iter()
        .filter(|d| {
            let n = d.name.as_str();
            !n.is_empty()
                && !n.starts_with('/')
                && !n.starts_with("rpmlib(")
                && !n.starts_with("config(")
                && !n.starts_with("interpreter(")
                && rpm_trigger_kind(d.flags).is_none()
                // Auto-generated soname / capability requires
                // (`libc.so.6()(64bit)`, `rtld(GNU_HASH)`, ...) are not valid
                // target package names — mirror the Provides filter.
                && !n.contains('(')
                && !n.contains(')')
                && !n.contains(".so")
        })
        .map(format_rpm_dependency)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The trigger kind an RPM dependency flag set encodes, if any.
fn rpm_trigger_kind(flags: rpm::DependencyFlags) -> Option<&'static str> {
    if flags.contains(rpm::DependencyFlags::TRIGGERIN) {
        Some("triggerin")
    } else if flags.contains(rpm::DependencyFlags::TRIGGERUN) {
        Some("triggerun")
    } else if flags.contains(rpm::DependencyFlags::TRIGGERPOSTUN) {
        Some("triggerpostun")
    } else if flags.contains(rpm::DependencyFlags::TRIGGERPREIN) {
        Some("triggerprein")
    } else {
        None
    }
}

/// Read RPM triggers. lx's own builder records them as trigger-flagged
/// Requires; distro rpms use the dedicated trigger tags. Both are read.
fn rpm_triggers(pkg: &rpm::Package) -> Vec<Trigger> {
    let mut out: Vec<Trigger> = Vec::new();
    if let Ok(requires) = pkg.metadata.get_requires() {
        for d in &requires {
            if let Some(kind) = rpm_trigger_kind(d.flags) {
                push_trigger(&mut out, kind, &d.name, "");
            }
        }
    }
    let h = &pkg.metadata.header;
    let names = h
        .get_entry_data_as_string_array(rpm::IndexTag::RPMTAG_TRIGGERNAME)
        .unwrap_or(&[]);
    let flags = h
        .get_entry_data_as_u32_array(rpm::IndexTag::RPMTAG_TRIGGERFLAGS)
        .unwrap_or_default();
    let indexes = h
        .get_entry_data_as_u32_array(rpm::IndexTag::RPMTAG_TRIGGERINDEX)
        .unwrap_or_default();
    let scripts = h
        .get_entry_data_as_string_array(rpm::IndexTag::RPMTAG_TRIGGERSCRIPTS)
        .unwrap_or(&[]);
    for (i, name) in names.iter().enumerate() {
        let raw = flags.get(i).copied().unwrap_or(0);
        let flags = rpm::DependencyFlags::from_bits_retain(raw);
        let Some(kind) = rpm_trigger_kind(flags) else {
            continue;
        };
        let script = indexes
            .get(i)
            .and_then(|idx| scripts.get(*idx as usize))
            .map(String::as_str)
            .unwrap_or("");
        push_trigger(&mut out, kind, name, script);
    }
    out
}

fn push_trigger(out: &mut Vec<Trigger>, kind: &str, name: &str, script: &str) {
    if name.is_empty() || out.iter().any(|t| t.kind == kind && t.name == name) {
        return;
    }
    out.push(Trigger {
        kind: kind.to_string(),
        name: name.to_string(),
        script: script.to_string(),
    });
}

/// Parse deb `DEBIAN/triggers` lines (`interest <name>`, `activate-await <name>`).
fn parse_deb_triggers(text: &str) -> Vec<Trigger> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let kind = it.next()?;
            let name = it.next()?.to_string();
            Some(Trigger {
                kind: kind.to_string(),
                name,
                script: String::new(),
            })
        })
        .collect()
}

/// Map the source's triggers onto the target's trigger model where one
/// exists, reporting the ones it cannot represent instead of dropping them
/// silently. deb and RPM trigger models differ (deb triggers are declarative
/// and script-less; RPM triggers name a condition package and carry a
/// scriptlet), so the mapping is best-effort by design.
fn apply_triggers_to_config(
    config: &mut crate::config::PackageConfig,
    script_bodies: &mut BTreeMap<&'static str, String>,
    triggers: &[Trigger],
    target: &str,
) {
    for t in triggers {
        let kind = t.kind.as_str();
        match target {
            // RPM -> deb: a %triggerin condition becomes an `interest` line,
            // and its scriptlet runs from the postinst's `triggered` branch.
            "deb" if kind == "triggerin" => {
                if !config.deb.triggers_interest.contains(&t.name) {
                    config.deb.triggers_interest.push(t.name.clone());
                }
                if t.script.trim().is_empty() {
                    eprintln!(
                        "  ⚠ RPM trigger '{}' carried as a deb `interest` line only (no scriptlet body in the source)",
                        t.name
                    );
                } else {
                    let postinst = script_bodies.entry("postinstall").or_default();
                    append_triggered_handler(postinst, &t.name, &t.script);
                }
            }
            "deb" if kind.starts_with("trigger") => {
                eprintln!(
                    "  ⚠ RPM '{} -- {}' has no deb equivalent (deb has no install/remove trigger of that shape); not carried",
                    kind, t.name
                );
            }
            // deb -> RPM: a declarative `interest` becomes a %triggerin
            // condition (no script body exists in deb).
            "rpm" if kind.starts_with("interest") => {
                let entry = format!("{}:", t.name);
                if !config.rpm.trigger_post_install.contains(&entry) {
                    config.rpm.trigger_post_install.push(entry);
                }
                eprintln!(
                    "  ⚠ deb '{} {}' carried as an RPM %triggerin condition (deb triggers carry no script body; condition-only)",
                    kind, t.name
                );
            }
            "rpm" if kind.starts_with("activate") => {
                eprintln!(
                    "  ⚠ deb '{} {}' has no RPM equivalent (RPM cannot activate another package's trigger); not carried",
                    kind, t.name
                );
            }
            "arch" => {
                eprintln!(
                    "  ⚠ Arch has no trigger mechanism; '{} {}' not carried",
                    kind, t.name
                );
            }
            _ => {}
        }
    }
}

/// Append a `triggered` branch to a deb postinst that runs `script` when the
/// named trigger fires.
fn append_triggered_handler(postinst: &mut String, name: &str, script: &str) {
    if !postinst.is_empty() && !postinst.ends_with('\n') {
        postinst.push('\n');
    }
    postinst.push_str("if [ \"$1\" = \"triggered\" ]; then\n");
    postinst.push_str(&format!("  case \"$2\" in\n    *{name}*)\n"));
    for line in script.lines() {
        postinst.push_str("      ");
        postinst.push_str(line);
        postinst.push('\n');
    }
    postinst.push_str("      ;;\n  esac\nfi\n");
}

/// Format RPM `Provides`, keeping real virtual provides and dropping the
/// auto-generated capabilities a build emits: the self-provide (`name`,
/// `name(x86-64)`), shared-library sonames (`libc.so.6()(64bit)`), and
/// file/capability provides — none of which are valid target package names.
fn format_rpm_provides(package: &str, deps: &[rpm::Dependency]) -> String {
    deps.iter()
        .filter(|d| is_meaningful_provide(&d.name, package))
        .map(format_rpm_dependency)
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_meaningful_provide(name: &str, package: &str) -> bool {
    let n = name.trim();
    if n.is_empty() || n.starts_with('/') {
        return false;
    }
    if n == package || n.starts_with(&format!("{package}(")) {
        return false;
    }
    const SKIP: &[&str] = &[
        "rpmlib(",
        "config(",
        "interpreter(",
        "user(",
        "group(",
        "pkgconfig(",
        "lib(",
        "perl(",
        "python(",
        "python3(",
        "ruby(",
        "metainfo(",
        "mimehandler(",
        "application(",
        "typelib(",
    ];
    if SKIP.iter().any(|p| n.starts_with(p)) {
        return false;
    }
    // Shared-library sonames and 64-bit capability provides.
    if n.contains(".so") || n.contains("(64bit)") || n.contains("()") {
        return false;
    }
    true
}

/// Config files an RPM marks `%config` / `%config(noreplace)`, as
/// `(absolute path, noreplace)`.
fn rpm_conffiles(pkg: &rpm::Package) -> Vec<(String, bool)> {
    let Ok(entries) = pkg.metadata.get_file_entries() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|e| e.flags.contains(rpm::FileFlags::CONFIG))
        .map(|e| {
            let p = e.path.to_string_lossy().to_string();
            let path = if p.starts_with('/') {
                p
            } else {
                format!("/{p}")
            };
            (path, e.flags.contains(rpm::FileFlags::NOREPLACE))
        })
        .collect()
}

/// Parse a deb `Conffiles:` value (one `/path [md5]` per line).
fn parse_deb_conffiles(raw: &str) -> Vec<(String, bool)> {
    raw.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|p| p.starts_with('/'))
        .map(|p| (p.to_string(), false))
        .collect()
}

/// Arch `backup` paths (pacman preserves user edits, like
/// `%config(noreplace)`).
fn arch_conffiles(fields: &BTreeMap<String, Vec<String>>) -> Vec<(String, bool)> {
    fields
        .get("backup")
        .map(|v| {
            v.iter()
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let path = if p.starts_with('/') {
                        p.to_string()
                    } else {
                        format!("/{p}")
                    };
                    (path, true)
                })
                .collect()
        })
        .unwrap_or_default()
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
    // Extract `.PKGINFO` and `.INSTALL` from the zstd-compressed tar.
    let data = fs::read(input)?;
    let decompressed = zstd::bulk::decompress(&data, 64 * 1024 * 1024)
        .context("zstd decompress failed for .pkg.tar.zst")?;
    let mut archive = tar::Archive::new(decompressed.as_slice());
    let mut pkginfo = String::new();
    let mut install = String::new();
    for entry in archive.entries().context("reading arch archive")? {
        let mut entry = entry?;
        let name = entry
            .path()?
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        match name.as_str() {
            ".PKGINFO" => {
                std::io::Read::read_to_string(&mut entry, &mut pkginfo)?;
            }
            ".INSTALL" => {
                std::io::Read::read_to_string(&mut entry, &mut install)?;
            }
            _ => {}
        }
    }
    // `.PKGINFO` keys can repeat (`depend`, `provides`, `conflict`, ...), so
    // collect every value per key instead of last-wins.
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in pkginfo.lines() {
        if let Some((k, v)) = line.split_once(" = ") {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() {
                fields.entry(k.to_string()).or_default().push(v.to_string());
            }
        }
    }
    let get = |k: &str| {
        fields
            .get(k)
            .and_then(|v| v.last())
            .cloned()
            .unwrap_or_default()
    };
    let join = |k: &str| fields.get(k).map(|v| v.join(", ")).unwrap_or_default();
    // `optdepend` entries are `pkg: reason`; only the package name maps to a
    // target Recommends.
    let recommends = fields
        .get("optdepend")
        .map(|v| {
            v.iter()
                .map(|s| s.split(':').next().unwrap_or(s).trim().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    // pacman folds an epoch into `pkgver` (`2:1.0-3`); split it back out so
    // the target format's own epoch field/prefix is used (and the epoch stays
    // out of artifact filenames).
    let (epoch, version) = split_arch_epoch(&get("pkgver"));
    Ok(SourceMeta {
        package: get("pkgname"),
        version,
        arch: get("arch"),
        maintainer: get("packager"),
        description: get("pkgdesc"),
        depends: join("depend"),
        recommends,
        // pacman's PKGINFO key is the singular `conflict`.
        conflicts: join("conflict"),
        replaces: join("replaces"),
        provides: join("provides"),
        epoch,
        distribution: "arch".to_string(),
        scripts: parse_arch_install(&install),
        conffiles: arch_conffiles(&fields),
        ..Default::default()
    })
}

/// Split a pacman `pkgver` (`[epoch:]version-release`) into its epoch and the
/// epoch-free remainder. A non-numeric prefix (not an epoch) is left intact.
fn split_arch_epoch(pkgver: &str) -> (String, String) {
    match pkgver.split_once(':') {
        Some((epoch, rest)) if !epoch.is_empty() && epoch.chars().all(|c| c.is_ascii_digit()) => {
            (epoch.to_string(), rest.to_string())
        }
        _ => (String::new(), pkgver.to_string()),
    }
}

/// Parse pacman `.INSTALL` shell functions into [`SourceMeta::scripts`] keys,
/// using the canonical names `map_scripts_to_bodies` maps from.
fn parse_arch_install(text: &str) -> BTreeMap<String, String> {
    const HOOKS: [(&str, &str); 6] = [
        ("pre_install", "preinst"),
        ("post_install", "postinst"),
        ("pre_remove", "prerm"),
        ("post_remove", "postrm"),
        ("pre_upgrade", "preupgrade"),
        ("post_upgrade", "postupgrade"),
    ];
    let mut scripts = BTreeMap::new();
    for (func, key) in HOOKS {
        if let Some(body) = extract_shell_function(text, func) {
            if !body.trim().is_empty() {
                scripts.insert(key.to_string(), body);
            }
        }
    }
    scripts
}

/// Extract the body of a shell function `name() { ... }` by brace matching.
fn extract_shell_function(text: &str, name: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let needle = name.as_bytes();
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        let rel = bytes[i..].windows(needle.len()).position(|w| w == needle)?;
        let start = i + rel;
        // Must be a standalone token (not `foo_pre_install`).
        let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        if before_ok {
            let mut j = start + needle.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes[j..].starts_with(b"()") {
                j += 2;
                while j < bytes.len() && bytes[j] != b'{' {
                    j += 1;
                }
                if j < bytes.len() {
                    let open = j;
                    let mut depth = 0i32;
                    for k in open..bytes.len() {
                        match bytes[k] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    return Some(
                                        String::from_utf8_lossy(&bytes[open + 1..k]).to_string(),
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        i = start + needle.len();
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
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
    let eco = if ecosystem.is_empty() {
        infer_ecosystem_from_dep(&meta.depends)
    } else {
        ecosystem.to_string()
    };
    let map = |value: &str| -> String {
        value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|dep| map_dep_name(dep, &eco, target))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let depends = map(&meta.depends);
    let recommends = map(&meta.recommends);
    let suggests = map(&meta.suggests);
    let conflicts = map(&meta.conflicts);
    let replaces = map(&meta.replaces);
    let provides = map(&meta.provides);
    let breaks = map(&meta.breaks);
    let predepends = map(&meta.predepends);
    meta.depends = depends;
    meta.recommends = recommends;
    meta.suggests = suggests;
    meta.conflicts = conflicts;
    meta.replaces = replaces;
    meta.provides = provides;
    meta.breaks = breaks;
    meta.predepends = predepends;
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
    if source == target {
        return meta;
    }
    let map = |value: &str| -> String {
        value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|dep| map_dep_syntax(dep, source, target))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let depends = map(&meta.depends);
    let recommends = map(&meta.recommends);
    let suggests = map(&meta.suggests);
    let conflicts = map(&meta.conflicts);
    let replaces = map(&meta.replaces);
    let provides = map(&meta.provides);
    let breaks = map(&meta.breaks);
    let predepends = map(&meta.predepends);
    meta.depends = depends;
    meta.recommends = recommends;
    meta.suggests = suggests;
    meta.conflicts = conflicts;
    meta.replaces = replaces;
    meta.provides = provides;
    meta.breaks = breaks;
    meta.predepends = predepends;
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
        // pacman syntax has no spaces around the operator: `name>=1.0`.
        "arch" => match constraint {
            Some(c) => format!("{name}{}", c.replace(' ', "")),
            None => name,
        },
        _ => dep.to_string(),
    }
}

/// Parse an Arch-style dep: `name>=1.0`, `name>1.0`, `name=1.0`, or bare `name`.
fn parse_arch_dep(dep: &str) -> (String, Option<String>) {
    // Split on the first comparison operator (longest first) and normalize
    // the constraint to `op version`, matching `parse_deb_dep`/`parse_rpm_dep`
    // so every target formats it correctly.
    for op in &[">=", "<=", ">", "<", "="] {
        if let Some(idx) = dep.find(op) {
            let name = dep[..idx].trim().to_string();
            let ver = dep[idx + op.len()..].trim();
            let constraint = if ver.is_empty() {
                (*op).to_string()
            } else {
                format!("{op} {ver}")
            };
            return (name, Some(constraint));
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
            // Reuse the packager's extractor so the control members
            // (`.PKGINFO`/`.MTREE`/`.INSTALL`) are skipped, not shipped.
            crate::archarchive::extract(input, dest)?;
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
    work_dir: &Path,
) -> Result<PathBuf> {
    let plugin = crate::plugins::get_packager(target_format)
        .ok_or_else(|| anyhow::anyhow!("unknown target plugin '{target_format}'"))?;

    let staging_root = work_dir.join("staging");
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
        recommends: meta.recommends.clone(),
        suggests: meta.suggests.clone(),
        conflicts: meta.conflicts.clone(),
        replaces: meta.replaces.clone(),
        provides: meta.provides.clone(),
        breaks: meta.breaks.clone(),
        predepends: meta.predepends.clone(),
        section: meta.section.clone(),
        priority: meta.priority.clone(),
        epoch: meta.epoch.clone(),
        package_format: target_format.to_string(),
        ..Default::default()
    };
    // Map source scripts to target config fields as bodies first (triggers may
    // append to the postinstall body), then materialize each body to a real
    // file: the packagers treat `config.scripts.*` as paths in the build env.
    let mut script_bodies = map_scripts_to_bodies(&meta.scripts, target_format);
    apply_triggers_to_config(
        &mut config,
        &mut script_bodies,
        &meta.triggers,
        target_format,
    );
    stage_script_bodies(&mut config, &script_bodies, work_dir);

    // Carry config-file semantics: stage each source conffile as a target
    // `contents` entry of kind `config`, which every packager turns into its
    // own conffile / `%config(noreplace)` / `backup` registration.
    for (path, noreplace) in &meta.conffiles {
        let src = install_tree.join(path.trim_start_matches('/'));
        if !src.is_file() {
            continue;
        }
        config.contents.push(crate::config::ContentEntry {
            src: src.to_string_lossy().to_string(),
            dst: path.clone(),
            kind: if *noreplace {
                "config|noreplace".to_string()
            } else {
                "config".to_string()
            },
            ..Default::default()
        });
    }

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

/// Recursively copy a directory tree, recreating symlinks as symlinks.
///
/// `fs::copy` follows links: an absolute or dangling link fails (its target
/// isn't installed yet), and a relative link resolving inside the tree is
/// silently flattened into a full copy of its target. Neither is correct for
/// package payloads, so `symlink_metadata` is used and links are recreated.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let target = dest.join(name);
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            let link = fs::read_link(&path)
                .with_context(|| format!("reading symlink {}", path.display()))?;
            replace_symlink(&link, &target).with_context(|| {
                format!("symlinking {} -> {}", target.display(), link.display())
            })?;
        } else if meta.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .with_context(|| format!("copying {} to {}", path.display(), target.display()))?;
        }
    }
    Ok(())
}

/// Create (or replace) a symlink at `target` pointing to `link`.
#[cfg(unix)]
fn replace_symlink(link: &Path, target: &Path) -> Result<()> {
    match std::os::unix::fs::symlink(link, target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(target)
                .with_context(|| format!("replacing stale symlink {}", target.display()))?;
            std::os::unix::fs::symlink(link, target)
                .with_context(|| format!("symlinking {} -> {}", target.display(), link.display()))
        }
        Err(e) => {
            Err(e).with_context(|| format!("symlinking {} -> {}", target.display(), link.display()))
        }
    }
}

#[cfg(not(unix))]
fn replace_symlink(link: &Path, target: &Path) -> Result<()> {
    fs::copy(link, target)?;
    Ok(())
}

/// Infer distribution from a Debian version string (e.g. "1.0+bookworm" → "bookworm").
fn infer_dist_from_version(version: &str) -> String {
    version
        .split_once('+')
        .map(|(_, after)| after.to_string())
        .unwrap_or_else(|| "bookworm".to_string())
}

/// Map extracted source-package scripts to target config field names,
/// returning the script *bodies* keyed by target field name.
///
/// Source deb: preinst, postinst, prerm, postrm
/// Source rpm: pre, post, preun, postun, pretrans, posttrans, verify
/// Source arch: preinst/postinst/prerm/postrm/preupgrade/postupgrade
///              (parsed from `.INSTALL`)
///
/// Target names are the canonical `config.scripts.*` fields the packagers
/// consume.
fn map_scripts_to_bodies(
    scripts: &BTreeMap<String, String>,
    target_format: &str,
) -> BTreeMap<&'static str, String> {
    fn pick(scripts: &BTreeMap<String, String>, keys: &[&str]) -> Option<String> {
        keys.iter()
            .find_map(|k| scripts.get(*k))
            .filter(|s| !s.trim().is_empty())
            .cloned()
    }

    let mut bodies: BTreeMap<&'static str, String> = BTreeMap::new();
    match target_format {
        "deb" => {
            // deb source → deb target: preinst→preinstall, etc.
            // rpm source → deb target: pre→preinstall, preun→preremove, etc.
            if let Some(s) = pick(scripts, &["preinst", "pre"]) {
                bodies.insert("preinstall", s);
            }
            if let Some(s) = pick(scripts, &["postinst", "post"]) {
                bodies.insert("postinstall", s);
            }
            if let Some(s) = pick(scripts, &["prerm", "preun"]) {
                bodies.insert("preremove", s);
            }
            if let Some(s) = pick(scripts, &["postrm", "postun"]) {
                bodies.insert("postremove", s);
            }
            if let Some(s) = pick(scripts, &["pretrans"]) {
                bodies.insert("pretrans", s);
            }
            if let Some(s) = pick(scripts, &["posttrans"]) {
                bodies.insert("posttrans", s);
            }
            if let Some(s) = pick(scripts, &["verify"]) {
                bodies.insert("verify", s);
            }
            if let Some(s) = pick(scripts, &["preupgrade"]) {
                bodies.insert("preupgrade", s);
            }
            if let Some(s) = pick(scripts, &["postupgrade"]) {
                bodies.insert("postupgrade", s);
            }
        }
        "rpm" => {
            // rpm source → rpm target: pre→preinstall, etc.
            // deb source → rpm target: preinst→preinstall, etc.
            if let Some(s) = pick(scripts, &["pre", "preinst"]) {
                bodies.insert("preinstall", s);
            }
            if let Some(s) = pick(scripts, &["post", "postinst"]) {
                bodies.insert("postinstall", s);
            }
            if let Some(s) = pick(scripts, &["preun", "prerm"]) {
                bodies.insert("preremove", s);
            }
            if let Some(s) = pick(scripts, &["postun", "postrm"]) {
                bodies.insert("postremove", s);
            }
            if let Some(s) = pick(scripts, &["pretrans"]) {
                bodies.insert("pretrans", s);
            }
            if let Some(s) = pick(scripts, &["posttrans"]) {
                bodies.insert("posttrans", s);
            }
            if let Some(s) = pick(scripts, &["verify"]) {
                bodies.insert("verify", s);
            }
        }
        _ => {
            // Arch `.INSTALL` only carries pre/post-upgrade hooks; map the
            // install/remove hooks onto them (best effort, as before).
            if let Some(s) = pick(scripts, &["pre", "preinst", "preupgrade"]) {
                bodies.insert("preupgrade", s);
            }
            if let Some(s) = pick(scripts, &["post", "postinst", "postupgrade"]) {
                bodies.insert("postupgrade", s);
            }
            if !bodies.contains_key("preupgrade") {
                if let Some(s) = pick(scripts, &["preun", "prerm"]) {
                    bodies.insert("preupgrade", s);
                }
            }
            if !bodies.contains_key("postupgrade") {
                if let Some(s) = pick(scripts, &["postun", "postrm"]) {
                    bodies.insert("postupgrade", s);
                }
            }
        }
    }
    bodies
}

/// Write each mapped script body to a real file under `work_dir` and point the
/// matching config field at it. The packagers read `config.scripts.*` with
/// `fs::read`, so these fields must be paths in the build environment, not the
/// script text itself (the bug that made every script-bearing conversion fail).
fn stage_script_bodies(
    config: &mut crate::config::PackageConfig,
    bodies: &BTreeMap<&'static str, String>,
    work_dir: &Path,
) {
    for (field, body) in bodies {
        let path = stage_script(work_dir, field, body);
        match *field {
            "preinstall" => config.scripts.preinstall = path,
            "postinstall" => config.scripts.postinstall = path,
            "preremove" => config.scripts.preremove = path,
            "postremove" => config.scripts.postremove = path,
            "pretrans" => config.scripts.pretrans = path,
            "posttrans" => config.scripts.posttrans = path,
            "verify" => config.scripts.verify = path,
            "preupgrade" => config.scripts.preupgrade_script = path,
            "postupgrade" => config.scripts.postupgrade_script = path,
            _ => {}
        }
    }
}

/// Materialize one script body under `<work_dir>/scripts/<field>` and return
/// its path. Returns an empty string (which the packagers treat as "unset")
/// if the file cannot be written.
fn stage_script(work_dir: &Path, field: &str, body: &str) -> String {
    let dir = work_dir.join("scripts");
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("  ⚠ failed to stage {field} script: {e}");
        return String::new();
    }
    let path = dir.join(field);
    if let Err(e) = fs::write(&path, body) {
        eprintln!("  ⚠ failed to stage {field} script: {e}");
        return String::new();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
    }
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rpm_requires_with_constraints() {
        let deps = vec![
            rpm::Dependency::any("bash"),
            rpm::Dependency::greater_eq("glibc", "2.17"),
            rpm::Dependency::less("zlib", "1.3"),
            rpm::Dependency::any("/bin/sh"),
            rpm::Dependency::rpmlib("CompressedFileNames", "3.0.4"),
        ];
        assert_eq!(
            format_rpm_requires(&deps),
            "bash, glibc >= 2.17, zlib < 1.3"
        );
    }

    #[test]
    fn rpm_provides_keep_virtuals_and_drop_auto_capabilities() {
        let deps = vec![
            rpm::Dependency::any("hello"),
            rpm::Dependency::any("hello(x86-64)"),
            rpm::Dependency::any("webserver"),
            rpm::Dependency::any("libc.so.6()(64bit)"),
            rpm::Dependency::any("/usr/bin/hello"),
            rpm::Dependency::rpmlib("CompressedFileNames", "3.0.4"),
        ];
        assert_eq!(format_rpm_provides("hello", &deps), "webserver");
    }

    #[test]
    fn parses_deb_trigger_lines() {
        let t = parse_deb_triggers("# comment\ninterest cups\ninterest-await bar\nactivate baz\n");
        assert_eq!(t.len(), 3);
        assert_eq!(
            (t[0].kind.as_str(), t[0].name.as_str()),
            ("interest", "cups")
        );
        assert_eq!(t[1].name, "bar");
        assert_eq!(
            (t[2].kind.as_str(), t[2].name.as_str()),
            ("activate", "baz")
        );
    }

    #[test]
    fn rpm_requires_hide_trigger_dependencies() {
        let deps = vec![
            rpm::Dependency::greater_eq("glibc", "2.17"),
            rpm::Dependency {
                name: "cups".into(),
                flags: rpm::DependencyFlags::TRIGGERIN,
                version: String::new(),
            },
        ];
        assert_eq!(format_rpm_requires(&deps), "glibc >= 2.17");
    }

    #[test]
    fn reads_lx_style_rpm_triggers() {
        use lx_lib::rpmarchive::{self, BuildOptions, PackageMeta, RpmTrigger};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
        let rpm_path = dir.path().join("hello.rpm");
        rpmarchive::build_with_options(
            &root,
            &PackageMeta {
                name: "hello",
                version: "1.0",
                release: "1",
                summary: "s",
                description: "d",
                license: "MIT",
                vendor: None,
                packager: None,
            },
            "amd64",
            0,
            &rpm_path,
            &BuildOptions {
                triggers: vec![RpmTrigger {
                    package: "cups".into(),
                    script: String::new(),
                }],
                trigger_flags: vec![rpm::DependencyFlags::TRIGGERIN],
                ..Default::default()
            },
            &lx_lib::filemeta::FileMetaMap::new(),
        )
        .unwrap();
        let pkg = rpm::Package::open(&rpm_path).unwrap();
        let triggers = rpm_triggers(&pkg);
        assert!(
            triggers
                .iter()
                .any(|t| t.kind == "triggerin" && t.name == "cups"),
            "{triggers:?}"
        );
    }

    #[test]
    fn trigger_mapping_carries_both_directions() {
        let mut cfg = crate::config::PackageConfig::default();
        let mut bodies: BTreeMap<&'static str, String> = BTreeMap::new();
        apply_triggers_to_config(
            &mut cfg,
            &mut bodies,
            &[Trigger {
                kind: "triggerin".into(),
                name: "bar".into(),
                script: "echo hi".into(),
            }],
            "deb",
        );
        assert!(cfg.deb.triggers_interest.contains(&"bar".to_string()));
        assert!(bodies
            .get("postinstall")
            .map(String::as_str)
            .unwrap_or("")
            .contains("echo hi"));

        let mut cfg = crate::config::PackageConfig::default();
        let mut bodies: BTreeMap<&'static str, String> = BTreeMap::new();
        apply_triggers_to_config(
            &mut cfg,
            &mut bodies,
            &[Trigger {
                kind: "interest".into(),
                name: "foo".into(),
                script: String::new(),
            }],
            "rpm",
        );
        assert_eq!(cfg.rpm.trigger_post_install, vec!["foo:".to_string()]);
    }

    #[test]
    fn splits_debian_epoch_only_when_numeric() {
        assert_eq!(
            split_deb_epoch("1:2.3-1"),
            ("1".to_string(), "2.3-1".to_string())
        );
        assert_eq!(
            split_deb_epoch("2.3-1"),
            (String::new(), "2.3-1".to_string())
        );
        assert_eq!(
            split_deb_epoch("abc:1"),
            (String::new(), "abc:1".to_string())
        );
    }

    #[test]
    fn normalizes_arch_through_the_debian_name() {
        assert_eq!(normalize_arch("x86_64", "rpm", "deb"), "amd64");
        assert_eq!(normalize_arch("aarch64", "arch", "deb"), "arm64");
        assert_eq!(normalize_arch("amd64", "deb", "rpm"), "x86_64");
        assert_eq!(normalize_arch("amd64", "deb", "arch"), "x86_64");
        // Unknown architectures pass through.
        assert_eq!(normalize_arch("mips64el", "deb", "deb"), "mips64el");
    }

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
        // Normalized to `op version` so every target formats it correctly.
        assert_eq!(ver.as_deref(), Some(">= 2.17"));
    }

    #[test]
    fn arch_dep_syntax_normalizes_to_every_target() {
        // arch source -> deb (parenthesized), rpm (spaces), arch (compact).
        let deb = map_dep_syntax("libcurl>=7.0", "arch", "deb");
        assert_eq!(deb, "libcurl (>= 7.0)");
        let rpm = map_dep_syntax("libcurl>=7.0", "arch", "rpm");
        assert_eq!(rpm, "libcurl >= 7.0");
        let arch = map_dep_syntax("libcurl>=7.0", "arch", "arch");
        assert_eq!(arch, "libcurl>=7.0");
        // deb source -> arch must not leave a space after the operator.
        assert_eq!(
            map_dep_syntax("libc6 (>= 2.36)", "deb", "arch"),
            "libc6>=2.36"
        );
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
            "glibc (>= 2.17)"
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
        assert_eq!(
            map_dep_syntax("glibc>=2.17", "arch", "rpm"),
            "glibc >= 2.17"
        );
    }

    #[test]
    fn map_dep_bare_passthrough() {
        assert_eq!(map_dep_syntax("glibc", "arch", "deb"), "glibc");
    }
}
