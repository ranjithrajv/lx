// SPDX-License-Identifier: GPL-3.0-or-later

//! Build an Alpine Linux `.apk` (apk-tools v2 format) entirely in-process.
//!
//! An `.apk` is a concatenation of gzip members:
//!
//! ```text
//! [ control.tar.gz ] [ optional signature.tar.gz ] [ data.tar.gz ]
//! ```
//!
//! `control.tar.gz` carries `.PKGINFO` — the key/value metadata apk reads
//! to install the package; `data.tar.gz` carries the payload. The
//! `datahash` line in `.PKGINFO` pins the SHA-256 of the *compressed* data
//! member, which apk verifies on install. Signing is not implemented (that
//! needs an RSA key emitted by `abuild`); unsigned packages install with
//! `apk add --allow-untrusted`.
//!
//! Mirrors `debarchive.rs` / `archarchive.rs`: deterministic tar+gzip,
//! sorted walk, normalized ownership and mtime, no subprocess.

use anyhow::{Context, Result};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Metadata rendered into `.PKGINFO`.
#[derive(Debug, Clone)]
pub struct PackageMeta<'a> {
    /// `pkgname` — package name.
    pub name: &'a str,
    /// `pkgver` — full Alpine version including the `-rN` revision.
    pub version: &'a str,
    pub description: &'a str,
    pub url: &'a str,
    pub license: &'a str,
    /// Runtime dependencies (`depend = …` lines).
    pub depends: &'a [String],
    /// Virtual provides (`provides = …` lines).
    pub provides: &'a [String],
    /// Replacements (`replaces = …` lines).
    pub replaces: &'a [String],
}

/// Build an apk package from a staged filesystem tree.
///
/// `root` is the staged tree (`root/usr/bin/foo` → `/usr/bin/foo`),
/// `arch` is the Alpine architecture name, and `apk_path` is the output.
pub fn build(
    root: &Path,
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    apk_path: &Path,
) -> Result<()> {
    // 1. Data member first: its compressed bytes are hashed into .PKGINFO.
    let data_gz = build_data_tar_gz(root, mtime)?;
    let datahash = sha256_hex(&data_gz);

    // 2. Control member: a tar holding .PKGINFO.
    let installed_size = calc_installed_size(root)?;
    let pkginfo = render_pkginfo(meta, arch, mtime, installed_size, &datahash);
    let control_tar = build_control_tar(&pkginfo, mtime)?;
    let control_gz = deterministic_gzip(&control_tar, mtime, 9)?;

    // 3. Concatenate control + data gzip streams.
    let mut out = Vec::with_capacity(control_gz.len() + data_gz.len());
    out.extend_from_slice(&control_gz);
    out.extend_from_slice(&data_gz);
    std::fs::write(apk_path, &out)
        .with_context(|| format!("failed to write '{}'", apk_path.display()))?;
    Ok(())
}

/// Render the `.PKGINFO` body.
pub fn render_pkginfo(
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    size: u64,
    datahash: &str,
) -> String {
    let desc = if meta.description.is_empty() {
        "No description"
    } else {
        meta.description
    };
    let license = if meta.license.is_empty() {
        "unknown"
    } else {
        meta.license
    };
    let url = if meta.url.is_empty() {
        "https://alpinelinux.org"
    } else {
        meta.url
    };
    let mut out = String::new();
    out.push_str(&format!("pkgname = {}\n", meta.name));
    out.push_str(&format!("pkgver = {}\n", meta.version));
    out.push_str(&format!("pkgdesc = {desc}\n"));
    out.push_str(&format!("url = {url}\n"));
    out.push_str(&format!("builddate = {}\n", mtime.max(0)));
    out.push_str("packager = lx <lx@latest-debs.org>\n");
    out.push_str(&format!("size = {size}\n"));
    out.push_str(&format!("arch = {arch}\n"));
    out.push_str(&format!("origin = {}\n", meta.name));
    out.push_str(&format!("license = {license}\n"));
    for dep in meta.depends {
        let dep = dep.trim();
        if !dep.is_empty() {
            out.push_str(&format!("depend = {dep}\n"));
        }
    }
    for p in meta.provides {
        let p = p.trim();
        if !p.is_empty() {
            out.push_str(&format!("provides = {p}\n"));
        }
    }
    for r in meta.replaces {
        let r = r.trim();
        if !r.is_empty() {
            out.push_str(&format!("replaces = {r}\n"));
        }
    }
    if !datahash.is_empty() {
        out.push_str(&format!("datahash = {datahash}\n"));
    }
    out
}

fn build_control_tar(pkginfo: &str, mtime: i64) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_file_bytes(&mut builder, ".PKGINFO", pkginfo.as_bytes(), 0o644, mtime)?;
        builder.finish()?;
    }
    Ok(tar_bytes)
}

fn build_data_tar_gz(root: &Path, mtime: i64) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_dir_sorted(&mut builder, root, root, mtime)?;
        builder.finish()?;
    }
    deterministic_gzip(&tar_bytes, mtime, 9)
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
        }
        Ok(())
    }
    let mut size = 0u64;
    if root.exists() {
        walk(root, &mut size)?;
    }
    Ok(size)
}

fn append_dir_sorted<W: Write>(
    builder: &mut tar::Builder<W>,
    original_root: &Path,
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
        let archive_path = rel.to_string_lossy().to_string();
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
            append_dir_sorted(builder, original_root, &fs_path, mtime)?;
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

/// Deterministic gzip member: fixed mtime header, no variable OS field.
fn deterministic_gzip(data: &[u8], mtime: i64, level: u32) -> Result<Vec<u8>> {
    let mut encoder = flate2::GzBuilder::new()
        .mtime(mtime.max(0) as u32)
        .write(Vec::new(), flate2::Compression::new(level));
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// Map Debian arch to Alpine arch (re-exported for the plugin).
pub fn to_alpine_arch(debian_arch: &str) -> &'static str {
    crate::constants::to_alpine_arch(debian_arch)
}
