// SPDX-License-Identifier: GPL-3.0-or-later

//! Build a `.deb` archive entirely in-process -- no `dpkg-deb`,
//! no subprocess at all.
//!
//! A `.deb` is just an `ar` container of three members (`debian-binary`,
//! `control.tar.gz`, `data.tar.gz`); nothing about that format requires an
//! external tool. Mirrors the same approach `nfpm` (Go) and `cargo-deb`
//! (Rust) both use for the same reason.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::filemeta::{self, FileMeta, FileMetaMap};

/// Build a `.deb` from a staged filesystem tree and a rendered control
/// file.
///
/// `root` is the staged tree; its contents become `data.tar`'s payload
/// (e.g. `root/usr/bin/foo` becomes `./usr/bin/foo` in the archive).
/// `control` is the already-rendered `DEBIAN/control` file content.
/// `mtime` (Unix epoch seconds) is stamped on every tar/ar entry and every
/// gzip header instead of each file's real filesystem timestamp, and the
/// tree is walked in sorted order -- so the result depends only on
/// `root`'s content, never on extraction order or wall-clock build time
/// (see `docs/decisions/2026-08-20-reproducible-builds.md`).
pub fn build(root: &Path, control: &[u8], mtime: i64, deb_path: &Path) -> Result<()> {
    build_with_compression(root, control, mtime, deb_path, "gzip")
}

/// Like [`build`] but with explicit `compression`: "gzip" (default),
/// "xz", "zstd", or "none". The compressed members are named accordingly
/// (`data.tar.gz` / `data.tar.xz` / `data.tar.zst` / `data.tar`) — dpkg
/// accepts all four. Mirrors nfpm's `deb.compression` option.
pub fn build_with_compression(
    root: &Path,
    control: &[u8],
    mtime: i64,
    deb_path: &Path,
    compression: &str,
) -> Result<()> {
    build_full(root, control, mtime, deb_path, compression, &[], None)
}

/// An extra control-member file beyond `control` and `md5sums`:
/// maintainer scripts (`postinst` etc., mode 0755) and `conffiles`
/// (mode 0644). Passed to [`build_full`].
#[derive(Debug, Clone)]
pub struct ControlMember {
    /// Member name inside the control tar, e.g. `"postinst"`, `"conffiles"`.
    pub name: String,
    pub content: Vec<u8>,
    /// Unix mode, e.g. 0o755 for maintainer scripts, 0o644 for conffiles.
    pub mode: u32,
}

/// Callback that turns the debsign payload (concatenated `debian-binary` +
/// control.tar.* + data.tar.*) into `_gpg{type}` member bytes.
pub type OriginSigner<'a> = dyn Fn(&[u8]) -> Result<Vec<u8>> + 'a;

/// Full-fidelity build: like [`build_with_compression`] plus additional
/// control members (maintainer scripts, conffiles). Extras are emitted in
/// the given order after `control` + `md5sums`; callers pass a sorted list
/// so output stays reproducible.
///
/// When `origin_signer` is `Some`, appends `_gpgorigin` (see
/// [`build_full_signed`] for other roles).
#[allow(clippy::type_complexity)]
pub fn build_full(
    root: &Path,
    control: &[u8],
    mtime: i64,
    deb_path: &Path,
    compression: &str,
    extras: &[ControlMember],
    origin_signer: Option<&OriginSigner>,
) -> Result<()> {
    build_full_signed(
        root,
        control,
        mtime,
        deb_path,
        compression,
        extras,
        origin_signer.map(|s| (s, "origin")),
    )
}

/// Like [`build_full`] but with an explicit debsign role (`origin` /
/// `maint` / `archive`) controlling the `_gpg{type}` member name.
#[allow(clippy::type_complexity)]
pub fn build_full_signed(
    root: &Path,
    control: &[u8],
    mtime: i64,
    deb_path: &Path,
    compression: &str,
    extras: &[ControlMember],
    gpg_signer: Option<(&OriginSigner, &str)>,
) -> Result<()> {
    build_full_signed_with_meta(
        root,
        control,
        mtime,
        deb_path,
        compression,
        extras,
        gpg_signer,
        &FileMetaMap::new(),
    )
}

/// [`build_full`] with per-file `contents[].file_info` overrides.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn build_full_with_meta(
    root: &Path,
    control: &[u8],
    mtime: i64,
    deb_path: &Path,
    compression: &str,
    extras: &[ControlMember],
    origin_signer: Option<&OriginSigner>,
    meta: &FileMetaMap,
) -> Result<()> {
    build_full_signed_with_meta(
        root,
        control,
        mtime,
        deb_path,
        compression,
        extras,
        origin_signer.map(|s| (s, "origin")),
        meta,
    )
}

/// [`build_full_signed`] with per-file `contents[].file_info` overrides.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn build_full_signed_with_meta(
    root: &Path,
    control: &[u8],
    mtime: i64,
    deb_path: &Path,
    compression: &str,
    extras: &[ControlMember],
    gpg_signer: Option<(&OriginSigner, &str)>,
    meta: &FileMetaMap,
) -> Result<()> {
    let comp = normalize_compression(compression)
        .with_context(|| format!("invalid compression '{compression}'"))?;
    let mut md5sums = String::new();
    let data_tar = build_data_tar(root, mtime, &mut md5sums, &comp, meta)
        .with_context(|| format!("failed to build data.tar.{ext}", ext = comp.ext()))?;
    let control_tar = build_control_tar(control, md5sums.as_bytes(), mtime, &comp, extras)
        .with_context(|| format!("failed to build control.tar.{ext}", ext = comp.ext()))?;
    write_ar_with_compression(deb_path, mtime, &control_tar, &data_tar, &comp, gpg_signer)
        .context("failed to write .deb ar container")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionKind {
    Gzip,
    Xz,
    Zstd,
    None,
}

impl CompressionKind {
    fn ext(&self) -> &'static str {
        match self {
            CompressionKind::Gzip => "gz",
            CompressionKind::Xz => "xz",
            CompressionKind::Zstd => "zst",
            CompressionKind::None => "tar",
        }
    }
}

/// A compression algorithm plus optional explicit level (the `:N` suffix
/// of `compression:`, e.g. `gzip:1`, `xz:6`, `zstd:19`). Levels are
/// validated per algorithm; the defaults match what was hardcoded before
/// levels became configurable (gzip best/9, xz preset 9, zstd 19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compression {
    pub kind: CompressionKind,
    pub level: Option<u32>,
}

impl Compression {
    fn ext(&self) -> &'static str {
        self.kind.ext()
    }
}

pub fn normalize_compression(s: &str) -> Result<Compression> {
    let trimmed = s.trim().to_ascii_lowercase();
    let (base, level_raw) = match trimmed.split_once(':') {
        Some((b, l)) => (b.trim(), Some(l.trim())),
        None => (trimmed.as_str(), None),
    };
    let kind = match base {
        // Empty string falls back to gzip (previous lenient behavior).
        "" | "gzip" | "gz" => CompressionKind::Gzip,
        "xz" => CompressionKind::Xz,
        "zstd" | "zst" => CompressionKind::Zstd,
        "none" | "tar" => CompressionKind::None,
        other => anyhow::bail!("unknown compression '{other}'"),
    };
    // Per-algorithm valid ranges. Defaults: gzip/xz max quality (9),
    // zstd 19 (dpkg's typical zstd level).
    let (min, max, default): (u32, u32, u32) = match kind {
        CompressionKind::Gzip | CompressionKind::Xz => (0, 9, 9),
        CompressionKind::Zstd => (1, 22, 19),
        CompressionKind::None => (0, 0, 0),
    };
    if kind == CompressionKind::None {
        if level_raw.is_some() {
            anyhow::bail!("compression 'none' takes no :level suffix");
        }
        return Ok(Compression { kind, level: None });
    }
    let level = match level_raw {
        None => default,
        Some(raw) => {
            let lvl: u32 = raw
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid {base} level '{raw}' (expected integer)"))?;
            if lvl < min || lvl > max {
                anyhow::bail!("{base} level {lvl} out of range ({min}-{max})");
            }
            lvl
        }
    };
    Ok(Compression {
        kind,
        level: Some(level),
    })
}

fn compress_tar(data: &[u8], mtime: i64, comp: &Compression) -> Result<Vec<u8>> {
    let level = comp.level.unwrap_or_default();
    match comp.kind {
        CompressionKind::Gzip => gzip(data, mtime, level),
        CompressionKind::Xz => xz(data, level),
        CompressionKind::Zstd => zstd_compress(data, level),
        CompressionKind::None => Ok(data.to_vec()),
    }
}

fn zstd_compress(data: &[u8], level: u32) -> Result<Vec<u8>> {
    zstd::bulk::compress(data, level as i32).context("failed to zstd compress")
}

/// Recursively tar+gzip `root`'s contents (sorted, normalized ownership
/// and mtime), accumulating an md5sums listing as it goes.
pub fn build_data_tar_gz(root: &Path, mtime: i64, md5sums: &mut String) -> Result<Vec<u8>> {
    build_data_tar_gz_with_meta(root, mtime, md5sums, &FileMetaMap::new())
}

/// [`build_data_tar_gz`] with per-file `contents[].file_info` overrides.
pub fn build_data_tar_gz_with_meta(
    root: &Path,
    mtime: i64,
    md5sums: &mut String,
    meta: &FileMetaMap,
) -> Result<Vec<u8>> {
    build_data_tar(
        root,
        mtime,
        md5sums,
        &Compression {
            kind: CompressionKind::Gzip,
            level: None,
        },
        meta,
    )
}

fn build_data_tar(
    root: &Path,
    mtime: i64,
    md5sums: &mut String,
    comp: &Compression,
    meta: &FileMetaMap,
) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_dir_sorted(&mut builder, root, "./", root, mtime, Some(md5sums), meta)?;
        builder.finish()?;
    }
    compress_tar(&tar_bytes, mtime, comp)
}

/// Tar+xz `fs_root`'s entire subtree (sorted, normalized ownership and
/// mtime), with every entry's archive path prefixed by `archive_prefix`
/// (e.g. `"eza-0.23.5/usr/"` so `fs_root/bin/eza` becomes
/// `"eza-0.23.5/usr/bin/eza"` in the archive -- the upstream/Debian source
/// tarball convention, not `data.tar`'s `"./"`-relative one). Used by
/// `source.rs` for a generated source package's `.orig.tar.xz` and
/// `.debian.tar.xz` members, matching real `dpkg-source -b` output (see
/// `docs/decisions/2026-08-20-docker-free-lintian-source.md`'s xz addendum).
pub fn tar_xz_tree(fs_root: &Path, archive_prefix: &str, mtime: i64) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_dir_sorted(
            &mut builder,
            fs_root,
            archive_prefix,
            fs_root,
            mtime,
            None,
            &FileMetaMap::new(),
        )?;
        builder.finish()?;
    }
    xz(&tar_bytes, 9)
}

/// Extract a `.deb`'s `data.tar.*` payload into `dest` (the inverse of
/// `build`, minus the control member). Handles gzip/xz/zstd/uncompressed
/// `data.tar.*` members produced by any `compression` setting.
pub fn extract(deb_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(deb_path)
        .with_context(|| format!("failed to open '{}'", deb_path.display()))?;
    let mut archive = ar::Archive::new(file);
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry?;
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if name.starts_with("data.tar") {
            std::fs::create_dir_all(dest)?;
            if name == "data.tar.gz" {
                let gz = flate2::read::GzDecoder::new(&mut entry);
                return tar::Archive::new(gz).unpack(dest).with_context(|| {
                    format!("failed to unpack data.tar.gz from '{}'", deb_path.display())
                });
            }
            if name == "data.tar.xz" {
                let mut data = Vec::new();
                std::io::copy(&mut entry, &mut data)?;
                let decoder = lzma_rust2::XzReader::new(data.as_slice(), true);
                return tar::Archive::new(decoder).unpack(dest).with_context(|| {
                    format!("failed to unpack data.tar.xz from '{}'", deb_path.display())
                });
            }
            if name == "data.tar.zst" || name == "data.tar.zstd" {
                let mut data = Vec::new();
                std::io::copy(&mut entry, &mut data)?;
                let decoded = zstd::bulk::decompress(&data, MAX_ZSTD_DECOMPRESSED_BYTES)
                    .context("failed to zstd decompress data.tar.zst")?;
                return tar::Archive::new(decoded.as_slice())
                    .unpack(dest)
                    .with_context(|| {
                        format!(
                            "failed to unpack data.tar.zst from '{}'",
                            deb_path.display()
                        )
                    });
            }
            if name == "data.tar" {
                return tar::Archive::new(&mut entry).unpack(dest).with_context(|| {
                    format!("failed to unpack data.tar from '{}'", deb_path.display())
                });
            }
        }
    }
    bail!(
        "'{}' has no data.tar.* member (supported: data.tar, data.tar.gz, data.tar.xz, data.tar.zst)",
        deb_path.display()
    );
}

/// Tar+gzip the control member set (just `control` + `md5sums` today --
/// no maintainer scripts or conffiles are needed for a repackaged upstream
/// binary).
/// Upper bound on the decompressed size accepted when unpacking a
/// zstd-compressed `data.tar.zst` member. `zstd::bulk::decompress`
/// requires a pre-allocated output bound; 256 MiB comfortably covers any
/// package lx produces while still capping memory on corrupt input.
const MAX_ZSTD_DECOMPRESSED_BYTES: usize = 256 * 1024 * 1024;

#[allow(dead_code)]
fn build_control_tar_gz(control: &[u8], md5sums: &[u8], mtime: i64) -> Result<Vec<u8>> {
    build_control_tar(
        control,
        md5sums,
        mtime,
        &Compression {
            kind: CompressionKind::Gzip,
            level: None,
        },
        &[],
    )
}

fn build_control_tar(
    control: &[u8],
    md5sums: &[u8],
    mtime: i64,
    comp: &Compression,
    extras: &[ControlMember],
) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_file_bytes(&mut builder, "./control", control, 0o644, mtime, None)?;
        append_file_bytes(&mut builder, "./md5sums", md5sums, 0o644, mtime, None)?;
        for extra in extras {
            append_file_bytes(
                &mut builder,
                &format!("./{}", extra.name),
                &extra.content,
                extra.mode,
                mtime,
                None,
            )?;
        }
        builder.finish()?;
    }
    compress_tar(&tar_bytes, mtime, comp)
}

/// Recursively append `fs_dir`'s children (sorted by filename) under
/// `archive_prefix` + their path relative to `original_root`, normalizing
/// uid/gid/mtime on every entry. `md5sums`, when given, accumulates
/// `<md5>  <path>` lines for every regular file (dpkg's `md5sums` control
/// member format) using the plain root-relative path regardless of
/// `archive_prefix` -- only `build_data_tar_gz` (the `.deb` case) passes
/// `Some`.
fn append_dir_sorted<W: Write>(
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

/// Deterministic gzip: fixed mtime in the header (no wall-clock leak), no
/// embedded filename/comment/OS-specific fields left to vary.
fn gzip(data: &[u8], mtime: i64, level: u32) -> Result<Vec<u8>> {
    deterministic_gzip_bytes(data, mtime, level)
}

/// Public deterministic-gzip helper for the doc-payload members staged
/// outside the tar members (`changelog.Debian.gz`, gzipped man pages):
/// same fixed-mtime treatment as [`gzip`], so every compressed byte in a
/// built package derives from content + `mtime` alone. Level 9 (best),
/// matching the previous hardcoded behavior at these sites.
pub fn deterministic_gzip_bytes(data: &[u8], mtime: i64, level: u32) -> Result<Vec<u8>> {
    let mut encoder = flate2::GzBuilder::new()
        .mtime(mtime.max(0) as u32)
        .write(Vec::new(), flate2::Compression::new(level));
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

/// Deterministic xz (single-threaded; the xz container has no
/// mtime/filename field to normalize unlike gzip, so determinism just
/// requires avoiding the multi-threaded encoder, whose block-splitting
/// could vary with thread scheduling). `preset` is 0-9; 9 was
/// `dpkg-source -b`'s own default and remains ours.
fn xz(data: &[u8], preset: u32) -> Result<Vec<u8>> {
    let options = lzma_rust2::XzOptions::with_preset(preset);
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), options)?;
    writer.write_all(data)?;
    Ok(writer.finish()?)
}

/// Write the outer `ar` container: `debian-binary`, `control.tar.*`,
/// `data.tar.*`, in that order (dpkg requires this exact order and reads
/// only as much of the archive as it needs, so anything after `data.tar.*`
/// -- e.g. `_gpgorigin` -- is safe to append).
#[allow(dead_code)]
fn write_ar(deb_path: &Path, mtime: i64, control_tar_gz: &[u8], data_tar_gz: &[u8]) -> Result<()> {
    write_ar_with_compression(
        deb_path,
        mtime,
        control_tar_gz,
        data_tar_gz,
        &Compression {
            kind: CompressionKind::Gzip,
            level: None,
        },
        None,
    )
}

fn write_ar_with_compression(
    deb_path: &Path,
    mtime: i64,
    control_tar: &[u8],
    data_tar: &[u8],
    comp: &Compression,
    #[allow(clippy::type_complexity)] gpg_signer: Option<(&OriginSigner, &str)>,
) -> Result<()> {
    let file = std::fs::File::create(deb_path)
        .with_context(|| format!("failed to create '{}'", deb_path.display()))?;
    let mut builder = ar::Builder::new(file);
    let (ctrl_name, data_name) = match comp.kind {
        CompressionKind::Gzip => ("control.tar.gz", "data.tar.gz"),
        CompressionKind::Xz => ("control.tar.xz", "data.tar.xz"),
        CompressionKind::Zstd => ("control.tar.zst", "data.tar.zst"),
        CompressionKind::None => ("control.tar", "data.tar"),
    };
    const DEBIAN_BINARY: &[u8] = b"2.0\n";
    let members: [(&str, &[u8]); 3] = [
        ("debian-binary", DEBIAN_BINARY),
        (ctrl_name, control_tar),
        (data_name, data_tar),
    ];
    for (name, data) in members {
        append_ar_member(&mut builder, name, data, mtime)?;
    }
    if let Some((sign, sig_type)) = gpg_signer {
        let mut payload =
            Vec::with_capacity(DEBIAN_BINARY.len() + control_tar.len() + data_tar.len());
        payload.extend_from_slice(DEBIAN_BINARY);
        payload.extend_from_slice(control_tar);
        payload.extend_from_slice(data_tar);
        let sig = sign(&payload).context("debsign signer failed")?;
        let typ = if sig_type.trim().is_empty() {
            "origin"
        } else {
            sig_type.trim()
        };
        let member = format!("_gpg{typ}");
        append_ar_member(&mut builder, &member, &sig, mtime)?;
    }
    Ok(())
}

fn append_ar_member<W: Write>(
    builder: &mut ar::Builder<W>,
    name: &str,
    data: &[u8],
    mtime: i64,
) -> Result<()> {
    let mut header = ar::Header::new(name.as_bytes().to_vec(), data.len() as u64);
    header.set_mtime(mtime.max(0) as u64);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mode(crate::constants::AR_MODE);
    builder.append(&header, data)?;
    Ok(())
}
