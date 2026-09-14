// SPDX-License-Identifier: GPL-3.0-or-later

//! Build a `.rpm` archive entirely in-process using the `rpm` crate.
//!
//! Mirrors `debarchive.rs`'s philosophy: no `rpmbuild`, no
//! subprocess. The output is a genuine RPM that `rpm -qip` / `dnf` accept.

use anyhow::{bail, Context, Result};
use sha2::Digest as _;
use std::io::Read;
use std::path::Path;

use crate::filemeta::FileMetaMap;

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
    build_with_options(
        root,
        meta,
        arch,
        mtime,
        rpm_path,
        &BuildOptions::default(),
        &FileMetaMap::new(),
    )
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
            .map(parse_rpm_relation)
            .collect()
    }

    // Debian `Breaks` has no direct RPM equivalent; fold it into conflicts.
    let mut conflicts = parse_list(conflicts);
    conflicts.extend(parse_list(breaks));

    let mut requires = parse_list(depends);
    // Pre-Depends → Requires with the legacy PREREQ flag.
    for clause in predepends
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let dep = parse_rpm_relation(clause);
        if !requires.iter().any(|d| d.name == dep.name) {
            requires.push(rpm::Dependency {
                name: dep.name,
                flags: dep.flags | rpm::DependencyFlags::PREREQ,
                version: dep.version,
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

/// Parse one relation clause into an [`rpm::Dependency`], accepting Debian
/// (`name (>= 1.2)`), RPM (`name >= 1.2`) and pacman (`name>=1.2`) spellings.
///
/// Previously every clause was passed to `rpm::Dependency::any`, so a
/// deb-style `depends: libc6 (>= 2.36)` became a requirement literally named
/// `libc6 (>= 2.36)` — with no version and no flags.
fn parse_rpm_relation(clause: &str) -> rpm::Dependency {
    // Debian alternatives (`a | b`) have no RPM equivalent; keep the first.
    let clause = clause.split('|').next().unwrap_or(clause).trim();
    if let Some(idx) = clause.find('(') {
        let name = clause[..idx].trim();
        let inner = clause[idx..]
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();
        if let Some((op, version)) = split_relation_op(inner) {
            return relation_dependency(name, op, version);
        }
        return rpm::Dependency::any(name);
    }
    // RPM / pacman form: `name op version`, spaces optional. Debian's strict
    // operators `<<`/`>>` are accepted too.
    for op in [">=", "<=", ">>", "<<", ">", "<", "="] {
        if let Some(idx) = clause.find(op) {
            let name = clause[..idx].trim();
            let version = clause[idx + op.len()..].trim();
            return relation_dependency(name, op, version);
        }
    }
    rpm::Dependency::any(clause)
}

/// Split a constraint body (`>= 1.2`, `<< 2.0`) into its operator and version.
fn split_relation_op(s: &str) -> Option<(&'static str, &str)> {
    for op in [">=", "<=", ">>", "<<", ">", "<", "="] {
        if let Some(rest) = s.strip_prefix(op) {
            return Some((op, rest.trim()));
        }
    }
    None
}

fn relation_dependency(name: &str, op: &str, version: &str) -> rpm::Dependency {
    if name.is_empty() {
        return rpm::Dependency::any(name);
    }
    match op {
        ">=" => rpm::Dependency::greater_eq(name, version),
        "<=" => rpm::Dependency::less_eq(name, version),
        ">" | ">>" => rpm::Dependency::greater(name, version),
        "<" | "<<" => rpm::Dependency::less(name, version),
        "=" => rpm::Dependency::eq(name, version),
        _ => rpm::Dependency::any(name),
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
    /// RPM `BuildHost:` header override (nfpm `rpm.buildhost`). Empty means
    /// the rpm crate default.
    pub buildhost: String,
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
    file_meta: &FileMetaMap,
) -> Result<()> {
    let rpm_arch = to_rpm_arch(arch);

    let builder = rpm::PackageBuilder::new(
        meta.name,
        meta.version,
        meta.license,
        rpm_arch,
        meta.summary,
    )
    .description(meta.description)
    .release(meta.release)
    .source_date(mtime.max(0) as u32);

    let builder = apply_meta_options(builder, opts);
    let builder = apply_scriptlets(builder, opts);
    let builder = apply_relations(builder, &opts.relations);
    let builder = apply_auto_relations(builder, root, opts);
    warn_ignored_defines(opts);
    let builder = apply_triggers(builder, opts);
    let mut builder = apply_vendor_packager(builder, meta);

    // Symlink entries need a real (empty) backing file; the link target goes
    // in the header.
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

    add_dir_recursive(
        root,
        root,
        &mut builder,
        empty_file.path(),
        &config_map,
        file_meta,
    )?;

    let signer = load_signer(opts)?;
    finalize_package(builder, signer, file_meta, mtime, rpm_path)
}

/// Epoch / buildhost / compression header tags.
///
/// `rpm.group` is *not* applied here: the `rpm` crate hardcodes RPMTAG_GROUP
/// to "Unspecified" and ignores `PackageBuilder::group`; the plugin reports it.
fn apply_meta_options(
    mut builder: rpm::PackageBuilder,
    opts: &BuildOptions,
) -> rpm::PackageBuilder {
    if let Some(e) = opts.epoch {
        builder = builder.epoch(e);
    }
    if !opts.buildhost.trim().is_empty() {
        builder = builder.build_host(opts.buildhost.clone());
    }
    if !opts.compression.is_empty() {
        if let Ok(comp) = opts.compression.parse::<rpm::CompressionType>() {
            builder = builder.compression(comp);
        }
    }
    builder
}

/// The seven RPM scriptlets (`%pre`/`%post`/…), each skipped when blank.
fn apply_scriptlets(mut builder: rpm::PackageBuilder, opts: &BuildOptions) -> rpm::PackageBuilder {
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
    builder
}

/// Relation fields (requires, provides, conflicts, obsoletes, recommends,
/// suggests). `rpm::Dependency` is not `Clone`, so each is rebuilt.
fn apply_relations(mut builder: rpm::PackageBuilder, rel: &RpmRelations) -> rpm::PackageBuilder {
    for dep in &rel.requires {
        builder = builder.requires(RpmRelations::rebuild(dep));
    }
    for dep in &rel.provides {
        builder = builder.provides(RpmRelations::rebuild(dep));
    }
    for dep in &rel.conflicts {
        builder = builder.conflicts(RpmRelations::rebuild(dep));
    }
    for dep in &rel.obsoletes {
        builder = builder.obsoletes(RpmRelations::rebuild(dep));
    }
    for dep in &rel.recommends {
        builder = builder.recommends(RpmRelations::rebuild(dep));
    }
    for dep in &rel.suggests {
        builder = builder.suggests(RpmRelations::rebuild(dep));
    }
    builder
}

/// Best-effort `--auto-requires` / `--auto-provides` (find-requires /
/// find-provides analogues over the staged ELF payload).
fn apply_auto_relations(
    mut builder: rpm::PackageBuilder,
    root: &Path,
    opts: &BuildOptions,
) -> rpm::PackageBuilder {
    if !(opts.auto_provides || opts.auto_requires) {
        return builder;
    }
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
    builder
}

/// `rpm.defines` needs an rpmbuild macro engine the in-process builder lacks;
/// report it instead of silently dropping it.
fn warn_ignored_defines(opts: &BuildOptions) {
    if !opts.defines.is_empty() {
        eprintln!(
            "  ⚠ rpm.defines is ignored: the in-process builder has no rpmbuild macro engine"
        );
    }
}

/// Trigger dependencies are always emitted; the script is a best-effort
/// post-install scriptlet (true `%triggerin` needs rpmbuild).
fn apply_triggers(mut builder: rpm::PackageBuilder, opts: &BuildOptions) -> rpm::PackageBuilder {
    for (trigger, flags) in opts.triggers.iter().zip(opts.trigger_flags.iter()) {
        let dep = rpm::Dependency {
            name: trigger.package.clone(),
            flags: *flags,
            version: String::new(),
        };
        builder = builder.requires(dep);
        if !trigger.script.trim().is_empty() {
            builder = builder.post_install_script(trigger.script.trim());
        }
    }
    builder
}

/// Vendor and packager header tags when present.
fn apply_vendor_packager(
    mut builder: rpm::PackageBuilder,
    meta: &PackageMeta,
) -> rpm::PackageBuilder {
    if let Some(v) = meta.vendor.filter(|v| !v.trim().is_empty()) {
        builder = builder.vendor(v);
    }
    if let Some(p) = meta.packager.filter(|p| !p.trim().is_empty()) {
        builder = builder.packager(p);
    }
    builder
}

/// Load the native PGP signer when a key is configured.
fn load_signer(opts: &BuildOptions) -> Result<Option<rpm::signature::pgp::Signer>> {
    let Some(key_file) = opts.sign_key_file else {
        return Ok(None);
    };
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
    Ok(Some(s))
}

/// Build, inject `file_info.lang` (the crate hardcodes every FILELANGS slot to
/// empty), apply any signature over the final header, and write the file.
///
/// With no languages to inject this is exactly `build()` followed by an
/// optional `sign_with_timestamp`, i.e. the same output as `build_and_sign`.
fn finalize_package(
    builder: rpm::PackageBuilder,
    signer: Option<rpm::signature::pgp::Signer>,
    file_meta: &FileMetaMap,
    mtime: i64,
    rpm_path: &Path,
) -> Result<()> {
    let mut pkg = builder.build().context("failed to build rpm package")?;
    let mut bytes = Vec::new();
    pkg.write(&mut bytes).context("failed to serialize .rpm")?;

    if let Some(langs) = collect_file_langs(file_meta, &pkg)? {
        let (patched, header_sha256) = inject_file_langs(bytes, &langs)?;
        pkg =
            rpm::Package::parse(&mut &patched[..]).context("failed to re-read the patched .rpm")?;
        // Unsigned: refresh the digest-only signature header for the rewritten
        // header. When a key is present the signature below covers it.
        if signer.is_none() {
            pkg.metadata.signature = rpm::Header::<rpm::IndexSignatureTag>::builder()
                .add_digest(&header_sha256)
                .build();
        }
        bytes = Vec::new();
        pkg.write(&mut bytes)
            .context("failed to serialize the patched .rpm")?;
    }

    if let Some(s) = &signer {
        pkg.sign_with_timestamp(s.clone(), mtime.max(0) as u32)
            .context("failed to sign rpm package")?;
        bytes = Vec::new();
        pkg.write(&mut bytes)
            .context("failed to serialize the signed .rpm")?;
    }

    std::fs::write(rpm_path, &bytes)
        .with_context(|| format!("failed to write '{}'", rpm_path.display()))?;
    Ok(())
}

fn add_dir_recursive(
    original_root: &Path,
    dir: &Path,
    builder: &mut rpm::PackageBuilder,
    empty_file: &Path,
    config: &std::collections::HashMap<String, bool>,
    file_meta: &FileMetaMap,
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
        let entry_meta = file_meta.get(&rpm_path);

        let ft = entry.file_type()?;
        if ft.is_symlink() {
            let target = std::fs::read_link(&fs_path)?;
            // rpm expects symlink target via FileOptions::symlink and a dummy
            // source file. Mode includes symlink type bits (0o120777).
            let mut opts = rpm::FileOptions::new(rpm_path)
                .symlink(target.to_string_lossy().to_string())
                .mode(0o120777i32);
            opts = apply_rpm_ownership(opts, entry_meta);
            // Use empty file as source; content is ignored for symlink.
            let b = std::mem::replace(builder, rpm::PackageBuilder::new("", "", "", "", ""));
            let nb = b
                .with_file(empty_file, opts)
                .context("failed to add symlink")?;
            *builder = nb;
        } else if ft.is_dir() {
            // RPM implicitly creates directories for files; we recurse but do
            // not add empty directory entries themselves.
            add_dir_recursive(
                original_root,
                &fs_path,
                builder,
                empty_file,
                config,
                file_meta,
            )?;
        } else if ft.is_file() {
            // Inherit mode from source file; rpm crate will read it if we
            // don't override, but we set explicitly for determinism. A
            // `file_info.mode` override wins.
            #[cfg(unix)]
            let mode = entry_meta.and_then(|m| m.mode).unwrap_or_else(|| {
                std::fs::metadata(&fs_path)
                    .map(|m| {
                        use std::os::unix::fs::PermissionsExt;
                        m.permissions().mode() & 0o777
                    })
                    .unwrap_or(0o644)
            });
            #[cfg(not(unix))]
            let mode = entry_meta.and_then(|m| m.mode).unwrap_or(0o644);

            // FileMode expects full mode with type bits; regular file is 0o100000 + perms.
            let file_mode = 0o100000 | mode;
            let mut opts = rpm::FileOptions::new(rpm_path.clone()).mode(file_mode as i32);
            opts = apply_rpm_ownership(opts, entry_meta);
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

/// Apply `file_info.owner`/`group` names and `contents` `doc`/`license`/
/// `readme` classification to an RPM file entry.
///
/// `file_info.lang` is handled separately: the crate exposes no `%lang`
/// setter, so it is injected into the built header by [`inject_file_langs`].
fn apply_rpm_ownership(
    mut opts: rpm::FileOptionsBuilder,
    meta: Option<&crate::filemeta::FileMeta>,
) -> rpm::FileOptionsBuilder {
    let Some(meta) = meta else {
        return opts;
    };
    if let Some(owner) = &meta.owner {
        opts = opts.user(owner.clone());
    }
    if let Some(group) = &meta.group {
        opts = opts.group(group.clone());
    }
    match meta.rpm_kind {
        Some(crate::filemeta::RpmFileKind::Doc) => opts = opts.is_doc(),
        Some(crate::filemeta::RpmFileKind::License) => opts = opts.is_license(),
        Some(crate::filemeta::RpmFileKind::Readme) => opts = opts.is_readme(),
        None => {}
    }
    opts
}

// ---------------------------------------------------------------------------
// `file_info.lang` → RPMTAG_FILELANGS
//
// The `rpm` crate hardcodes every language slot to `""` (builder.rs:
// `file_langs.push("".to_string())`) and exposes no per-file setter, on any
// release through 0.28. Rather than fork the crate, the built header is
// rewritten: the language strings are appended to the data store and the
// FILELANGS index entry is repointed at them. Nothing before the new strings
// moves, so every other entry offset stays valid; the header digest is then
// refreshed (and the package re-signed, when a key is configured).
// ---------------------------------------------------------------------------

/// `RPMTAG_FILELANGS`.
const RPMTAG_FILELANGS: u32 = 1097;
/// `RPMTAG_HEADERIMMUTABLE`: the header's region descriptor.
const RPMTAG_HEADERIMMUTABLE: u32 = 63;
/// Size of the region descriptor (`HEADERIMMUTABLE`).
const RPM_REGION_DESCRIPTOR_LEN: usize = 16;
/// Fixed RPM lead length preceding the signature header.
const RPM_LEAD_LEN: usize = 96;
/// Every RPM header opens with a 16-byte index header.
const RPM_INDEX_HEADER_LEN: usize = 16;
/// Every RPM index entry is 16 bytes.
const RPM_INDEX_ENTRY_LEN: usize = 16;
/// RPM header magic (3 bytes) plus the always-1 version byte.
const RPM_HEADER_MAGIC: [u8; 4] = [0x8e, 0xad, 0xe8, 0x01];

/// The fixed-size prefix of an RPM header.
struct RpmIndexHeader {
    num_entries: u32,
    data_len: u32,
}

impl RpmIndexHeader {
    /// On-disk length of the whole header (index header + entries + store).
    fn total_len(&self) -> usize {
        self.store_offset() + self.data_len as usize
    }

    /// Byte offset of the data store, relative to the header start.
    fn store_offset(&self) -> usize {
        RPM_INDEX_HEADER_LEN + self.num_entries as usize * RPM_INDEX_ENTRY_LEN
    }
}

fn parse_rpm_index_header(slice: &[u8]) -> Result<RpmIndexHeader> {
    if slice.len() < RPM_INDEX_HEADER_LEN {
        bail!("truncated rpm header (need {RPM_INDEX_HEADER_LEN} bytes)");
    }
    if slice[..4] != RPM_HEADER_MAGIC {
        bail!("unexpected rpm header magic");
    }
    Ok(RpmIndexHeader {
        num_entries: u32::from_be_bytes(slice[8..12].try_into().expect("4 bytes")),
        data_len: u32::from_be_bytes(slice[12..16].try_into().expect("4 bytes")),
    })
}

/// Build the `%lang` array in RPM header order (parallel to BASENAMES and
/// DIRNAMES), taking each entry's language from the `file_info` overrides.
/// Returns `None` when no installed file sets a language.
fn collect_file_langs(file_meta: &FileMetaMap, pkg: &rpm::Package) -> Result<Option<Vec<String>>> {
    let entries = pkg
        .metadata
        .get_file_entries()
        .context("failed to read rpm file entries for file_info.lang")?;
    let langs: Vec<String> = entries
        .iter()
        .map(|entry| {
            file_meta
                .get(&entry.path.to_string_lossy().to_string())
                .and_then(|m| m.lang.clone())
                .unwrap_or_default()
        })
        .collect();
    Ok(langs.iter().any(|l| !l.is_empty()).then_some(langs))
}

/// Rewrite a serialized `.rpm`'s main header to carry `langs`, returning the
/// new file bytes plus the SHA256 (hex) of the rewritten header.
fn inject_file_langs(bytes: Vec<u8>, langs: &[String]) -> Result<(Vec<u8>, String)> {
    if bytes.len() < RPM_LEAD_LEN + RPM_INDEX_HEADER_LEN {
        bail!("rpm file is too short to hold a header");
    }

    // Signature header: header + entries + store + 8-byte alignment padding.
    let sig = parse_rpm_index_header(&bytes[RPM_LEAD_LEN..])?;
    let sig_total = sig.total_len() + (8 - (sig.data_len as usize % 8)) % 8;
    let main_start = RPM_LEAD_LEN + sig_total;

    if main_start + RPM_INDEX_HEADER_LEN > bytes.len() {
        bail!("rpm file is truncated before the main header");
    }
    let main = parse_rpm_index_header(&bytes[main_start..])?;
    let main_len = main.total_len();
    if main_start + main_len > bytes.len() {
        bail!("rpm main header extends past the end of the file");
    }

    let mut header = bytes[main_start..main_start + main_len].to_vec();
    let store_offset = main.store_offset();
    let store = header[store_offset..].to_vec();

    // Locate the FILELANGS entry and the header region descriptor
    // (`HEADERIMMUTABLE`). The `rpm` crate appends the descriptor as the final
    // 16 bytes of the store; the language strings are inserted before it and
    // the descriptor is re-appended, so it stays the store's final entry.
    let mut filelangs: Option<usize> = None;
    let mut region: Option<usize> = None;
    for i in 0..main.num_entries as usize {
        let entry = RPM_INDEX_HEADER_LEN + i * RPM_INDEX_ENTRY_LEN;
        let tag = u32::from_be_bytes(header[entry..entry + 4].try_into().expect("4 bytes"));
        if tag == RPMTAG_FILELANGS {
            filelangs = Some(entry);
        } else if tag == RPMTAG_HEADERIMMUTABLE {
            region = Some(entry);
        }
    }
    let filelangs = filelangs.context("rpm main header has no RPMTAG_FILELANGS entry")?;
    let region = region.context("rpm main header has no region descriptor")?;

    let descriptor_offset =
        i32::from_be_bytes(header[region + 8..region + 12].try_into().expect("4 bytes"));
    let descriptor_offset =
        usize::try_from(descriptor_offset).context("rpm region descriptor offset is negative")?;
    let descriptor_end = descriptor_offset
        .checked_add(RPM_REGION_DESCRIPTOR_LEN)
        .filter(|end| *end <= store.len())
        .context("rpm region descriptor runs past the store")?;
    if descriptor_end != store.len() {
        bail!("rpm region descriptor is not the last entry in the header store");
    }

    let mut new_store = store[..descriptor_offset].to_vec();
    let langs_offset = i32::try_from(new_store.len()).context("rpm header store too large")?;
    for lang in langs {
        new_store.extend_from_slice(lang.as_bytes());
        new_store.push(0);
    }
    let new_descriptor_offset =
        i32::try_from(new_store.len()).context("rpm header store too large")?;
    new_store.extend_from_slice(&store[descriptor_offset..descriptor_end]);

    header[filelangs + 8..filelangs + 12].copy_from_slice(&langs_offset.to_be_bytes());
    header[filelangs + 12..filelangs + 16].copy_from_slice(&(langs.len() as u32).to_be_bytes());
    header[region + 8..region + 12].copy_from_slice(&new_descriptor_offset.to_be_bytes());

    // Refresh the data-section size, then splice the new store back in.
    let new_len = u32::try_from(new_store.len()).context("rpm header store too large")?;
    header[12..16].copy_from_slice(&new_len.to_be_bytes());
    header.truncate(store_offset);
    header.extend_from_slice(&new_store);

    let header_sha256 = hex::encode(sha2::Sha256::digest(&header));

    let mut out = Vec::with_capacity(bytes.len() + header.len());
    out.extend_from_slice(&bytes[..main_start]);
    out.extend_from_slice(&header);
    out.extend_from_slice(&bytes[main_start + main_len..]);
    Ok((out, header_sha256))
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
