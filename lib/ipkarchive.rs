// SPDX-License-Identifier: GPL-3.0-or-later

//! Build an OpenWrt / opkg `.ipk` entirely in-process.
//!
//! An `.ipk` is an `ar` container of `debian-binary`, `control.tar.gz`, and
//! `data.tar.gz` — byte-for-byte the same layout as a `.deb`, with an
//! OpenWrt-flavoured control file. Rather than duplicate the `ar`/tar/gzip
//! machinery, this module delegates to [`crate::debarchive`] and only owns
//! the ipk-specific naming and control dialect (see
//! [`crate::plugins::ipk`]).
//!
//! Gzip is the only compression opkg has always accepted, so the ipk path
//! pins it regardless of the shared `compression:` setting.

use anyhow::{Context, Result};
use std::path::Path;

use crate::debarchive::ControlMember;
use crate::filemeta::FileMetaMap;

/// Build an `.ipk` from a staged filesystem tree and a rendered control
/// file. `control` is the already-rendered OpenWrt control content.
pub fn build(root: &Path, control: &[u8], mtime: i64, ipk_path: &Path) -> Result<()> {
    build_with_scripts(root, control, &[], mtime, ipk_path)
}

/// Build an `.ipk` with extra control members: opkg maintainer scripts
/// (`preinst`/`postinst`/`prerm`/`postrm`) and a `conffiles` list. Extra
/// members ride along in `control.tar.gz`, exactly like a `.deb`.
pub fn build_with_scripts(
    root: &Path,
    control: &[u8],
    extras: &[ControlMember],
    mtime: i64,
    ipk_path: &Path,
) -> Result<()> {
    build_with_scripts_with_meta(root, control, extras, mtime, ipk_path, &FileMetaMap::new())
}

/// [`build_with_scripts`] with per-file `contents[].file_info` overrides.
pub fn build_with_scripts_with_meta(
    root: &Path,
    control: &[u8],
    extras: &[ControlMember],
    mtime: i64,
    ipk_path: &Path,
    meta: &FileMetaMap,
) -> Result<()> {
    crate::debarchive::build_full_with_meta(
        root, control, mtime, ipk_path, "gzip", extras, None, meta,
    )
    .with_context(|| format!("failed to build {}", ipk_path.display()))
}

/// Extract an `.ipk`'s `data.tar.gz` payload into `dest` (the inverse of
/// [`build`]). ipk and deb share the `ar` + `data.tar.*` layout, so this
/// reuses the deb extractor.
pub fn extract(ipk_path: &Path, dest: &Path) -> Result<()> {
    crate::debarchive::extract(ipk_path, dest)
        .with_context(|| format!("failed to extract {}", ipk_path.display()))
}
