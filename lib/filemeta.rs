// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-file metadata overrides produced by `contents[].file_info` and
//! `contents[].disown_subtree`.
//!
//! The archive writers walk the staged filesystem, where mode comes from the
//! file's own permission bits and ownership/timestamps are normalized to the
//! reproducible defaults (uid/gid 0, one global mtime). nfpm's `file_info:`
//! lets a `contents:` entry override those per path — including owner/group
//! names that need not exist on the build host — so the staging step records
//! the overrides here and each writer consults the map by installed path
//! (`/usr/bin/foo`).
//!
//! Absent entries mean "use the existing behavior", preserving reproducible
//! builds for everything the config does not override.

use anyhow::Result;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;

/// RPM file classification from `contents[].type` (`doc`/`license`/`readme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpmFileKind {
    Doc,
    License,
    Readme,
}

/// Per-path metadata override.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileMeta {
    /// Permission bits (no file-type bits).
    pub mode: Option<u32>,
    /// Owner user name (archive header `uname`).
    pub owner: Option<String>,
    /// Owner group name (archive header `gname`).
    pub group: Option<String>,
    /// Modification time, Unix epoch seconds.
    pub mtime: Option<i64>,
    /// RPM `%lang(<lang>)` tag (parsed; not yet emitted by the `rpm` crate).
    pub lang: Option<String>,
    /// Directory is not owned by the package (`disown_subtree`): writers
    /// skip its explicit entry and let the parent file imply the directory.
    pub disown: bool,
    /// RPM `%doc` / `%license` / `%readme` classification.
    pub rpm_kind: Option<RpmFileKind>,
}

impl FileMeta {
    /// True when nothing is set and the entry can be omitted.
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.owner.is_none()
            && self.group.is_none()
            && self.mtime.is_none()
            && self.lang.is_none()
            && !self.disown
            && self.rpm_kind.is_none()
    }
}

/// Overrides keyed by absolute installed path (e.g. `/usr/bin/tool`).
pub type FileMetaMap = HashMap<String, FileMeta>;

/// Normalize a root-relative archive path (`usr/bin/foo`, `./usr/bin/foo`)
/// to the installed path used as the map key (`/usr/bin/foo`).
pub fn installed_path(rel: &str) -> String {
    let rel = rel.trim_start_matches("./").trim_start_matches('/');
    format!("/{rel}")
}

/// Resolve a user name to its numeric id, falling back to 0 (`root`). The
/// name is still recorded in the tar header's `uname` field, so a name that
/// does not exist on the build host survives into the package.
pub fn resolve_uid(name: &str) -> u32 {
    resolve_nss(name, true)
}

/// Resolve a group name to its numeric id, falling back to 0.
pub fn resolve_gid(name: &str) -> u32 {
    resolve_nss(name, false)
}

fn resolve_nss(name: &str, user: bool) -> u32 {
    let Ok(cname) = CString::new(name) else {
        return 0;
    };
    // SAFETY: `cname` is a valid NUL-terminated C string for the duration of
    // the call; getpwnam/getgrnam return pointers into libc-owned storage
    // that we only read (uid/gid) before any other NSS call could reuse it.
    unsafe {
        if user {
            let pw = libc::getpwnam(cname.as_ptr());
            if pw.is_null() {
                0
            } else {
                (*pw).pw_uid
            }
        } else {
            let gr = libc::getgrnam(cname.as_ptr());
            if gr.is_null() {
                0
            } else {
                (*gr).gr_gid
            }
        }
    }
}

/// Look up the override for an archive path (`usr/bin/foo` or
/// `./usr/bin/foo`).
pub fn lookup<'a>(map: &'a FileMetaMap, archive_path: &str) -> Option<&'a FileMeta> {
    map.get(&installed_path(archive_path.trim_end_matches('/')))
}

/// Apply `meta` (or the reproducible defaults) to a tar entry header. Shared
/// by the deb/arch/apk writers, which all emit GNU tar entries by hand.
pub fn apply_tar_header(
    header: &mut tar::Header,
    meta: Option<&FileMeta>,
    default_mode: u32,
    default_mtime: i64,
) {
    let mode = meta.and_then(|m| m.mode).unwrap_or(default_mode);
    header.set_mode(mode);
    let mtime = meta.and_then(|m| m.mtime).unwrap_or(default_mtime).max(0) as u64;
    header.set_mtime(mtime);
    let (uid, gid) = match meta {
        Some(m) => (
            m.owner.as_deref().map(resolve_uid).unwrap_or(0),
            m.group.as_deref().map(resolve_gid).unwrap_or(0),
        ),
        None => (0, 0),
    };
    header.set_uid(u64::from(uid));
    header.set_gid(u64::from(gid));
    if let Some(m) = meta {
        if let Some(owner) = &m.owner {
            let _ = header.set_username(owner);
        }
        if let Some(group) = &m.group {
            let _ = header.set_groupname(group);
        }
    }
}

/// True when `path` begins with the ELF magic (`\x7fELF`). A short/unreadable
/// file is not an ELF (returns `Ok(false)` for a short read).
pub fn is_elf(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(&magic == b"\x7fELF")
}
