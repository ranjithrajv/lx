// SPDX-License-Identifier: GPL-3.0-or-later

//! Build an Arch Linux `.pkg.tar.zst` pacman package entirely in-process.
//!
//! Mirrors `debarchive.rs` / `rpmarchive.rs`: no `makepkg`, fully in-process.
//! An Arch package is a `tar` (optionally `zstd`-compressed) containing
//! `.PKGINFO` + `.MTREE` + payload. pacman verifies it via `bsdtar`.

use anyhow::{Context, Result};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::filemeta::{self, FileMeta, FileMetaMap};

/// Package metadata fields shared by [`build`] and `render_pkginfo`.
#[derive(Debug, Clone, Copy)]
pub struct PackageMeta<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub release: &'a str,
    pub description: &'a str,
    pub url: &'a str,
    pub license: &'a str,
}

/// Fallback packager identity when the config doesn't supply one.
const DEFAULT_PACKAGER: &str = "lx <lx@latest-debs.org>";

/// Optional package-relation metadata for a pacman package. Entries are
/// already in pacman syntax (`name>=1.2`, `name: description`).
///
/// Kept separate from [`PackageMeta`] so existing callers that only need
/// the core metadata keep working unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct PackageRelations<'a> {
    /// `depend = ...` entries (hard runtime deps).
    pub depends: &'a [String],
    /// `optdepend = ...` entries (Debian Recommends/Suggests).
    pub optdepends: &'a [String],
    /// `conflict = ...` entries.
    pub conflicts: &'a [String],
    /// `provides = ...` entries.
    pub provides: &'a [String],
    /// `replaces = ...` entries.
    pub replaces: &'a [String],
    /// `backup = ...` entries — installed paths relative to `/`, no
    /// leading slash (config files pacman preserves on upgrade).
    pub backup: &'a [String],
}

/// Translate a Debian-style dependency relation string into pacman
/// dependency entries.
///
/// `"libc6 (>= 2.34), libssl3 | libssl1.1, foo:any"` becomes
/// `["libc6>=2.34", "libssl3", "foo"]`. Debian alternatives (`a | b`) keep
/// the first arm (pacman expresses alternatives via `provides`, not a
/// single relation); multiarch qualifiers (`foo:amd64`, `foo:any`) and
/// `[arch]` restrictions are dropped.
pub fn debian_relations_to_pacman(relation: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in relation.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        // Debian alternative: keep the first arm.
        let first = raw.split('|').next().unwrap_or(raw).trim();
        if first.is_empty() {
            continue;
        }
        // Split "name (>= 1.2)" into name + operator/version.
        let (name, constraint) = match first.split_once('(') {
            Some((name, ver)) => {
                let ver = ver.trim().trim_end_matches(')').trim();
                let (op, value) = split_constraint(ver);
                (name.trim(), Some((op, value)))
            }
            None => (first, None),
        };
        // Drop a Debian multiarch qualifier (`foo:any`) or arch restriction
        // (`foo [amd64]`).
        let name = name.split(':').next().unwrap_or(name);
        let name = name.split('[').next().unwrap_or(name).trim();
        if name.is_empty() {
            continue;
        }
        match constraint {
            Some((op, value)) if !value.is_empty() => {
                out.push(format!("{name}{op}{value}"));
            }
            _ => out.push(name.to_string()),
        }
    }
    out
}

/// Split a Debian version constraint (`>= 1.2`, `<< 2.0`, `= 1`) into a
/// pacman comparison operator and the bare version.
fn split_constraint(ver: &str) -> (&'static str, String) {
    let ver = ver.trim();
    for (deb, pac) in [
        (">=", ">="),
        ("<=", "<="),
        (">>", ">"),
        ("<<", "<"),
        ("==", "="),
        ("=", "="),
        (">", ">"),
        ("<", "<"),
    ] {
        if let Some(rest) = ver.strip_prefix(deb) {
            return (pac, rest.trim().replace(' ', ""));
        }
    }
    (">=", ver.replace(' ', ""))
}

/// Build a pacman package from a staged filesystem tree.
///
/// `root` is the staged tree (e.g. `root/usr/bin/foo` → `/usr/bin/foo`).
/// `arch_path` is the output `*.pkg.tar.zst`.
/// `install_script` is an optional `.INSTALL` file body (pre/post-upgrade
/// hooks); when non-empty it is added as the `.INSTALL` member.
pub fn build(
    root: &Path,
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    arch_path: &Path,
    install_script: Option<&str>,
) -> Result<()> {
    build_with_relations(
        root,
        meta,
        &PackageRelations::default(),
        None,
        None,
        arch,
        mtime,
        arch_path,
        install_script,
        &FileMetaMap::new(),
    )
}

/// Like [`build`] but also emits pacman relation/config metadata
/// (`depend`, `optdepend`, `conflict`, `provides`, `replaces`, `backup`),
/// an optional `packager` and an optional epoch (rendered into `pkgver` as
/// `epoch:version-release`, matching makepkg's `get_full_version`).
///
/// `file_meta` carries `contents[].file_info` / `disown_subtree` overrides.
#[allow(clippy::too_many_arguments)]
pub fn build_with_relations(
    root: &Path,
    meta: &PackageMeta,
    relations: &PackageRelations,
    packager: Option<&str>,
    epoch: Option<&str>,
    arch: &str,
    mtime: i64,
    arch_path: &Path,
    install_script: Option<&str>,
    file_meta: &FileMetaMap,
) -> Result<()> {
    let pacman_arch = to_pacman_arch(arch);
    let pkgver = match epoch.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => format!("{e}:{}-{}", meta.version, meta.release),
        None => format!("{}-{}", meta.version, meta.release),
    };

    // Calculate installed size by walking the tree.
    let installed_size = calc_installed_size(root)?;

    // Render .PKGINFO
    let pkginfo = render_pkginfo_with(
        meta,
        relations,
        packager,
        &pkgver,
        pacman_arch,
        mtime,
        installed_size,
    );

    // Build uncompressed tar in memory.
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);

        // .PKGINFO
        append_file_bytes(
            &mut builder,
            ".PKGINFO",
            pkginfo.as_bytes(),
            0o644,
            mtime,
            None,
        )?;
        // .MTREE will be generated after we know all payload entries,
        // but for determinism we need to include it now. Simplest: generate
        // MTREE from the same walk and add as regular file.
        let mtree = render_mtree(root, mtime, file_meta)?;
        append_file_bytes(&mut builder, ".MTREE", mtree.as_bytes(), 0o644, mtime, None)?;

        // Optional .INSTALL (pre/post-upgrade hooks).
        if let Some(script) = install_script {
            if !script.trim().is_empty() {
                append_file_bytes(
                    &mut builder,
                    ".INSTALL",
                    script.as_bytes(),
                    0o644,
                    mtime,
                    None,
                )?;
            }
        }

        // Payload: recursively add root contents (sorted, normalized).
        append_dir_sorted(&mut builder, root, "", root, mtime, file_meta)?;

        builder.finish()?;
    }

    // Zstd compress deterministically (level 19, single-threaded).
    let zst = zstd_compress(&tar_bytes)?;

    std::fs::write(arch_path, &zst)
        .with_context(|| format!("failed to write '{}'", arch_path.display()))?;
    Ok(())
}

/// Extract a `.pkg.tar.zst`'s payload into `dest`, skipping the `.PKGINFO`
/// / `.MTREE` metadata members (the inverse of [`build`]).
pub fn extract(arch_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(arch_path)
        .with_context(|| format!("failed to open '{}'", arch_path.display()))?;
    let decoder = zstd::stream::read::Decoder::new(file)
        .with_context(|| format!("failed to zstd-decode '{}'", arch_path.display()))?;
    let mut archive = tar::Archive::new(decoder);
    std::fs::create_dir_all(dest)?;
    for entry in archive
        .entries()
        .with_context(|| format!("failed to read entries of '{}'", arch_path.display()))?
    {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let name = path.to_string_lossy();
        if name == ".PKGINFO" || name == ".MTREE" {
            continue;
        }
        entry
            .unpack_in(dest)
            .with_context(|| format!("failed to unpack '{name}'"))?;
    }
    Ok(())
}

fn calc_installed_size(root: &Path) -> Result<u64> {
    fn walk(dir: &Path, total: &mut u64) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                walk(&entry.path(), total)?;
            } else if ty.is_file() {
                *total += entry.metadata()?.len();
            }
            // symlinks contribute 0
        }
        Ok(())
    }
    let mut size = 0u64;
    if root.exists() {
        walk(root, &mut size)?;
    }
    Ok(size)
}

pub fn render_pkginfo(
    meta: &PackageMeta,
    pkgver: &str,
    arch: &str,
    mtime: i64,
    size: u64,
) -> String {
    render_pkginfo_with(
        meta,
        &PackageRelations::default(),
        None,
        pkgver,
        arch,
        mtime,
        size,
    )
}

/// Like [`render_pkginfo`] but emits pacman relation and config metadata.
pub fn render_pkginfo_with(
    meta: &PackageMeta,
    relations: &PackageRelations,
    packager: Option<&str>,
    pkgver: &str,
    arch: &str,
    mtime: i64,
    size: u64,
) -> String {
    let name = meta.name;
    let desc = if meta.description.is_empty() {
        "No description".to_string()
    } else {
        meta.description.to_string()
    };
    let url = if meta.url.is_empty() {
        format!("https://github.com/{name}")
    } else {
        meta.url.to_string()
    };
    let license = if meta.license.is_empty() {
        "custom"
    } else {
        meta.license
    };
    // packager fallback
    let packager = packager
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(DEFAULT_PACKAGER);
    // Arch's builddate is unix epoch seconds.
    let builddate = mtime.max(0).to_string();

    let mut out = format!(
        "pkgname = {name}\n\
         pkgver = {pkgver}\n\
         pkgdesc = {desc}\n\
         url = {url}\n\
         builddate = {builddate}\n\
         packager = {packager}\n\
         size = {size}\n\
         arch = {arch}\n\
         license = {license}\n"
    );

    // Relation/config tags. pacman reads these regardless of order.
    let tags: [(&str, &[String]); 6] = [
        ("depend", relations.depends),
        ("optdepend", relations.optdepends),
        ("conflict", relations.conflicts),
        ("provides", relations.provides),
        ("replaces", relations.replaces),
        ("backup", relations.backup),
    ];
    for (tag, entries) in tags {
        for entry in entries {
            let entry = entry.trim();
            if !entry.is_empty() {
                out.push_str(&format!("{tag} = {entry}\n"));
            }
        }
    }
    out
}

/// Render an Arch `.INSTALL` file from the pre/post-upgrade hooks.
/// Mirrors nfpm's `archlinux.scripts.preupgrade`/`postupgrade`. Each
/// non-empty hook becomes a `pre_upgrade()` / `post_upgrade()` function
/// in the returned string. Returns `None` when neither hook is set, so
/// the caller can skip the `.INSTALL` member entirely.
pub fn render_install_script(preupgrade: &str, postupgrade: &str) -> Option<String> {
    let pre = preupgrade.trim();
    let post = postupgrade.trim();
    if pre.is_empty() && post.is_empty() {
        return None;
    }
    let mut out = String::new();
    if !pre.is_empty() {
        out.push_str(&format!(
            "pre_upgrade() {{
{pre}
}}

"
        ));
    }
    if !post.is_empty() {
        out.push_str(&format!(
            "post_upgrade() {{
{post}
}}

"
        ));
    }
    Some(out)
}

fn render_mtree(root: &Path, mtime: i64, file_meta: &FileMetaMap) -> Result<String> {
    // Minimal mtree: #mtree header + entries for each file/dir/symlink.
    // Format: ./path type=file mode=644 time=mtime size=123 sha256digest=...
    let mut out = String::from("#mtree\n");
    // Add entries for .PKGINFO and .MTREE themselves (size unknown at this
    // point, use 0 – pacman only verifies presence).
    out.push_str(&format!(
        "./.PKGINFO time={}.0 mode=644 type=file size=0\n",
        mtime.max(0)
    ));
    out.push_str(&format!(
        "./.MTREE time={}.0 mode=644 type=file size=0\n",
        mtime.max(0)
    ));

    fn walk_mtree(
        original_root: &Path,
        dir: &Path,
        mtime: i64,
        file_meta: &FileMetaMap,
        out: &mut String,
    ) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let fs_path = entry.path();
            let rel = fs_path
                .strip_prefix(original_root)
                .expect("walked path must be under original_root");
            let mtree_path = format!("./{}", rel.to_string_lossy());
            let ty = entry.file_type()?;
            let entry_meta = filemeta::lookup(file_meta, &rel.to_string_lossy());
            let time = entry_meta.and_then(|m| m.mtime).unwrap_or(mtime).max(0);
            if ty.is_symlink() {
                let target = std::fs::read_link(&fs_path)?;
                let mode = entry_meta.and_then(|m| m.mode).unwrap_or(0o777);
                out.push_str(&format!(
                    "{mtree_path} time={time}.0 mode={mode:o} type=link link={}\n",
                    target.to_string_lossy()
                ));
            } else if ty.is_dir() {
                // `disown_subtree` dirs stay out of the manifest; the files
                // beneath them are still listed.
                let disowned = entry_meta.is_some_and(|m| m.disown);
                if !disowned {
                    let mode = entry_meta.and_then(|m| m.mode).unwrap_or(0o755);
                    out.push_str(&format!(
                        "{mtree_path} time={time}.0 mode={mode:o} type=dir\n"
                    ));
                }
                walk_mtree(original_root, &fs_path, mtime, file_meta, out)?;
            } else if ty.is_file() {
                let meta = std::fs::metadata(&fs_path)?;
                let size = meta.len();
                let mode = entry_meta
                    .and_then(|m| m.mode)
                    .unwrap_or_else(|| meta.permissions().mode() & 0o777);
                // sha256 for mtree digest (optional but nice)
                let digest = sha256_hex(&std::fs::read(&fs_path)?);
                out.push_str(&format!(
                    "{mtree_path} time={time}.0 mode={mode:o} type=file size={size} sha256digest={digest}\n"
                ));
            }
        }
        Ok(())
    }

    walk_mtree(root, root, mtime, file_meta, &mut out)?;
    Ok(out)
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

fn append_dir_sorted<W: Write>(
    builder: &mut tar::Builder<W>,
    original_root: &Path,
    archive_prefix: &str,
    fs_dir: &Path,
    mtime: i64,
    meta: &FileMetaMap,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(fs_dir)
        .with_context(|| format!("failed to read '{}'", fs_dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let fs_path = entry.path();
        let rel = fs_path
            .strip_prefix(original_root)
            .expect("walked path must be under original_root");
        let archive_path = format!("{archive_prefix}{}", rel.to_string_lossy());
        let file_type = entry.file_type()?;
        let entry_meta = filemeta::lookup(meta, &archive_path);

        if file_type.is_symlink() {
            let target = std::fs::read_link(&fs_path)?;
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_uid(0);
            header.set_gid(0);
            filemeta::apply_tar_header(
                &mut header,
                entry_meta,
                crate::constants::SYMLINK_MODE,
                mtime,
            );
            header.set_path(&archive_path)?;
            header.set_link_name(&target)?;
            header.set_cksum();
            builder.append(&header, std::io::empty())?;
        } else if file_type.is_dir() {
            if !entry_meta.is_some_and(|m| m.disown) {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_uid(0);
                header.set_gid(0);
                filemeta::apply_tar_header(
                    &mut header,
                    entry_meta,
                    crate::constants::DIR_MODE,
                    mtime,
                );
                header.set_path(format!("{archive_path}/"))?;
                header.set_cksum();
                builder.append(&header, std::io::empty())?;
            }
            append_dir_sorted(
                builder,
                original_root,
                archive_prefix,
                &fs_path,
                mtime,
                meta,
            )?;
        } else {
            let content = std::fs::read(&fs_path)
                .with_context(|| format!("failed to read '{}'", fs_path.display()))?;
            let mode = std::fs::metadata(&fs_path)?.permissions().mode() & 0o777;
            append_file_bytes(builder, &archive_path, &content, mode, mtime, entry_meta)?;
        }
    }
    Ok(())
}

fn append_file_bytes<W: Write>(
    builder: &mut tar::Builder<W>,
    archive_path: &str,
    content: &[u8],
    mode: u32,
    mtime: i64,
    meta: Option<&FileMeta>,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(content.len() as u64);
    header.set_uid(0);
    header.set_gid(0);
    filemeta::apply_tar_header(&mut header, meta, mode, mtime);
    header.set_path(archive_path)?;
    header.set_cksum();
    builder.append(&header, content)?;
    Ok(())
}

fn zstd_compress(data: &[u8]) -> Result<Vec<u8>> {
    // Level 19, single-threaded for determinism (zstd's default).
    let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 19)?;
    // Disable checksum for determinism? Keep default.
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

pub fn to_pacman_arch(debian_arch: &str) -> &'static str {
    crate::constants::to_pacman_arch(debian_arch)
}
