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
    let pacman_arch = to_pacman_arch(arch);
    let pkgver = format!("{}-{}", meta.version, meta.release);

    // Calculate installed size by walking the tree.
    let installed_size = calc_installed_size(root)?;

    // Render .PKGINFO
    let pkginfo = render_pkginfo(meta, &pkgver, pacman_arch, mtime, installed_size);

    // Build uncompressed tar in memory.
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);

        // .PKGINFO
        append_file_bytes(&mut builder, ".PKGINFO", pkginfo.as_bytes(), 0o644, mtime)?;
        // .MTREE will be generated after we know all payload entries,
        // but for determinism we need to include it now. Simplest: generate
        // MTREE from the same walk and add as regular file.
        let mtree = render_mtree(root, mtime)?;
        append_file_bytes(&mut builder, ".MTREE", mtree.as_bytes(), 0o644, mtime)?;

        // Optional .INSTALL (pre/post-upgrade hooks).
        if let Some(script) = install_script {
            if !script.trim().is_empty() {
                append_file_bytes(&mut builder, ".INSTALL", script.as_bytes(), 0o644, mtime)?;
            }
        }

        // Payload: recursively add root contents (sorted, normalized).
        append_dir_sorted(&mut builder, root, "", root, mtime)?;

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
    let packager = "lx <lx@latest-debs.org>";
    // Arch's builddate is unix epoch seconds.
    let builddate = mtime.max(0).to_string();

    format!(
        "pkgname = {name}\n\
         pkgver = {pkgver}\n\
         pkgdesc = {desc}\n\
         url = {url}\n\
         builddate = {builddate}\n\
         packager = {packager}\n\
         size = {size}\n\
         arch = {arch}\n\
         license = {license}\n"
    )
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

fn render_mtree(root: &Path, mtime: i64) -> Result<String> {
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

    fn walk_mtree(original_root: &Path, dir: &Path, mtime: i64, out: &mut String) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let fs_path = entry.path();
            let rel = fs_path
                .strip_prefix(original_root)
                .expect("walked path must be under original_root");
            let mtree_path = format!("./{}", rel.to_string_lossy());
            let ty = entry.file_type()?;
            let time = mtime.max(0);
            if ty.is_symlink() {
                let target = std::fs::read_link(&fs_path)?;
                out.push_str(&format!(
                    "{mtree_path} time={time}.0 mode=777 type=link link={}\n",
                    target.to_string_lossy()
                ));
            } else if ty.is_dir() {
                out.push_str(&format!("{mtree_path} time={time}.0 mode=755 type=dir\n"));
                walk_mtree(original_root, &fs_path, mtime, out)?;
            } else if ty.is_file() {
                let meta = std::fs::metadata(&fs_path)?;
                let size = meta.len();
                let mode = meta.permissions().mode() & 0o777;
                // sha256 for mtree digest (optional but nice)
                let digest = sha256_hex(&std::fs::read(&fs_path)?);
                out.push_str(&format!(
                    "{mtree_path} time={time}.0 mode={mode:o} type=file size={size} sha256digest={digest}\n"
                ));
            }
        }
        Ok(())
    }

    walk_mtree(root, root, mtime, &mut out)?;
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

        if file_type.is_symlink() {
            let target = std::fs::read_link(&fs_path)?;
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(crate::constants::SYMLINK_MODE);
            header.set_mtime(mtime.max(0) as u64);
            header.set_uid(0);
            header.set_gid(0);
            header.set_path(&archive_path)?;
            header.set_link_name(&target)?;
            header.set_cksum();
            builder.append(&header, std::io::empty())?;
        } else if file_type.is_dir() {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(crate::constants::DIR_MODE);
            header.set_mtime(mtime.max(0) as u64);
            header.set_uid(0);
            header.set_gid(0);
            header.set_path(format!("{archive_path}/"))?;
            header.set_cksum();
            builder.append(&header, std::io::empty())?;
            append_dir_sorted(builder, original_root, archive_prefix, &fs_path, mtime)?;
        } else {
            let content = std::fs::read(&fs_path)
                .with_context(|| format!("failed to read '{}'", fs_path.display()))?;
            let mode = std::fs::metadata(&fs_path)?.permissions().mode() & 0o777;
            append_file_bytes(builder, &archive_path, &content, mode, mtime)?;
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
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(content.len() as u64);
    header.set_mode(mode);
    header.set_mtime(mtime.max(0) as u64);
    header.set_uid(0);
    header.set_gid(0);
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
