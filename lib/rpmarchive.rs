// SPDX-License-Identifier: GPL-3.0-or-later

//! Build a `.rpm` archive entirely in-process using the `rpm` crate.
//!
//! Mirrors `debarchive.rs`'s philosophy: no `rpmbuild`, no
//! subprocess. The output is a genuine RPM that `rpm -qip` / `dnf` accept.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;

/// RPM header tag fields shared by [`build`], [`build_with_options`], and
/// [`build_srpm`].
#[derive(Debug, Clone, Copy)]
pub struct PackageMeta<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub release: &'a str,
    pub summary: &'a str,
    pub description: &'a str,
    pub license: &'a str,
    /// RPM `vendor` header tag (e.g. "Fedora Project"). Optional.
    pub vendor: Option<&'a str>,
    /// RPM `packager` header tag (e.g. "Jane Doe <jane@example.com>").
    /// Optional; falls back to maintainer-style identity when unset.
    pub packager: Option<&'a str>,
}

/// Build an `.rpm` from a staged filesystem tree.
///
/// `root` is the staged tree; its contents become the RPM payload
/// (e.g. `root/usr/bin/foo` becomes `/usr/bin/foo` in the installed system).
/// `meta` fields map directly to RPM header tags. `mtime` is used as
/// `source_date` for reproducibility.
pub fn build(
    root: &Path,
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    rpm_path: &Path,
) -> Result<()> {
    build_with_options(root, meta, arch, mtime, rpm_path, &BuildOptions::default())
}

/// RPM relation fields (requires, provides, conflicts, obsoletes,
/// recommends, suggests) parsed from the package config's comma-separated
/// relation strings. Each entry is a name-only `Dependency::any(name)`.
///
/// Note: `rpm::Dependency` does not implement `Clone`, so this struct
/// manually reconstructs dependencies from its public fields where needed.
#[derive(Debug, Default)]
pub struct RpmRelations {
    pub requires: Vec<rpm::Dependency>,
    pub provides: Vec<rpm::Dependency>,
    pub conflicts: Vec<rpm::Dependency>,
    pub obsoletes: Vec<rpm::Dependency>,
    pub recommends: Vec<rpm::Dependency>,
    pub suggests: Vec<rpm::Dependency>,
}

impl RpmRelations {
    /// Rebuild a `Dependency` from its public fields (the type does not
    /// implement `Clone`).
    fn rebuild(dep: &rpm::Dependency) -> rpm::Dependency {
        rpm::Dependency {
            name: dep.name.clone(),
            flags: dep.flags,
            version: dep.version.clone(),
        }
    }
}

/// Parse comma-separated relation strings into [`RpmRelations`]. Each
/// non-empty, trimmed entry becomes a `Dependency::any(name)`. Empty or
/// whitespace-only strings yield empty vectors.
///
/// Debian's `Pre-Depends` has no exact RPM equivalent; entries are folded
/// into `Requires` with the legacy `PREREQ` flag, which is the closest RPM
/// ordering guarantee. Debian `Breaks` has no direct RPM equivalent either
/// and is folded into `Conflicts`.
#[allow(clippy::too_many_arguments)]
pub fn parse_rpm_relations(
    depends: &str,
    recommends: &str,
    suggests: &str,
    conflicts: &str,
    replaces: &str,
    provides: &str,
    breaks: &str,
    predepends: &str,
) -> RpmRelations {
    fn parse_list(s: &str) -> Vec<rpm::Dependency> {
        s.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(rpm::Dependency::any)
            .collect()
    }

    // Debian `Breaks` has no direct RPM equivalent; fold it into conflicts.
    let mut conflicts = parse_list(conflicts);
    conflicts.extend(parse_list(breaks));

    let mut requires = parse_list(depends);
    // Pre-Depends → Requires with the legacy PREREQ flag.
    for name in predepends
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !requires.iter().any(|d| d.name == name) {
            requires.push(rpm::Dependency {
                name: name.to_string(),
                flags: rpm::DependencyFlags::PREREQ,
                version: String::new(),
            });
        }
    }

    RpmRelations {
        requires,
        recommends: parse_list(recommends),
        suggests: parse_list(suggests),
        conflicts,
        obsoletes: parse_list(replaces),
        provides: parse_list(provides),
    }
}

/// An RPM trigger: fires a script when a named package is installed or
/// removed. Mirrors fpm's `--rpm-trigger-*` flags. The `script` body is the
/// shell code to run; `package` is the package that triggers it.
///
/// Note: full trigger script emission requires rpmbuild or a future
/// rpm-crate version. The trigger *dependency* (the condition) is always
/// emitted; the script is best-effort.
#[derive(Debug, Clone, Default)]
pub struct RpmTrigger {
    /// Package name that fires the trigger (the condition).
    pub package: String,
    /// Script body to run when the trigger fires.
    pub script: String,
}

/// Optional scriptlets, triggers, and signing for [`build_with_options`].
#[derive(Debug, Default)]
pub struct BuildOptions<'a> {
    /// `%pre` scriptlet body (`scripts.preinstall`).
    pub pre_install: Option<&'a str>,
    /// `%post` scriptlet body (`scripts.postinstall`).
    pub post_install: Option<&'a str>,
    /// `%preun` scriptlet body (`scripts.preremove`).
    pub pre_uninstall: Option<&'a str>,
    /// `%postun` scriptlet body (`scripts.postremove`).
    pub post_uninstall: Option<&'a str>,
    /// `%pretrans` scriptlet body (`scripts.pretrans`). Transaction-level
    /// scriptlet that runs before any package in the transaction.
    pub pre_trans: Option<&'a str>,
    /// `%posttrans` scriptlet body (`scripts.posttrans`). Transaction-level
    /// scriptlet that runs after the entire transaction completes.
    pub post_trans: Option<&'a str>,
    /// `%verify` scriptlet body (`scripts.verify`). Runs when `rpm -V`
    /// verifies the package.
    pub verify_script: Option<&'a str>,
    /// When set, an armored secret key is loaded natively and the PGP
    /// signature is embedded in the RPM header (`rpm -K` verifiable).
    pub sign_key_file: Option<&'a Path>,
    /// Passphrase for the signing key, if any.
    pub sign_passphrase: Option<&'a str>,
    /// RPM relation fields (requires, provides, conflicts, obsoletes,
    /// recommends, suggests).
    pub relations: RpmRelations,
    /// RPM triggers (pre/post install/uninstall). The trigger dependencies
    /// are emitted; scripts are stored as `%trigger*` scriptlets when the
    /// rpm crate supports it.
    pub triggers: Vec<RpmTrigger>,
    /// Flags for each trigger, parallel to `triggers`. One of
    /// `TRIGGERPREIN`, `TRIGGERIN`, `TRIGGERUN`, `TRIGGERPOSTUN`.
    pub trigger_flags: Vec<rpm::DependencyFlags>,
    /// Payload compression algorithm. One of: `gzip`, `xz`, `lzma`, `zstd`,
    /// `none`. Empty means rpm crate default (gzip).
    /// Mirrors fpm's `--rpm-compression` and nfpm's `rpm.compression`.
    pub compression: String,
    /// Auto-generate `Provides:` for every file/shared library the package
    /// installs (rpm's `--auto-provides`). Default: true.
    pub auto_provides: bool,
    /// Auto-generate `Requires:` from shared-library dependencies detected
    /// in the payload (rpm's `--auto-requires`). Default: true.
    pub auto_requires: bool,
    /// rpmbuild-style macro definitions (e.g. `_unpackaged_files_terminate_build 0`).
    /// Each entry is a `"KEY VALUE"` string. The in-process builder has no
    /// macro engine, so these are reported as unsupported (a warning) rather
    /// than silently ignored.
    pub defines: Vec<String>,
    /// Absolute installed paths marked `%config`.
    pub config_files: Vec<String>,
    /// Absolute installed paths marked `%config(noreplace)`.
    pub config_noreplace_files: Vec<String>,
    /// RPM epoch header tag. `None` means 0 (the RPM default).
    pub epoch: Option<u32>,
}

/// Parse `"package: script_path"` trigger entries from config. The script
/// path is read from the build environment. Returns triggers and their
/// flags for the given trigger kind.
pub fn parse_rpm_triggers(
    entries: &[String],
    flags: rpm::DependencyFlags,
    build_dir: &std::path::Path,
) -> anyhow::Result<Vec<(RpmTrigger, rpm::DependencyFlags)>> {
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (package, script_path) = entry.split_once(':').ok_or_else(|| {
            anyhow::anyhow!(
                "invalid rpm trigger entry '{}' (expected 'package: script_path')",
                entry
            )
        })?;
        let package = package.trim();
        let script_path = script_path.trim();
        if package.is_empty() {
            continue;
        }
        let script = if script_path.is_empty() {
            String::new()
        } else {
            let path = build_dir.join(script_path);
            std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read trigger script '{}'", path.display()))?
        };
        out.push((
            RpmTrigger {
                package: package.to_string(),
                script,
            },
            flags,
        ));
    }
    Ok(out)
}

/// Like [`build`] plus scriptlets (`%pre`/`%post`/`%preun`/`%postun`) and
/// optional native embedded PGP signing.
pub fn build_with_options(
    root: &Path,
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    rpm_path: &Path,
    opts: &BuildOptions,
) -> Result<()> {
    let rpm_arch = to_rpm_arch(arch);

    let mut builder = rpm::PackageBuilder::new(
        meta.name,
        meta.version,
        meta.license,
        rpm_arch,
        meta.summary,
    )
    .description(meta.description)
    .release(meta.release)
    .source_date(mtime.max(0) as u32);

    // Apply the epoch header tag when the config pins one.
    if let Some(e) = opts.epoch {
        builder = builder.epoch(e);
    }

    // Apply payload compression if specified (mirrors fpm's --rpm-compression).
    if !opts.compression.is_empty() {
        if let Ok(comp) = opts.compression.parse::<rpm::CompressionType>() {
            builder = builder.compression(comp);
        }
    }

    if let Some(s) = opts.pre_install.map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.pre_install_script(s);
    }
    if let Some(s) = opts.post_install.map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.post_install_script(s);
    }
    if let Some(s) = opts.pre_uninstall.map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.pre_uninstall_script(s);
    }
    if let Some(s) = opts.post_uninstall.map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.post_uninstall_script(s);
    }
    if let Some(s) = opts.pre_trans.map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.pre_trans_script(s);
    }
    if let Some(s) = opts.post_trans.map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.post_trans_script(s);
    }
    if let Some(s) = opts.verify_script.map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.verify_script(s);
    }

    // Apply relation fields (requires, provides, conflicts, obsoletes,
    // recommends, suggests). `rpm::Dependency` does not implement `Clone`,
    // so rebuild each from its public fields.
    for dep in &opts.relations.requires {
        builder = builder.requires(RpmRelations::rebuild(dep));
    }
    for dep in &opts.relations.provides {
        builder = builder.provides(RpmRelations::rebuild(dep));
    }
    for dep in &opts.relations.conflicts {
        builder = builder.conflicts(RpmRelations::rebuild(dep));
    }
    for dep in &opts.relations.obsoletes {
        builder = builder.obsoletes(RpmRelations::rebuild(dep));
    }
    for dep in &opts.relations.recommends {
        builder = builder.recommends(RpmRelations::rebuild(dep));
    }
    for dep in &opts.relations.suggests {
        builder = builder.suggests(RpmRelations::rebuild(dep));
    }

    // Best-effort `--auto-requires` / `--auto-provides` (find-requires /
    // find-provides analogues over the staged ELF payload).
    if opts.auto_provides || opts.auto_requires {
        let (auto_requires, auto_provides) = auto_elf_relations(root);
        if opts.auto_requires {
            // A package never needs to require a soname it provides itself.
            let explicit: std::collections::HashSet<&str> = opts
                .relations
                .provides
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            let mut seen: std::collections::HashSet<String> = opts
                .relations
                .requires
                .iter()
                .map(|d| d.name.clone())
                .collect();
            for name in auto_requires {
                if !explicit.contains(name.as_str()) && seen.insert(name.clone()) {
                    builder = builder.requires(rpm::Dependency::any(name));
                }
            }
        }
        if opts.auto_provides {
            let mut seen: std::collections::HashSet<String> = opts
                .relations
                .provides
                .iter()
                .map(|d| d.name.clone())
                .collect();
            for name in auto_provides {
                if seen.insert(name.clone()) {
                    builder = builder.provides(rpm::Dependency::any(name));
                }
            }
        }
    }

    // `rpm.defines` needs an rpmbuild macro engine; the in-process builder
    // has none. Report it instead of silently dropping it.
    if !opts.defines.is_empty() {
        eprintln!(
            "  ⚠ rpm.defines is ignored: the in-process builder has no rpmbuild macro engine"
        );
    }

    // Apply RPM triggers as dependencies with trigger flags. The trigger
    // dependency (the condition) is always emitted. The associated script
    // is emitted alongside as a best-effort scriptlet (true RPM triggers
    // need %triggerin/%triggerun scriptlets which the rpm crate doesn't
    // yet expose natively).
    for (trigger, flags) in opts.triggers.iter().zip(opts.trigger_flags.iter()) {
        let dep = rpm::Dependency {
            name: trigger.package.clone(),
            flags: *flags,
            version: String::new(),
        };
        builder = builder.requires(dep);
        // Best-effort: emit the trigger script as a post-install scriptlet.
        // A full implementation would use %triggerin/%triggerun scriptlets
        // via rpmbuild or a future rpm-crate version.
        if !trigger.script.trim().is_empty() {
            builder = builder.post_install_script(trigger.script.trim());
        }
    }

    // Apply vendor and packager header tags when present.
    if let Some(v) = meta.vendor.filter(|v| !v.trim().is_empty()) {
        builder = builder.vendor(v);
    }
    if let Some(p) = meta.packager.filter(|p| !p.trim().is_empty()) {
        builder = builder.packager(p);
    }

    // Walk staged tree in sorted order for reproducibility, adding each
    // regular file and symlink as an RPM file entry.
    // We need a temp empty file for symlink entries (rpm crate reads a real
    // file; for symlinks the content is irrelevant and the link target is
    // stored in the header).
    let empty_file = tempfile::NamedTempFile::new()?;
    std::fs::write(empty_file.path(), b"")?;

    // Absolute installed path -> `%config(noreplace)`? (`%config` otherwise.)
    let mut config_map: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for f in &opts.config_files {
        config_map.insert(f.clone(), false);
    }
    for f in &opts.config_noreplace_files {
        config_map.insert(f.clone(), true);
    }

    add_dir_recursive(root, root, &mut builder, empty_file.path(), &config_map)?;

    // Sign during the same build pass when configured — `build_and_sign`
    // embeds the PGP signature header before anything is written.
    let signer;
    if let Some(key_file) = opts.sign_key_file {
        let key_bytes = crate::sign::SignRequest {
            key_file,
            key_id: "",
            passphrase: opts.sign_passphrase,
        }
        .key_bytes()?;
        let mut s = rpm::signature::pgp::Signer::load_from_asc_bytes(&key_bytes)
            .context("failed to load RPM signing key")?;
        if let Some(pass) = opts.sign_passphrase.filter(|p| !p.is_empty()) {
            s = s.with_key_passphrase(pass);
        }
        signer = Some(s);
    } else {
        signer = None;
    }

    let pkg = match &signer {
        Some(s) => builder
            .build_and_sign(s.clone())
            .context("failed to build+sign rpm package")?,
        None => builder.build().context("failed to build rpm package")?,
    };

    let mut out = std::fs::File::create(rpm_path)
        .with_context(|| format!("failed to create '{}'", rpm_path.display()))?;
    pkg.write(&mut out).context("failed to write .rpm")?;
    Ok(())
}

fn add_dir_recursive(
    original_root: &Path,
    dir: &Path,
    builder: &mut rpm::PackageBuilder,
    empty_file: &Path,
    config: &std::collections::HashMap<String, bool>,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read '{}'", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let fs_path = entry.path();
        let rel = fs_path
            .strip_prefix(original_root)
            .expect("walked path must be under original_root");
        // RPM payload paths are absolute (e.g. /usr/bin/foo)
        let rpm_path = format!("/{}", rel.to_string_lossy());

        let ft = entry.file_type()?;
        if ft.is_symlink() {
            let target = std::fs::read_link(&fs_path)?;
            // rpm expects symlink target via FileOptions::symlink and a dummy
            // source file. Mode includes symlink type bits (0o120777).
            let opts = rpm::FileOptions::new(rpm_path)
                .symlink(target.to_string_lossy().to_string())
                .mode(0o120777i32);
            // Use empty file as source; content is ignored for symlink.
            let b = std::mem::replace(builder, rpm::PackageBuilder::new("", "", "", "", ""));
            let nb = b
                .with_file(empty_file, opts)
                .context("failed to add symlink")?;
            *builder = nb;
        } else if ft.is_dir() {
            // RPM implicitly creates directories for files; we recurse but do
            // not add empty directory entries themselves.
            add_dir_recursive(original_root, &fs_path, builder, empty_file, config)?;
        } else if ft.is_file() {
            // Inherit mode from source file; rpm crate will read it if we
            // don't override, but we set explicitly for determinism.
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&fs_path)?.permissions().mode() & 0o777
            };
            #[cfg(not(unix))]
            let mode = 0o644u32;

            // FileMode expects full mode with type bits; regular file is 0o100000 + perms.
            let file_mode = 0o100000 | mode;
            let mut opts = rpm::FileOptions::new(rpm_path.clone()).mode(file_mode as i32);
            // `%config` / `%config(noreplace)` marking for contents entries.
            if let Some(&noreplace) = config.get(&rpm_path) {
                opts = if noreplace {
                    opts.is_config_noreplace()
                } else {
                    opts.is_config()
                };
            }
            let b = std::mem::replace(builder, rpm::PackageBuilder::new("", "", "", "", ""));
            let nb = b
                .with_file(&fs_path, opts)
                .with_context(|| format!("failed to add file '{}'", fs_path.display()))?;
            *builder = nb;
        }
    }
    Ok(())
}

/// Best-effort find-requires / find-provides over the staged payload.
///
/// `requires` collects every `DT_NEEDED` soname (minus the dynamic loader)
/// as RPM's `name()(N bit)`; `provides` collects the same form for staged
/// files that look like shared libraries, using the file name as the
/// SONAME. This is the in-process analogue of rpmbuild's
/// `%__find_requires`/`%__find_provides` helpers.
fn auto_elf_relations(root: &Path) -> (Vec<String>, Vec<String>) {
    use std::collections::BTreeSet;
    let mut requires = BTreeSet::new();
    let mut provides = BTreeSet::new();
    let Ok(files) = crate::scandeps::find_elf_files(root) else {
        return (Vec::new(), Vec::new());
    };
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        // EI_CLASS: 1 = 32-bit, 2 = 64-bit. find_elf_files already validated
        // the ELF magic.
        let bits = match bytes.get(4) {
            Some(1) => 32,
            _ => 64,
        };
        if let Ok(libs) = crate::elfdeps::needed_libraries(&bytes) {
            for lib in libs {
                if is_dynamic_loader(&lib) {
                    continue;
                }
                requires.insert(format!("{lib}()({bits}bit)"));
            }
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("lib") && name.contains(".so") {
                provides.insert(format!("{name}()({bits}bit)"));
                provides.insert(name.to_string());
            }
        }
    }
    (
        requires.into_iter().collect(),
        provides.into_iter().collect(),
    )
}

/// The dynamic linker is loaded by the kernel, not provided as an RPM
/// dependency, so find-requires skips it.
fn is_dynamic_loader(soname: &str) -> bool {
    soname.starts_with("ld-") || soname.starts_with("linux-vdso")
}

/// Map Debian architecture names to RPM architecture names.
pub fn to_rpm_arch(debian_arch: &str) -> &'static str {
    crate::constants::to_rpm_arch(debian_arch)
}

/// Build a source RPM (`.src.rpm`): a spec file plus one source tarball,
/// stored flat (no installed paths) with `arch = "src"`. Unlike [`build`],
/// there is no staged filesystem tree to walk -- the two members are given
/// directly as bytes.
pub fn build_srpm(
    meta: &PackageMeta,
    spec: (&str, &[u8]),
    source: (&str, &[u8]),
    mtime: i64,
    srpm_path: &Path,
) -> Result<()> {
    let (spec_name, spec_bytes) = spec;
    let (source_name, source_bytes) = source;
    let builder =
        rpm::PackageBuilder::new(meta.name, meta.version, meta.license, "src", meta.summary)
            .description(meta.description)
            .release(meta.release)
            .source_date(mtime.max(0) as u32);

    let empty_dir = tempfile::tempdir()?;
    let spec_path = empty_dir.path().join(spec_name);
    std::fs::write(&spec_path, spec_bytes)?;
    let source_path = empty_dir.path().join(source_name);
    std::fs::write(&source_path, source_bytes)?;

    let builder = builder
        .with_file(
            &spec_path,
            rpm::FileOptions::new(format!("./{spec_name}")).mode(0o100644i32),
        )
        .context("failed to add spec file to srpm")?
        .with_file(
            &source_path,
            rpm::FileOptions::new(format!("./{source_name}")).mode(0o100644i32),
        )
        .context("failed to add source tarball to srpm")?;

    let pkg = builder.build().context("failed to build srpm")?;
    let mut out = std::fs::File::create(srpm_path)
        .with_context(|| format!("failed to create '{}'", srpm_path.display()))?;
    pkg.write(&mut out).context("failed to write .src.rpm")?;
    Ok(())
}

/// Extract an `.rpm`'s cpio payload into `dest` (the inverse of [`build`]).
/// Reconstructs regular files, directories, and symlinks (whose targets
/// live in the `RPMTAG_FILELINKTOS` header entry, not the cpio payload --
/// see `add_dir_recursive`'s symlink handling above).
pub fn extract(rpm_path: &Path, dest: &Path) -> Result<()> {
    let pkg = rpm::Package::open(rpm_path)
        .with_context(|| format!("failed to open '{}'", rpm_path.display()))?;

    let linkto: std::collections::HashMap<String, String> = pkg
        .metadata
        .get_file_entries()
        .context("failed to read rpm file entries")?
        .into_iter()
        .map(|e| (e.path.to_string_lossy().to_string(), e.linkto))
        .collect();

    let compressor = pkg
        .metadata
        .get_payload_compressor()
        .unwrap_or(rpm::CompressionType::None);
    let payload = decompress_payload(&pkg.content, compressor)?;

    std::fs::create_dir_all(dest)?;
    let mut cursor: &[u8] = &payload;
    loop {
        let reader = cpio::newc::Reader::new(cursor).context("failed to read cpio entry header")?;
        let entry = reader.entry().clone();
        if entry.is_trailer() {
            break;
        }
        let name = entry.name().trim_start_matches("./").to_string();
        let out_path = dest.join(&name);
        let file_type = entry.mode() & 0o170000;

        if file_type == 0o040000 {
            std::fs::create_dir_all(&out_path)?;
            cursor = reader
                .to_writer(std::io::sink())
                .context("failed to skip cpio directory entry")?;
        } else if file_type == 0o120000 {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let abs_name = format!("/{name}");
            let target = linkto.get(&abs_name).cloned().unwrap_or_default();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &out_path)
                .with_context(|| format!("failed to symlink '{}'", out_path.display()))?;
            cursor = reader
                .to_writer(std::io::sink())
                .context("failed to skip cpio symlink entry")?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out_path)
                .with_context(|| format!("failed to create '{}'", out_path.display()))?;
            cursor = reader
                .to_writer(&mut f)
                .with_context(|| format!("failed to extract '{}'", out_path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    &out_path,
                    std::fs::Permissions::from_mode(entry.mode() & 0o7777),
                )?;
            }
        }
    }
    Ok(())
}

fn decompress_payload(content: &[u8], compressor: rpm::CompressionType) -> Result<Vec<u8>> {
    match compressor {
        rpm::CompressionType::None => Ok(content.to_vec()),
        rpm::CompressionType::Gzip => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(content).read_to_end(&mut out)?;
            Ok(out)
        }
        rpm::CompressionType::Zstd => {
            zstd::stream::decode_all(content).context("failed to zstd-decode rpm payload")
        }
        rpm::CompressionType::Xz => {
            let mut out = Vec::new();
            lzma_rust2::XzReader::new(content, true).read_to_end(&mut out)?;
            Ok(out)
        }
        other => bail!("unsupported rpm payload compressor: {other:?}"),
    }
}
