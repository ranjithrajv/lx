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
//! member, which apk verifies on install. When an RSA key is supplied the
//! control segment is signed and a `.SIGN.RSA.<keyname>` segment is
//! prepended ([`build_with_signature`]); unsigned packages install with
//! `apk add --allow-untrusted`.
//!
//! Mirrors `debarchive.rs` / `archarchive.rs`: deterministic tar+gzip,
//! sorted walk, normalized ownership and mtime, no subprocess.

use anyhow::{Context, Result};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::filemeta::{self, FileMeta, FileMetaMap};

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
    build_with_signature(root, meta, arch, mtime, apk_path, None)
}

/// A control-segment signer for apk v2. `member_name` is the full
/// `.SIGN.RSA.<keyname>` tar member; `sign` receives the gzipped control
/// segment and returns its DER PKCS#1 v1.5 RSA signature.
pub struct ApkSigner<'a> {
    pub member_name: &'a str,
    pub sign: &'a dyn Fn(&[u8]) -> Result<Vec<u8>>,
}

/// An extra file in the apk *control* segment — install scripts
/// (`.pre-install`, `.post-install`, `.pre-deinstall`, `.post-deinstall`,
/// `.pre-upgrade`, `.post-upgrade`) and any other control data.
pub struct ApkControlFile<'a> {
    pub name: &'a str,
    pub content: &'a [u8],
    pub mode: u32,
}

/// Build an apk, optionally signed. A signed apk v2 is
/// `[signature.tar.gz][control.tar.gz][data.tar.gz]`; the signature covers
/// the compressed control segment.
pub fn build_with_signature(
    root: &Path,
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    apk_path: &Path,
    signer: Option<ApkSigner<'_>>,
) -> Result<()> {
    build_full(root, meta, arch, mtime, apk_path, signer, &[])
}

/// Like [`build_with_signature`] plus extra control-segment files (install
/// scripts).
pub fn build_full(
    root: &Path,
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    apk_path: &Path,
    signer: Option<ApkSigner<'_>>,
    control_files: &[ApkControlFile<'_>],
) -> Result<()> {
    build_full_with_meta(
        root,
        meta,
        arch,
        mtime,
        apk_path,
        signer,
        control_files,
        &FileMetaMap::new(),
    )
}

/// [`build_full`] with per-file `contents[].file_info` overrides.
#[allow(clippy::too_many_arguments)]
pub fn build_full_with_meta(
    root: &Path,
    meta: &PackageMeta,
    arch: &str,
    mtime: i64,
    apk_path: &Path,
    signer: Option<ApkSigner<'_>>,
    control_files: &[ApkControlFile<'_>],
    file_meta: &FileMetaMap,
) -> Result<()> {
    // 1. Data member first: its compressed bytes are hashed into .PKGINFO.
    let data_gz = build_data_tar_gz(root, mtime, file_meta)?;
    let datahash = sha256_hex(&data_gz);

    // 2. Control member: a tar holding .PKGINFO and any install scripts (no
    //    end-of-archive records, so it concatenates with the data segment).
    let installed_size = calc_installed_size(root)?;
    let pkginfo = render_pkginfo(meta, arch, mtime, installed_size, &datahash);
    let control_tar = build_control_tar(&pkginfo, mtime, control_files)?;
    let control_gz = deterministic_gzip(&control_tar, mtime, 9)?;

    // 3. Optional signature segment, prepended over the control segment.
    let signature_gz = match signer {
        Some(s) => {
            let sig = (s.sign)(&control_gz)?;
            Some(signature_segment(s.member_name, &sig, mtime)?)
        }
        None => None,
    };

    // 4. Concatenate signature? + control + data gzip streams.
    let mut out = Vec::with_capacity(
        signature_gz.as_ref().map(|s| s.len()).unwrap_or(0) + control_gz.len() + data_gz.len(),
    );
    if let Some(sig) = &signature_gz {
        out.extend_from_slice(sig);
    }
    out.extend_from_slice(&control_gz);
    out.extend_from_slice(&data_gz);
    std::fs::write(apk_path, &out)
        .with_context(|| format!("failed to write '{}'", apk_path.display()))?;
    Ok(())
}

/// Build a gzipped signature tar segment (`.SIGN.RSA.<keyname>`), without
/// end-of-archive records so it concatenates with the following segments.
pub fn signature_segment(member_name: &str, signature: &[u8], mtime: i64) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_file_bytes(&mut builder, member_name, signature, 0o644, mtime, None)?;
    }
    // `tar::Builder` appends end-of-archive records even without `finish()`;
    // apk wants them only on the final data segment.
    deterministic_gzip(&strip_end_of_archive(tar_bytes), mtime, 9)
}

/// Drop the trailing 512-byte zero end-of-archive records (`tar::Builder`
/// writes them on drop). Non-final apk tar segments must not carry them.
fn strip_end_of_archive(mut tar_bytes: Vec<u8>) -> Vec<u8> {
    while tar_bytes.len() >= 512 && tar_bytes[tar_bytes.len() - 512..].iter().all(|&b| b == 0) {
        tar_bytes.truncate(tar_bytes.len() - 512);
    }
    tar_bytes
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

/// The control tar segment. No end-of-archive records: the data segment
/// (which keeps them) is the only terminator of the concatenated apk tar.
fn build_control_tar(
    pkginfo: &str,
    mtime: i64,
    control_files: &[ApkControlFile<'_>],
) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_file_bytes(
            &mut builder,
            ".PKGINFO",
            pkginfo.as_bytes(),
            0o644,
            mtime,
            None,
        )?;
        for f in control_files {
            append_file_bytes(&mut builder, f.name, f.content, f.mode, mtime, None)?;
        }
    }
    // No end-of-archive records: the data segment (which keeps them) is the
    // only terminator of the concatenated apk tar.
    Ok(strip_end_of_archive(tar_bytes))
}

fn build_data_tar_gz(root: &Path, mtime: i64, meta: &FileMetaMap) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_dir_sorted(&mut builder, root, root, mtime, meta)?;
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
        let archive_path = rel.to_string_lossy().to_string();
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
            // `disown_subtree` dirs are not owned by the package: skip the
            // explicit entry, keep the children.
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
            append_dir_sorted(builder, original_root, &fs_path, mtime, meta)?;
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

/// The `C:` checksum apk records for a package in an `APKINDEX`:
/// `Q1` + base64(SHA-1 of the package's **compressed control segment**).
pub fn control_checksum(apk_path: &Path) -> Result<String> {
    use base64::Engine as _;
    use sha1::Digest;
    let data =
        std::fs::read(apk_path).with_context(|| format!("reading '{}'", apk_path.display()))?;
    let control = control_segment(&data)?;
    let mut h = sha1::Sha1::new();
    h.update(control);
    Ok(format!(
        "Q1{}",
        base64::engine::general_purpose::STANDARD.encode(h.finalize())
    ))
}

/// Read the `.PKGINFO` body from an `.apk`, signed or unsigned.
pub fn read_pkginfo(apk_path: &Path) -> Result<String> {
    use std::io::Read;
    let data =
        std::fs::read(apk_path).with_context(|| format!("reading '{}'", apk_path.display()))?;
    let control = control_segment(&data)?;
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(control)
        .read_to_end(&mut decoded)
        .with_context(|| format!("decompressing control segment of '{}'", apk_path.display()))?;
    let mut tar = tar::Archive::new(decoded.as_slice());
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path()?.to_string_lossy() == ".PKGINFO" {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            return Ok(s);
        }
    }
    anyhow::bail!("'{}' has no .PKGINFO member", apk_path.display())
}

/// The raw (compressed) control segment: the first gzip member, or the
/// second when the package is signed (the first is the `.SIGN.*` segment).
fn control_segment(data: &[u8]) -> Result<&[u8]> {
    let first = gzip_member(data, 0)?;
    if member_is_signature(first)? {
        gzip_member(data, 1)
    } else {
        Ok(first)
    }
}

/// True when the first tar entry of a gzip member is a `.SIGN.*` file.
fn member_is_signature(gz: &[u8]) -> Result<bool> {
    use std::io::Read;
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut decoded)?;
    let mut tar = tar::Archive::new(decoded.as_slice());
    if let Some(entry) = tar.entries()?.next() {
        let entry = entry?;
        return Ok(entry.path()?.to_string_lossy().starts_with(".SIGN."));
    }
    Ok(false)
}

/// The raw bytes of the `n`-th gzip member of `data`.
fn gzip_member(data: &[u8], n: usize) -> Result<&[u8]> {
    let mut start = 0usize;
    for i in 0..=n {
        let len = gzip_member_len(&data[start..])?;
        if i == n {
            return Ok(&data[start..start + len]);
        }
        start += len;
    }
    anyhow::bail!("apk has fewer than {} gzip members", n + 1)
}

/// Length in bytes of the gzip member at the start of `data`.
fn gzip_member_len(data: &[u8]) -> Result<usize> {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b {
        anyhow::bail!("not a gzip stream");
    }
    let flg = data[3];
    let mut i = 10usize; // fixed gzip header
    if flg & 0x04 != 0 {
        // FEXTRA
        if data.len() < i + 2 {
            anyhow::bail!("truncated gzip FEXTRA");
        }
        let xlen = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
        i += 2 + xlen;
    }
    if flg & 0x08 != 0 {
        // FNAME
        while i < data.len() && data[i] != 0 {
            i += 1;
        }
        i += 1;
    }
    if flg & 0x10 != 0 {
        // FCOMMENT
        while i < data.len() && data[i] != 0 {
            i += 1;
        }
        i += 1;
    }
    if flg & 0x02 != 0 {
        i += 2; // FHCRC
    }
    // Inflate the raw deflate stream to locate its end; a `bufread` decoder
    // leaves the unconsumed tail in the reader, so the consumed count is the
    // deflate length.
    use std::io::Read;
    let mut decoder = flate2::bufread::DeflateDecoder::new(&data[i..]);
    let mut sink = Vec::new();
    decoder.read_to_end(&mut sink)?;
    let deflate_len = data.len() - i - decoder.into_inner().len();
    Ok(i + deflate_len + 8) // CRC32 + ISIZE
}
