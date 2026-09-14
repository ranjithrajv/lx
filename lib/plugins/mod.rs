// SPDX-License-Identifier: GPL-3.0-or-later

//! Packager architecture for `lx`.
//!
//! Each package format (`.deb`, `.rpm`, …) is a plugin implementing the
//! [`Packager`] trait. The build pipeline is format-agnostic: it resolves
//! assets, stages the install tree via shared helpers, then delegates the
//! actual archive creation to the selected plugin.
//!
//! Registration is static and explicit — no dynamic loading — so adding a new
//! format is just implementing `Packager` and registering it in
//! [`registry`] / [`all_packagers`].

pub mod apk;
pub mod arch;
pub mod artifact;
pub mod build_system;
pub mod deb;
pub mod depmap;
pub mod forge;
pub mod ipk;
pub mod msix;
pub mod osxpkg;
pub mod package_index;
pub mod plugin;
pub mod registry;
pub mod rpm;
pub mod signer;

use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::config::{ContentEntry, PackageConfig};
use crate::filemeta::{installed_path, FileMeta, FileMetaMap, RpmFileKind};
use crate::plugins::plugin::{Plugin, PluginSet};
use lx_lib::github::RepoLicense;

/// Context passed to a plugin's build method. Contains everything the plugin
/// needs to render its control/spec metadata and archive the staged tree.
pub struct BuildContext<'a> {
    /// The full package config (package_name, github_repo, epoch, etc.).
    pub cfg: &'a PackageConfig,
    /// Resolved job (dist, arch, asset, tag, published_at).
    pub job: &'a crate::build::ResolvedJob,
    /// Directory containing the extracted binary tree to stage (binary_dir).
    pub binary_dir: &'a Path,
    /// Staging root that the plugin should populate (e.g. `root/usr/bin/...`).
    /// Already created; plugin stages files under it then archives it.
    pub staging_root: &'a Path,
    /// Detected upstream license, if any.
    pub license: Option<&'a RepoLicense>,
    /// Debian-style version string (upstream prefix stripped).
    pub debian_version: &'a str,
    /// Build revision (e.g. "1").
    pub build_version: &'a str,
    /// Reproducible mtime (SOURCE_DATE_EPOCH or published_at).
    pub mtime: i64,
    /// Signing key file for formats that embed signatures natively
    /// (rpm), and for deb when `sign_method` is `"debsign"`. `None` when
    /// signing is disabled. deb `detach` signs post-build instead
    /// (detached `.sig`).
    pub sign_key: Option<&'a Path>,
    /// Optional gpg `--local-user` key id / fingerprint (deb signing).
    pub sign_key_id: &'a str,
    /// Passphrase for the embedded-signature key (rpm / debsign),
    /// resolved from env.
    pub sign_passphrase: Option<&'a str>,
    /// Deb signing method: `"detach"` (post-build `.sig`) or `"debsign"`
    /// (embedded `_gpgorigin`). Ignored by rpm/arch.
    pub sign_method: &'a str,
    /// Binary dependencies detected via ELF analysis. Merged into Depends:
    /// by the format plugins (deb only; rpm/arch handle deps differently).
    pub detected_deps: Vec<String>,
}

// ---------------------------------------------------------------------------
// Shared helpers for packager plugins (DRY: deb/rpm/arch all used to duplicate these)
// ---------------------------------------------------------------------------

/// Resolve the homepage URL from config, accounting for each provider's
/// host conventions and self-hosted overrides.
pub fn resolve_homepage(cfg: &PackageConfig) -> String {
    use lx_lib::constants::*;
    match cfg.effective_forge_source().as_str() {
        "gitlab" => homepage_for_gitlab(&cfg.github_repo, cfg.gitlab_host.as_deref().unwrap_or("")),
        "gitea" => homepage_for_gitea(&cfg.github_repo, cfg.gitea_host.as_deref().unwrap_or("")),
        "forgejo" => {
            homepage_for_forgejo(&cfg.github_repo, cfg.forgejo_host.as_deref().unwrap_or(""))
        }
        "bitbucket" => homepage_for_bitbucket(&cfg.github_repo),
        "gerrit" => homepage_for_gerrit(&cfg.github_repo, cfg.gerrit_host.as_deref().unwrap_or("")),
        "gitee" => homepage_for_gitee(&cfg.github_repo, cfg.gitee_host.as_deref().unwrap_or("")),
        "sourceforge" => homepage_for_sourceforge(&cfg.github_repo),
        _ => homepage_for_github(&cfg.github_repo),
    }
}

/// Compute `{build_version}+{dist}` formatted for non-debian release strings
/// (the `+` is replaced with `.` per RPM/Arch conventions).
pub fn format_release(build_version: &str, dist: &str) -> String {
    format!("{build_version}+{dist}").replace('+', ".")
}

/// Create the sibling output directory `__out` next to the staging root.
/// The archive builders tar/zst the whole staging tree, so an in-tree out
/// dir would package the artifact into itself.
pub fn output_dir(staging_root: &Path) -> anyhow::Result<PathBuf> {
    let dir = staging_root
        .parent()
        .context("staging root has no parent")?
        .join("__out");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A package-format plugin.
///
/// Implementors are stateless; they are registered once in [`registry`].
pub trait Packager: Plugin {
    /// File extension without dot (e.g. `"deb"`, `"rpm"`).
    fn file_extension(&self) -> &'static str;

    /// Default distributions when `debian_distributions` is empty and the
    /// plugin is selected. For `deb` this is Debian suites; for `rpm` this
    /// is RPM-based distros.
    // Exercised by `default_distributions_differ_by_format` below; not yet
    /// called by the live distribution-resolution path, which still reads
    // the DEFAULT_*_DISTRIBUTIONS constants directly (see config.rs).
    #[allow(dead_code)]
    fn default_distributions(&self) -> &'static [&'static str];

    /// Whether an architecture is supported for a given distribution.
    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool;

    /// Build a single package archive.
    ///
    /// The plugin stages the install tree under `ctx.staging_root` (using
    /// shared helpers like [`stage_install_tree`]) and then creates the
    /// archive at the returned `PathBuf` (typically inside a temp dir).
    fn build(&self, ctx: &BuildContext) -> Result<PathBuf>;
}

/// All known plugins, in registration order.
pub fn all_packagers() -> Vec<Box<dyn Packager>> {
    vec![
        Box::new(deb::DebPackager),
        Box::new(rpm::RpmPackager),
        Box::new(arch::ArchPackager),
        Box::new(apk::ApkPackager),
        Box::new(ipk::IpkPackager),
        Box::new(msix::MsixPackager),
        Box::new(osxpkg::OsxPkgPackager),
    ]
}

/// Look up a plugin by name (case-insensitive, aliases resolved: `pkg` →
/// `osxpkg`). Returns `None` for unknown.
pub fn get_packager(name: &str) -> Option<Box<dyn Packager>> {
    PluginSet::new(all_packagers()).take(&crate::config::canonical_format(name))
}

/// Available plugin names for error messages / help text.
pub fn packager_names() -> Vec<&'static str> {
    PluginSet::new(all_packagers()).names()
}

/// Expand a user-supplied `--format` value into the concrete formats to
/// build. `all` expands to every registered packager (deb, rpm, arch, apk,
/// ipk); a comma-separated list is split and trimmed; a single format
/// returns `None` so the caller can pass it through unchanged (preserving
/// the existing validation and error messages).
pub fn expand_formats(value: &str) -> Option<Vec<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("all") {
        return Some(packager_names().into_iter().map(str::to_string).collect());
    }
    if v.contains(',') {
        let list: Vec<String> = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !list.is_empty() {
            return Some(list);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Shared staging helpers (format-agnostic, used by all plugins)
// ---------------------------------------------------------------------------

/// Stage the package's installed filesystem tree under `root`.
///
/// Mirrors the old `build.rs::stage_install_tree` — format-agnostic file
/// placement logic that every plugin reuses. Handles `bundle` vs flat mode,
/// `binary_rename`, and ancillary file (man page / license) staging.
///
/// When `cfg.prefix` is non-empty (set via `--prefix` for `--from-dir`/
/// `--from-file` builds), all files are staged flat under `<root><prefix>`
/// instead of the default `/usr/bin` layout — a simple "put these files
/// here" mode for fpm-style "you supply files" builds.
pub fn stage_install_tree(
    cfg: &PackageConfig,
    binary_dir: &Path,
    root: &Path,
    mtime: i64,
) -> anyhow::Result<()> {
    // Custom-prefix mode (fpm-style): dump files at the given absolute path
    // inside the package, with no ELF detection or ancillary staging.
    if !cfg.prefix.is_empty() {
        let dest_dir = root.join(cfg.prefix.trim_start_matches('/'));
        std::fs::create_dir_all(&dest_dir)?;
        copy_dir_recursive(binary_dir, &dest_dir)?;
        return Ok(());
    }

    let usr_bin = root.join("usr").join("bin");
    std::fs::create_dir_all(&usr_bin)?;

    if cfg.bundle {
        let lib_dir = root.join("usr").join("lib").join(&cfg.package_name);
        copy_dir_recursive(binary_dir, &lib_dir)?;

        let bin_subdir = lib_dir.join("bin");
        if bin_subdir.is_dir() {
            symlink_elf_executables(
                &bin_subdir,
                &usr_bin,
                &format!("/usr/lib/{}/bin", cfg.package_name),
            )?;
        }
        symlink_elf_executables(
            &lib_dir,
            &usr_bin,
            &format!("/usr/lib/{}", cfg.package_name),
        )?;
    } else {
        let umask = cfg.effective_umask();
        for entry in std::fs::read_dir(binary_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            if is_elf(&path)? {
                let dest = usr_bin.join(entry.file_name());
                std::fs::copy(&path, &dest)?;
                make_executable(&dest)?;
                apply_umask(&dest, umask)?;
            } else {
                stage_ancillary_file(&path, root, &cfg.package_name, mtime, umask)?;
            }
        }
    }

    if !cfg.binary_rename.is_empty() {
        apply_binary_rename(&usr_bin, &cfg.binary_rename)?;
    }

    if std::fs::read_dir(&usr_bin)?.next().is_none() {
        anyhow::bail!(
            "no executables landed in /usr/bin ({}) - check binary_path/bundle config",
            if cfg.bundle {
                "bundle=true"
            } else {
                "flat mode"
            }
        );
    }

    // Make binaries relocatable (RPATH = $ORIGIN/../lib).
    super::relocatable::make_relocatable(&usr_bin)?;

    // Detect binary dependencies via ELF analysis + distro package lookup.
    let _detected_deps = super::bindep::detect_binary_deps(root).unwrap_or_default();

    Ok(())
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(&target, &dest_path)?;
        } else if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

fn symlink_elf_executables(src_dir: &Path, usr_bin: &Path, abs_prefix: &str) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file() && is_elf(&path)? {
            make_executable(&path)?;
            let name = entry.file_name();
            let target = format!("{abs_prefix}/{}", name.to_string_lossy());
            std::os::unix::fs::symlink(&target, usr_bin.join(&name))?;
        }
    }
    Ok(())
}

pub(crate) fn is_elf(path: &Path) -> anyhow::Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(&magic == b"\x7fELF")
}

fn make_executable(path: &Path) -> anyhow::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | lx_lib::constants::EXEC_MODE_MASK);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// Apply umask to a file's permissions: `mode = mode & !umask`.
/// When `umask` is `None`, the file's mode is left unchanged.
fn apply_umask(path: &Path, umask: Option<u32>) -> anyhow::Result<()> {
    let Some(mask) = umask else {
        return Ok(());
    };
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() & !mask);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// Returns true if `s` contains glob meta-characters (`*`, `?`, `[`).
fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

fn stage_ancillary_file(
    path: &Path,
    root: &Path,
    pkg_name: &str,
    mtime: i64,
    umask: Option<u32>,
) -> anyhow::Result<()> {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return Ok(()),
    };
    if let Some(section) = man_section(name) {
        let man_dir = root.join("usr/share/man").join(format!("man{section}"));
        std::fs::create_dir_all(&man_dir)?;
        if name.ends_with(".gz") {
            let dest = man_dir.join(name);
            std::fs::copy(path, &dest)?;
            apply_umask(&dest, umask)?;
        } else {
            let dest = man_dir.join(format!("{name}.gz"));
            gzip_file_to(path, &dest, mtime)?;
            apply_umask(&dest, umask)?;
        }
    } else if is_license_like(name) {
        let doc_dir = root.join("usr/share/doc").join(pkg_name);
        std::fs::create_dir_all(&doc_dir)?;
        let dest = doc_dir.join(name);
        std::fs::copy(path, &dest)?;
        apply_umask(&dest, umask)?;
    }
    Ok(())
}

pub fn man_section(name: &str) -> Option<u8> {
    let stem = name.strip_suffix(".gz").unwrap_or(name);
    let ext = Path::new(stem).extension()?.to_str()?;
    if ext.len() == 1 {
        let c = ext.as_bytes()[0];
        if c.is_ascii_digit() && c != b'0' {
            return Some(c - b'0');
        }
    }
    None
}

pub fn is_license_like(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("LICENSE") || upper.starts_with("COPYING") || upper.starts_with("NOTICE")
}

fn gzip_file_to(src: &Path, dest: &Path, mtime: i64) -> anyhow::Result<()> {
    use std::io::Write;
    let data = std::fs::read(src)?;
    let mut out = std::fs::File::create(dest)?;
    // Same deterministic treatment as the tar members (fixed mtime header)
    // via debarchive's shared helper.
    let gz = lx_lib::debarchive::deterministic_gzip_bytes(&data, mtime, 9)?;
    out.write_all(&gz)?;
    Ok(())
}

fn apply_binary_rename(usr_bin: &Path, rename: &str) -> anyhow::Result<()> {
    let files: Vec<_> = std::fs::read_dir(usr_bin)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .collect();
    if files.len() == 1 {
        std::fs::rename(files[0].path(), usr_bin.join(rename))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// contents / scripts / conffiles (nfpm-parity staging overlays)
// ---------------------------------------------------------------------------

/// Stage `contents:` entries into the staged tree, after the release
/// payload. `src` paths resolve against the current working directory
/// (the build environment); `dst` is an absolute installed path under
/// `root`.
///
/// `format` is the active package format (`deb` / `rpm` / `arch`). Entries
/// with a non-empty `packager` field are skipped unless it matches
/// (case-insensitive), matching nfpm.
///
/// A `contents:` entry staged as a config file. `noreplace` is true for
/// `config|noreplace` (rpm `%config(noreplace)`, pacman backup-preserved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedConfig {
    /// Absolute installed path (e.g. `/etc/foo.conf`).
    pub path: String,
    pub noreplace: bool,
}

/// Returns the absolute installed paths registered as deb conffiles —
/// entries whose type is `config`, `config|noreplace`, or
/// `config|missingok`. Other formats ignore the return value.
pub fn apply_contents(
    cfg: &PackageConfig,
    root: &Path,
    format: &str,
) -> anyhow::Result<Vec<String>> {
    Ok(apply_contents_with_config(cfg, root, format)?
        .into_iter()
        .map(|c| c.path)
        .collect())
}

/// Like [`apply_contents`] but keeps the config kind so rpm can mark
/// `%config(noreplace)` and Arch can build its `backup` list.
///
/// Drops the per-file metadata map; callers that need it (every archive
/// writer, for `file_info` / `disown_subtree`) use [`apply_contents_full`].
pub fn apply_contents_with_config(
    cfg: &PackageConfig,
    root: &Path,
    format: &str,
) -> anyhow::Result<Vec<StagedConfig>> {
    Ok(apply_contents_full(cfg, root, format)?.0)
}

/// Stage every applicable `contents:` entry and return the registered config
/// files plus the per-file metadata overrides (`file_info`, `disown_subtree`)
/// keyed by installed path for the archive writers.
pub fn apply_contents_full(
    cfg: &PackageConfig,
    root: &Path,
    format: &str,
) -> anyhow::Result<(Vec<StagedConfig>, FileMetaMap)> {
    let format = format.trim().to_ascii_lowercase();
    let umask = cfg.effective_umask();
    let mut configs = Vec::new();
    let mut meta = FileMetaMap::new();
    for entry in &cfg.contents {
        let packager = entry.packager.trim().to_ascii_lowercase();
        if !packager.is_empty() && packager != format {
            continue;
        }
        // nfpm's `expand: true` expands `$VAR` / `${VAR}` in src and dst.
        let entry = if entry.expand {
            let mut e = entry.clone();
            e.src = os_expand(&e.src);
            e.dst = os_expand(&e.dst);
            e
        } else {
            entry.clone()
        };
        let dst_abs = safe_join(root, &entry.dst)?;
        let staged: Vec<PathBuf> = match entry.kind.as_str() {
            "" | "file" => stage_contents_entry(
                Path::new(&entry.src),
                &dst_abs,
                &entry.kind,
                umask,
                cfg.disable_globbing,
                false,
            )?,
            "config" | "config|noreplace" | "config|missingok" => {
                configs.push(StagedConfig {
                    path: entry.dst.clone(),
                    noreplace: entry.kind == "config|noreplace",
                });
                stage_contents_entry(
                    Path::new(&entry.src),
                    &dst_abs,
                    &entry.kind,
                    umask,
                    cfg.disable_globbing,
                    false,
                )?
            }
            "tree" => stage_contents_entry(
                Path::new(&entry.src),
                &dst_abs,
                &entry.kind,
                umask,
                cfg.disable_globbing,
                true,
            )?,
            // nfpm's "config tree" types: copy the tree, then register every
            // regular file as a config file.
            "config|tree" | "config|noreplace|tree" | "config|missingok|tree" => {
                let staged = stage_contents_entry(
                    Path::new(&entry.src),
                    &dst_abs,
                    "tree",
                    umask,
                    cfg.disable_globbing,
                    true,
                )?;
                let noreplace = entry.kind == "config|noreplace|tree";
                for p in &staged {
                    for f in walk_paths(p)? {
                        if f.is_file() {
                            configs.push(StagedConfig {
                                path: installed_for(root, &f)?,
                                noreplace,
                            });
                        }
                    }
                }
                staged
            }
            // RPM doc/license/readme classification; a plain file elsewhere.
            "doc" | "license" | "licence" | "readme" => stage_contents_entry(
                Path::new(&entry.src),
                &dst_abs,
                &entry.kind,
                umask,
                cfg.disable_globbing,
                false,
            )?,
            // nfpm symlink semantics: both src and dst are paths *inside*
            // the package; nothing is read from the build environment.
            "symlink" => {
                if let Some(parent) = dst_abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let _ = std::fs::remove_file(&dst_abs);
                #[cfg(unix)]
                std::os::unix::fs::symlink(&entry.src, &dst_abs)?;
                vec![dst_abs.clone()]
            }
            "dir" => {
                std::fs::create_dir_all(&dst_abs)?;
                vec![dst_abs.clone()]
            }
            // RPM-only directive; a no-op for deb/arch (matches nfpm,
            // which ignores ghost files for non-rpm packagers).
            "ghost" => Vec::new(),
            other => anyhow::bail!(
                "unsupported contents type '{other}' (validate() should have caught this)"
            ),
        };
        record_entry_meta(&mut meta, root, &entry, &staged)?;
    }
    configs.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((configs, meta))
}

/// Record the `file_info` / `disown_subtree` overrides for one entry's staged
/// paths. `tree` and `dir` entries apply to their whole subtree; a symlink
/// carries ownership but never a mode (matching nfpm).
fn record_entry_meta(
    meta: &mut FileMetaMap,
    root: &Path,
    entry: &ContentEntry,
    staged: &[PathBuf],
) -> anyhow::Result<()> {
    if staged.is_empty() {
        return Ok(());
    }
    let base = FileMeta {
        mode: entry.file_info.parsed_mode()?,
        owner: non_empty(&entry.file_info.owner),
        group: non_empty(&entry.file_info.group),
        mtime: entry.file_info.parsed_mtime()?,
        lang: non_empty(&entry.file_info.lang),
        disown: false,
        rpm_kind: match entry.kind.as_str() {
            "doc" => Some(RpmFileKind::Doc),
            "license" | "licence" => Some(RpmFileKind::License),
            "readme" => Some(RpmFileKind::Readme),
            _ => None,
        },
    };
    let is_symlink = entry.kind == "symlink";
    let recurse = matches!(
        entry.kind.as_str(),
        "tree" | "dir" | "config|tree" | "config|noreplace|tree" | "config|missingok|tree"
    );
    for path in staged {
        if recurse || path.is_dir() {
            for p in walk_paths(path)? {
                let installed = installed_for(root, &p)?;
                let mut m = base.clone();
                if p.is_dir()
                    && !entry.disown_subtree.is_empty()
                    && matches_any(&installed, &entry.disown_subtree)
                {
                    m.disown = true;
                }
                if !m.is_empty() {
                    meta.insert(installed, m);
                }
            }
        } else {
            let mut m = base.clone();
            if is_symlink {
                m.mode = None;
            }
            if !m.is_empty() {
                meta.insert(installed_for(root, path)?, m);
            }
        }
    }
    Ok(())
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn installed_for(root: &Path, path: &Path) -> anyhow::Result<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("staged path '{}' escaped the root", path.display()))?;
    Ok(installed_path(&rel.to_string_lossy()))
}

/// `path` plus every descendant, for applying a tree-wide `file_info`.
fn walk_paths(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = vec![path.to_path_buf()];
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            out.extend(walk_paths(&entry.path())?);
        }
    }
    Ok(out)
}

fn matches_any(installed: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        let p = if p.starts_with('/') {
            p.clone()
        } else {
            format!("/{p}")
        };
        installed == p
            || glob::Pattern::new(&p)
                .map(|pat| pat.matches(installed))
                .unwrap_or(false)
    })
}

/// Go `os.Expand` semantics: `$NAME` / `${NAME}` become the environment
/// value, or the empty string when unset. Used by `contents[].expand: true`.
fn os_expand(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('{') => {
                chars.next();
                let mut name = String::new();
                let mut closed = false;
                for c2 in chars.by_ref() {
                    if c2 == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c2);
                }
                if closed {
                    out.push_str(&std::env::var(&name).unwrap_or_default());
                } else {
                    out.push_str("${");
                    out.push_str(&name);
                }
            }
            Some(c2) if c2.is_ascii_alphanumeric() || c2 == '_' => {
                let mut name = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_alphanumeric() || next == '_' {
                        name.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push_str(&std::env::var(&name).unwrap_or_default());
            }
            _ => out.push('$'),
        }
    }
    out
}

/// Stage a single contents entry, expanding glob patterns when applicable.
///
/// For `file`/`config*` types: if `src` contains glob characters and
/// `disable_globbing` is false, expand the pattern and stage each matching
/// file into `dst` (treated as a directory). Otherwise, stage `src` directly
/// to `dst`.
///
/// For `tree` type: if `src` contains glob characters and `disable_globbing`
/// is false, expand the pattern and copy each matching directory as a tree
/// into `dst`. Otherwise, copy `src` recursively to `dst`.
fn stage_contents_entry(
    src: &Path,
    dst: &Path,
    kind: &str,
    umask: Option<u32>,
    disable_globbing: bool,
    is_tree: bool,
) -> anyhow::Result<Vec<PathBuf>> {
    let src_str = src.to_string_lossy();
    let should_glob = !disable_globbing && is_glob_pattern(&src_str);

    if !should_glob {
        // Non-glob path: original behavior.
        if is_tree {
            copy_dir_recursive(src, dst)?;
        } else {
            stage_file(src, dst, umask)?;
        }
        return Ok(vec![dst.to_path_buf()]);
    }

    // Glob expansion path.
    let is_dir_type = is_tree || kind == "dir";
    let matches: Vec<_> = glob::glob(&src_str)
        .with_context(|| format!("invalid glob pattern '{src_str}'"))?
        .filter_map(|r| r.ok())
        .collect();

    if matches.is_empty() {
        anyhow::bail!("glob pattern '{src_str}' matched no files");
    }

    let mut staged = Vec::new();
    if is_dir_type {
        // For tree type: copy each matching directory as a subtree.
        std::fs::create_dir_all(dst)?;
        for matched in matches {
            if matched.is_dir() {
                let name = matched.file_name().ok_or_else(|| {
                    anyhow::anyhow!("glob match '{}' has no file name", matched.display())
                })?;
                let dest = dst.join(name);
                copy_dir_recursive(&matched, &dest)?;
                staged.push(dest);
            }
        }
    } else {
        // For file/config type: stage each matching file into dst (as directory).
        std::fs::create_dir_all(dst)?;
        for matched in matches {
            if matched.is_file() {
                let name = matched.file_name().ok_or_else(|| {
                    anyhow::anyhow!("glob match '{}' has no file name", matched.display())
                })?;
                let dest = dst.join(name);
                stage_file(&matched, &dest, umask)?;
                staged.push(dest);
            }
        }
    }
    Ok(staged)
}

/// Read the configured maintainer scripts from the build environment and
/// render them as deb control members (`preinst`/`postinst`/`prerm`/
/// `postrm`, mode 0755). Empty when none are configured.
///
/// When `cfg.template_scripts` is true, each script file is processed
/// through the template engine before being staged, replacing `<%= key %>`
/// expressions with package values from the build context.
pub fn maintainer_script_members(
    ctx: &BuildContext,
) -> anyhow::Result<Vec<lx_lib::debarchive::ControlMember>> {
    let cfg = ctx.cfg;
    let pairs = [
        ("preinst", cfg.scripts.preinstall.trim()),
        ("postinst", cfg.scripts.postinstall.trim()),
        ("prerm", cfg.scripts.preremove.trim()),
        ("postrm", cfg.scripts.postremove.trim()),
        ("preupgrade", cfg.scripts.preupgrade_script.trim()),
        ("postupgrade", cfg.scripts.postupgrade_script.trim()),
    ];
    let template_context = if cfg.template_scripts {
        Some(lx_lib::templating::build_context(cfg, ctx.job))
    } else {
        None
    };
    let mut members = Vec::new();
    for (name, path) in pairs {
        if path.is_empty() {
            continue;
        }
        let content = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("failed to read {} script '{}': {e}", name, path))?;
        let content = if let Some(ref tmpl_ctx) = template_context {
            let script_text = String::from_utf8_lossy(&content);
            let rendered = lx_lib::templating::render_template(&script_text, tmpl_ctx);
            rendered.into_bytes()
        } else {
            content
        };
        members.push(lx_lib::debarchive::ControlMember {
            name: name.to_string(),
            content,
            mode: 0o755,
        });
    }
    Ok(members)
}

/// Read a maintainer-script file from the build environment, applying
/// `template_scripts` rendering when enabled. Returns `None` for an empty
/// path so callers can skip unset hooks.
///
/// Shared by the rpm and Arch plugins; the deb plugin's
/// [`maintainer_script_members`] does the same inline because it also has to
/// emit the members as a multi-file control set.
pub fn render_script_body(
    cfg: &PackageConfig,
    job: &crate::build::ResolvedJob,
    path: &str,
) -> anyhow::Result<Option<String>> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read script '{path}': {e}"))?;
    let content = if cfg.template_scripts {
        let ctx = lx_lib::templating::build_context(cfg, job);
        lx_lib::templating::render_template(&content, &ctx)
    } else {
        content
    };
    Ok(Some(content))
}

/// Read the deb-specific control members from the build environment: debconf
/// `templates` (mode 0644) and `config` (mode 0755), `rules` (mode 0755),
/// and the `triggers` file (mode 0644) listing `interest`/`activate`
/// lines. Mirrors nfpm's `deb.scripts.rules`, `deb.scripts.templates`,
/// `deb.scripts.config`, and `deb.triggers.{interest,activate}`.
///
/// Returns an empty vec when none of these are configured, so callers can
/// skip this call entirely for non-deb formats.
pub fn deb_extra_members(
    cfg: &PackageConfig,
) -> anyhow::Result<Vec<lx_lib::debarchive::ControlMember>> {
    let deb = &cfg.deb;
    let mut members = Vec::new();

    // rules (mode 0755)
    if !deb.rules.trim().is_empty() {
        let content = std::fs::read(&deb.rules).map_err(|e| {
            anyhow::anyhow!(
                "failed to read deb rules script '{}': {e}",
                deb.rules.trim()
            )
        })?;
        members.push(lx_lib::debarchive::ControlMember {
            name: "rules".to_string(),
            content,
            mode: 0o755,
        });
    }

    // debconf templates (mode 0644)
    if !deb.templates.trim().is_empty() {
        let content = std::fs::read(&deb.templates).map_err(|e| {
            anyhow::anyhow!(
                "failed to read debconf templates '{}': {e}",
                deb.templates.trim()
            )
        })?;
        members.push(lx_lib::debarchive::ControlMember {
            name: "templates".to_string(),
            content,
            mode: 0o644,
        });
    }

    // debconf config (mode 0755)
    if !deb.config.trim().is_empty() {
        let content = std::fs::read(&deb.config).map_err(|e| {
            anyhow::anyhow!(
                "failed to read debconf config script '{}': {e}",
                deb.config.trim()
            )
        })?;
        members.push(lx_lib::debarchive::ControlMember {
            name: "config".to_string(),
            content,
            mode: 0o755,
        });
    }

    // triggers (mode 0644)
    let mut trigger_lines: Vec<String> = Vec::new();
    for t in &deb.triggers_interest {
        trigger_lines.push(format!("interest {}", t.trim()));
    }
    for t in &deb.triggers_interest_await {
        trigger_lines.push(format!("interest_await {}", t.trim()));
    }
    for t in &deb.triggers_interest_noawait {
        trigger_lines.push(format!("interest_noawait {}", t.trim()));
    }
    for t in &deb.triggers_activate {
        trigger_lines.push(format!("activate {}", t.trim()));
    }
    for t in &deb.triggers_activate_await {
        trigger_lines.push(format!("activate_await {}", t.trim()));
    }
    for t in &deb.triggers_activate_noawait {
        trigger_lines.push(format!("activate_noawait {}", t.trim()));
    }
    if !trigger_lines.is_empty() {
        trigger_lines.push(String::new()); // trailing newline
        members.push(lx_lib::debarchive::ControlMember {
            name: "triggers".to_string(),
            content: trigger_lines.join("\n").into_bytes(),
            mode: 0o644,
        });
    }

    Ok(members)
}

fn stage_file(src: &Path, dst: &Path, umask: Option<u32>) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)
        .map(|_| ())
        .with_context(|| format!("failed to copy '{}' to '{}'", src.display(), dst.display()))?;
    apply_umask(dst, umask)?;
    Ok(())
}

/// Join an absolute installed path onto the staging root, rejecting any
/// attempt to escape it (`..` components).
pub fn safe_join(root: &Path, installed_abs: &str) -> anyhow::Result<std::path::PathBuf> {
    if !installed_abs.starts_with('/') {
        anyhow::bail!("contents dst must be absolute: '{installed_abs}'");
    }
    let rel = installed_abs.trim_start_matches('/');
    let joined = root.join(rel);
    if joined.components().any(|c| c.as_os_str() == "..") {
        anyhow::bail!("contents dst escapes the package root: '{installed_abs}'");
    }
    Ok(joined)
}

/// The extended-description continuation line shared by every rendered
/// control file (binary `DEBIAN/control`, source `debian/control`). One
/// constant so the wording can't drift between renderers again — it had
/// already split into "upstream GitHub release" vs "the upstream release"
/// variants before this was centralized.
pub(crate) const PACKAGED_FROM_LINE: &str = " Packaged from the upstream release for Debian.";

/// Render sorted extra control fields (skipping core fields already
/// rendered explicitly). Shared by the deb plugin's binary control file
/// and the source package's `debian/control`.
pub(crate) fn render_extra_fields(fields: &std::collections::HashMap<String, String>) -> String {
    if fields.is_empty() {
        return String::new();
    }
    let mut keys: Vec<_> = fields.keys().collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        let v = &fields[k];
        if k.trim().is_empty() || v.trim().is_empty() {
            continue;
        }
        // Avoid duplicating core fields if user accidentally sets them via fields.
        let lower = k.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "package"
                | "version"
                | "architecture"
                | "maintainer"
                | "description"
                | "section"
                | "priority"
                | "homepage"
                | "depends"
                | "recommends"
                | "suggests"
                | "conflicts"
                | "replaces"
                | "provides"
                | "breaks"
                | "pre-depends"
                | "pre_depends"
                | "predepends"
                | "source"
                | "standards-version"
                | "build-depends"
        ) {
            continue;
        }
        out.push_str(&format!("{}: {}\n", k.trim(), v.trim()));
    }
    out
}
