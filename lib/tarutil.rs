// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared tar/gzip helpers for the archive backends.
//!
//! `debarchive`, `archarchive` and `apkarchive` each walked the staged tree
//! with byte-identical tar-entry emission (regular/symlink/directory entries,
//! uid/gid 0, normalised mtime). The walk lived in all three, so a change to
//! entry emission had to be made three times or the formats silently diverged.
//! It lives here once.

use anyhow::{Context, Result};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::filemeta::{self, FileMeta, FileMetaMap};

/// Recursively append `fs_dir`'s children (sorted by filename) under
/// `archive_prefix` + their path relative to `original_root`, normalizing
/// uid/gid/mtime on every entry. `md5sums`, when given, accumulates
/// `<md5>  <path>` lines for every regular file (dpkg's `md5sums` control
/// member format) using the plain root-relative path regardless of
/// `archive_prefix` -- only the `.deb` data tar passes `Some`.
pub fn append_dir_sorted<W: Write>(
    builder: &mut tar::Builder<W>,
    original_root: &Path,
    archive_prefix: &str,
    fs_dir: &Path,
    mtime: i64,
    mut md5sums: Option<&mut String>,
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
            // `disown_subtree` directories are not owned by the package:
            // recurse, but omit the explicit directory entry so the parent
            // file (or the OS) implies it.
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
                md5sums.as_deref_mut(),
                meta,
            )?;
        } else {
            let content = std::fs::read(&fs_path)
                .with_context(|| format!("failed to read '{}'", fs_path.display()))?;
            let mode = std::fs::metadata(&fs_path)?.permissions().mode() & 0o777;
            if let Some(sums) = md5sums.as_deref_mut() {
                let digest = md5::compute(&content);
                sums.push_str(&format!("{digest:x}  {}\n", rel.to_string_lossy()));
            }
            append_file_bytes(builder, &archive_path, &content, mode, mtime, entry_meta)?;
        }
    }
    Ok(())
}

/// Append one regular-file tar entry with a normalized header.
pub fn append_file_bytes<W: Write>(
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

/// Deterministic gzip: fixed mtime in the header (no wall-clock leak), no
/// embedded filename/comment/OS-specific fields left to vary.
pub fn deterministic_gzip(data: &[u8], mtime: i64, level: u32) -> Result<Vec<u8>> {
    let mut encoder = flate2::GzBuilder::new()
        .mtime(mtime.max(0) as u32)
        .write(Vec::new(), flate2::Compression::new(level));
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

/// Total size of the regular files under `root` (symlinks count 0), used for
/// the pacman/apk "installed size" metadata.
pub fn calc_installed_size(root: &Path) -> Result<u64> {
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

/// Lowercase hex SHA-256 of an in-memory buffer.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}
