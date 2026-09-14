// SPDX-License-Identifier: GPL-3.0-or-later

//! Build a macOS flat package (`.pkg`) entirely in-process.
//!
//! A flat package is a **xar** archive holding `PackageInfo` and a `Payload`
//! (a gzip-compressed `cpio` archive), plus optional `Scripts` (also cpio).
//! This writer produces the xar container and the package metadata natively —
//! no macOS `pkgbuild`/`xar` needed — mirroring fpm's `osxpkg` output format.
//!
//! Scope: the archive is **unsigned and carries no `Bom`**. That is enough to
//! read the payload/metadata and to serve as fpm-parity output, but some
//! `installer` code paths expect a Bom/signature. This is called out rather
//! than hidden.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

use flate2::write::ZlibEncoder;
use flate2::Compression as GzLevel;
use sha2::{Digest as _, Sha256};

/// Package metadata rendered into `PackageInfo`.
pub struct OsxPackageInfo<'a> {
    pub identifier: &'a str,
    pub version: &'a str,
    pub install_location: &'a str,
    /// `none` | `logout` | `restart` | `shutdown`.
    pub postinstall_action: &'a str,
    /// Script names present in `Scripts` (e.g. `postinstall`).
    pub scripts: &'a [&'a str],
}

/// One member of the xar archive.
struct XarMember {
    name: String,
    /// Bytes as stored in the heap (possibly compressed).
    archived: Vec<u8>,
    /// Uncompressed length.
    length: u64,
    /// Optional xar encoding style (e.g. `application/x-gzip`).
    encoding: Option<&'static str>,
    /// SHA-1 (hex) of the *uncompressed* data.
    extracted_sha1: String,
}

/// Build the `.pkg` at `out` from the staged `root`.
pub fn build(
    root: &Path,
    info: &OsxPackageInfo<'_>,
    scripts: &[(String, Vec<u8>)],
    out: &Path,
    mtime: i64,
) -> Result<()> {
    let (cpio, file_count, installed_kbytes) = payload_cpio(root, mtime)?;
    let payload_gz = gzip(&cpio)?;
    let package_info = render_package_info(info, file_count, installed_kbytes);
    let package_info_bytes = package_info.into_bytes();

    let mut members = vec![
        XarMember {
            extracted_sha1: sha1_hex(&package_info_bytes),
            length: package_info_bytes.len() as u64,
            archived: package_info_bytes,
            name: "PackageInfo".to_string(),
            encoding: Some("application/octet-stream"),
        },
        XarMember {
            name: "Payload".to_string(),
            length: cpio.len() as u64,
            extracted_sha1: sha1_hex(&cpio),
            archived: payload_gz,
            encoding: Some("application/x-gzip"),
        },
    ];
    if !scripts.is_empty() {
        let scripts_cpio = scripts_cpio(scripts);
        members.push(XarMember {
            extracted_sha1: sha1_hex(&scripts_cpio),
            length: scripts_cpio.len() as u64,
            archived: scripts_cpio,
            name: "Scripts".to_string(),
            encoding: Some("application/octet-stream"),
        });
    }

    write_xar(out, &members, mtime)
}

fn render_package_info(
    info: &OsxPackageInfo<'_>,
    file_count: u64,
    installed_kbytes: u64,
) -> String {
    let mut scripts = String::new();
    if !info.scripts.is_empty() {
        scripts.push_str("  <scripts>\n");
        for s in info.scripts {
            scripts.push_str(&format!("    <{s} file=\"./{s}\"/>\n"));
        }
        scripts.push_str("  </scripts>\n");
    }
    let install_location = if info.install_location.is_empty() {
        "/"
    } else {
        info.install_location
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <pkg-info format-version=\"2\" identifier=\"{id}\" version=\"{version}\" \
         install-location=\"{loc}\" auth=\"root\" postinstall-action=\"{action}\">\n\
         \x20 <payload numberOfFiles=\"{files}\" installKBytes=\"{kbytes}\"/>\n\
         {scripts}</pkg-info>\n",
        id = xml_escape(info.identifier),
        version = xml_escape(info.version),
        loc = xml_escape(install_location),
        action = xml_escape(info.postinstall_action),
        files = file_count,
        kbytes = installed_kbytes,
    )
}

/// Walk `root`, producing a newc `cpio` archive with `./`-prefixed relative
/// paths. Returns (cpio bytes, file count, installed KiB).
fn payload_cpio(root: &Path, mtime: i64) -> Result<(Vec<u8>, u64, u64)> {
    let mut entries: Vec<CpioEntry> = Vec::new();
    collect_cpio(root, root, &mut entries, mtime.max(0) as u32)?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = Vec::new();
    let mut files = 0u64;
    let mut bytes = 0u64;
    for (idx, e) in entries.iter().enumerate() {
        let mode = if e.is_dir {
            0o040755
        } else if e.is_symlink {
            0o120777
        } else {
            0o100000 | (e.mode & 0o7777)
        };
        // newc stores a symlink's target in its data section.
        let data: &[u8] = if e.is_symlink {
            e.link.as_bytes()
        } else {
            e.data.as_deref().unwrap_or(&[])
        };
        write_cpio_entry(&mut out, (idx + 1) as u32, mode, e.mtime, &e.name, data);
        if !e.is_dir {
            files += 1;
            bytes += data.len() as u64;
        }
    }
    write_cpio_entry(&mut out, 0, 0, 0, "TRAILER!!!", &[]);
    Ok((out, files, bytes.div_ceil(1024)))
}

struct CpioEntry {
    name: String,
    is_dir: bool,
    is_symlink: bool,
    mode: u32,
    mtime: u32,
    link: String,
    data: Option<Vec<u8>>,
}

fn collect_cpio(root: &Path, dir: &Path, out: &mut Vec<CpioEntry>, mtime: u32) -> Result<()> {
    let mut children: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let path = child.path();
        let rel = path.strip_prefix(root).expect("walked path under root");
        let name = format!("./{}", rel.to_string_lossy());
        let meta = std::fs::symlink_metadata(&path)?;
        let ft = meta.file_type();
        if ft.is_symlink() {
            let target = std::fs::read_link(&path)?;
            out.push(CpioEntry {
                name,
                is_dir: false,
                is_symlink: true,
                mode: 0o777,
                mtime,
                link: target.to_string_lossy().to_string(),
                data: None,
            });
        } else if ft.is_dir() {
            out.push(CpioEntry {
                name: name.clone(),
                is_dir: true,
                is_symlink: false,
                mode: 0o755,
                mtime,
                link: String::new(),
                data: None,
            });
            collect_cpio(root, &path, out, mtime)?;
        } else if ft.is_file() {
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o7777
            };
            out.push(CpioEntry {
                name,
                is_dir: false,
                is_symlink: false,
                mode,
                mtime,
                link: String::new(),
                data: Some(std::fs::read(&path)?),
            });
        }
    }
    Ok(())
}

/// `Scripts` is a cpio archive with top-level, executable script entries.
fn scripts_cpio(scripts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, (name, body)) in scripts.iter().enumerate() {
        write_cpio_entry(&mut out, (i + 1) as u32, 0o100755, 0, name, body);
    }
    write_cpio_entry(&mut out, 0, 0, 0, "TRAILER!!!", &[]);
    out
}

/// Write one newc cpio entry (110-byte ASCII header + name + 4-byte-aligned
/// data).
fn write_cpio_entry(out: &mut Vec<u8>, ino: u32, mode: u32, mtime: u32, name: &str, data: &[u8]) {
    let namesize = name.len() as u32 + 1;
    let header = format!(
        "070701{ino:08x}{mode:08x}{uid:08x}{gid:08x}{nlink:08x}{mtime:08x}\
         {filesize:08x}{devmajor:08x}{devminor:08x}{rdevmajor:08x}{rdevminor:08x}\
         {namesize:08x}{check:08x}",
        uid = 0,
        gid = 0,
        nlink = 1,
        filesize = data.len() as u32,
        devmajor = 0,
        devminor = 0,
        rdevmajor = 0,
        rdevminor = 0,
        check = 0,
    );
    debug_assert_eq!(header.len(), 110, "newc header must be 110 bytes");
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    cpio_pad(out);
    out.extend_from_slice(data);
    cpio_pad(out);
}

fn cpio_pad(out: &mut Vec<u8>) {
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

// ---------------------------------------------------------------------------
// XAR container
// ---------------------------------------------------------------------------

fn write_xar(out: &Path, members: &[XarMember], mtime: i64) -> Result<()> {
    // XAR timestamps count seconds since 2001-01-01 (Apple epoch).
    let creation = mtime.max(0) - 978_307_200;

    // Heap: members laid out back to back, then the archive checksum (SHA-256
    // of the compressed TOC). Each data's <offset>/<size> points into the heap.
    let mut heap = Vec::new();
    let mut offsets = Vec::with_capacity(members.len());
    for m in members {
        offsets.push(heap.len() as u64);
        heap.extend_from_slice(&m.archived);
    }
    let checksum_offset = heap.len() as u64;
    const CHECKSUM_SIZE: u64 = 32;

    let mut toc = format!(
        "<xar><toc><creation-time>{creation}</creation-time>\
         <checksum style=\"sha256\"><offset>{checksum_offset}</offset>\
         <size>{CHECKSUM_SIZE}</size></checksum>"
    );
    for (i, m) in members.iter().enumerate() {
        toc.push_str(&format!(
            "<file id=\"{id}\"><name>{name}</name><type>file</type><data>\
             <length>{stored}</length><offset>{offset}</offset><size>{extracted}</size>",
            id = i + 1,
            name = xml_escape(&m.name),
            // xar's `<length>` is the archived (stored) size; `<size>` is the
            // extracted (uncompressed) size.
            stored = m.archived.len(),
            offset = offsets[i],
            extracted = m.length,
        ));
        if let Some(enc) = m.encoding {
            toc.push_str(&format!("<encoding style=\"{enc}\"/>"));
        }
        toc.push_str(&format!(
            "<extracted-checksum style=\"sha1\">{}</extracted-checksum>\
             <archived-checksum style=\"sha1\">{}</archived-checksum>",
            m.extracted_sha1,
            sha1_hex(&m.archived),
        ));
        toc.push_str("</data></file>");
    }
    toc.push_str("</toc></xar>");

    let mut enc = ZlibEncoder::new(Vec::new(), GzLevel::default());
    enc.write_all(toc.as_bytes())?;
    let toc_z = enc.finish()?;

    // The checksum is the SHA-256 of the *compressed* TOC bytes (this is what
    // `apple-xar`/`xar` verify and what the RSA signature later signs).
    let checksum = Sha256::digest(&toc_z).to_vec();
    debug_assert_eq!(checksum.len() as u64, CHECKSUM_SIZE);
    heap.extend_from_slice(&checksum);

    // Header (28 bytes, big-endian): magic, size, version, compressed toc
    // length, uncompressed toc length, checksum algorithm (3 = SHA-256).
    let mut bytes = Vec::with_capacity(28 + toc_z.len() + heap.len());
    bytes.extend_from_slice(b"xar!");
    bytes.extend_from_slice(&(28u16).to_be_bytes());
    bytes.extend_from_slice(&(1u16).to_be_bytes());
    bytes.extend_from_slice(&(toc_z.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&(toc.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&(3u32).to_be_bytes());
    bytes.extend_from_slice(&toc_z);
    bytes.extend_from_slice(&heap);
    std::fs::write(out, bytes).with_context(|| format!("failed to write '{}'", out.display()))?;
    Ok(())
}

fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), GzLevel::best());
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

fn sha1_hex(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(data);
    hex::encode(h.finalize())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
