//! Build a `.deb` archive entirely in-process -- no `dpkg-deb`, no Docker,
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
    let mut md5sums = String::new();
    let data_tar_gz =
        build_data_tar_gz(root, mtime, &mut md5sums).context("failed to build data.tar.gz")?;
    let control_tar_gz = build_control_tar_gz(control, md5sums.as_bytes(), mtime)
        .context("failed to build control.tar.gz")?;
    write_ar(deb_path, mtime, &control_tar_gz, &data_tar_gz)
        .context("failed to write .deb ar container")
}

/// Recursively tar+gzip `root`'s contents (sorted, normalized ownership
/// and mtime), accumulating an md5sums listing as it goes.
fn build_data_tar_gz(root: &Path, mtime: i64, md5sums: &mut String) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_dir_sorted(&mut builder, root, "./", root, mtime, Some(md5sums))?;
        builder.finish()?;
    }
    gzip(&tar_bytes, mtime)
}

/// Tar+gzip `fs_root`'s entire subtree (sorted, normalized ownership and
/// mtime), with every entry's archive path prefixed by `archive_prefix`
/// (e.g. `"eza-0.23.5/usr/"` so `fs_root/bin/eza` becomes
/// `"eza-0.23.5/usr/bin/eza"` in the archive -- the upstream/Debian source
/// tarball convention, not `data.tar`'s `"./"`-relative one). Used by
/// `source.rs` for a generated source package's `.orig.tar.gz` and
/// `.debian.tar.gz` members.
pub fn tar_gz_tree(fs_root: &Path, archive_prefix: &str, mtime: i64) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_dir_sorted(&mut builder, fs_root, archive_prefix, fs_root, mtime, None)?;
        builder.finish()?;
    }
    gzip(&tar_bytes, mtime)
}

/// Extract a `.deb`'s `data.tar(.gz)` payload into `dest` (the inverse of
/// `build`, minus the control member). Scoped to what `lpt` itself
/// produces -- gzip or uncompressed `data.tar` -- not a general
/// dpkg-deb-compatible reader for arbitrary `.deb`s built by other tools
/// (which may use xz/zstd/bzip2); the only caller extracts a `.deb`
/// `lpt build` itself just built moments earlier.
pub fn extract(deb_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(deb_path)
        .with_context(|| format!("failed to open '{}'", deb_path.display()))?;
    let mut archive = ar::Archive::new(file);
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry?;
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if name == "data.tar.gz" {
            std::fs::create_dir_all(dest)?;
            let gz = flate2::read::GzDecoder::new(&mut entry);
            return tar::Archive::new(gz).unpack(dest).with_context(|| {
                format!("failed to unpack data.tar.gz from '{}'", deb_path.display())
            });
        }
        if name == "data.tar" {
            std::fs::create_dir_all(dest)?;
            return tar::Archive::new(&mut entry).unpack(dest).with_context(|| {
                format!("failed to unpack data.tar from '{}'", deb_path.display())
            });
        }
    }
    bail!(
        "'{}' has no data.tar(.gz) member (only .deb files with a gzip or \
         uncompressed data.tar are supported)",
        deb_path.display()
    );
}

/// Tar+gzip the control member set (just `control` + `md5sums` today --
/// no maintainer scripts or conffiles are needed for a repackaged upstream
/// binary).
fn build_control_tar_gz(control: &[u8], md5sums: &[u8], mtime: i64) -> Result<Vec<u8>> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        append_file_bytes(&mut builder, "./control", control, 0o644, mtime)?;
        append_file_bytes(&mut builder, "./md5sums", md5sums, 0o644, mtime)?;
        builder.finish()?;
    }
    gzip(&tar_bytes, mtime)
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
            header.set_mode(0o777);
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
            header.set_mode(0o755);
            header.set_mtime(mtime.max(0) as u64);
            header.set_uid(0);
            header.set_gid(0);
            header.set_path(format!("{archive_path}/"))?;
            header.set_cksum();
            builder.append(&header, std::io::empty())?;
            append_dir_sorted(
                builder,
                original_root,
                archive_prefix,
                &fs_path,
                mtime,
                md5sums.as_deref_mut(),
            )?;
        } else {
            let content = std::fs::read(&fs_path)
                .with_context(|| format!("failed to read '{}'", fs_path.display()))?;
            let mode = std::fs::metadata(&fs_path)?.permissions().mode() & 0o777;
            if let Some(sums) = md5sums.as_deref_mut() {
                let digest = md5::compute(&content);
                sums.push_str(&format!("{digest:x}  {}\n", rel.to_string_lossy()));
            }
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

/// Deterministic gzip: fixed mtime in the header (no wall-clock leak), no
/// embedded filename/comment/OS-specific fields left to vary.
fn gzip(data: &[u8], mtime: i64) -> Result<Vec<u8>> {
    let mut encoder = flate2::GzBuilder::new()
        .mtime(mtime.max(0) as u32)
        .write(Vec::new(), flate2::Compression::best());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

/// Write the outer `ar` container: `debian-binary`, `control.tar.gz`,
/// `data.tar.gz`, in that order (dpkg requires this exact order and reads
/// only as much of the archive as it needs, so anything after `data.tar.*`
/// -- e.g. a detached signature -- is safe to append later if ever
/// needed).
fn write_ar(deb_path: &Path, mtime: i64, control_tar_gz: &[u8], data_tar_gz: &[u8]) -> Result<()> {
    let file = std::fs::File::create(deb_path)
        .with_context(|| format!("failed to create '{}'", deb_path.display()))?;
    let mut builder = ar::Builder::new(file);
    let members: [(&str, &[u8]); 3] = [
        ("debian-binary", b"2.0\n"),
        ("control.tar.gz", control_tar_gz),
        ("data.tar.gz", data_tar_gz),
    ];
    for (name, data) in members {
        let mut header = ar::Header::new(name.as_bytes().to_vec(), data.len() as u64);
        header.set_mtime(mtime.max(0) as u64);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mode(0o100644);
        builder.append(&header, data)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_round_trips_build() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
        let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

        let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        build(root.path(), b"Package: hello\n", 0, &deb_path).unwrap();

        let dest = tempfile::tempdir().unwrap();
        extract(&deb_path, dest.path()).unwrap();

        assert_eq!(
            std::fs::read(dest.path().join("usr/bin/hello")).unwrap(),
            b"payload"
        );
        let mode = std::fs::metadata(dest.path().join("usr/bin/hello"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "executable bit lost on round-trip");
    }

    #[test]
    fn extract_rejects_deb_with_no_data_tar() {
        // An ar archive with only debian-binary -- no data.tar(.gz) member.
        let path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let file = std::fs::File::create(&path).unwrap();
        let mut builder = ar::Builder::new(file);
        let header = ar::Header::new(b"debian-binary".to_vec(), 4);
        builder.append(&header, &b"2.0\n"[..]).unwrap();
        drop(builder);

        let dest = tempfile::tempdir().unwrap();
        let err = extract(&path, dest.path()).unwrap_err();
        assert!(err.to_string().contains("no data.tar"));
    }

    #[test]
    fn tar_gz_tree_uses_the_given_prefix_not_dot_slash() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("bin")).unwrap();
        std::fs::write(root.path().join("bin/eza"), b"elf").unwrap();

        let bytes = tar_gz_tree(root.path(), "eza-0.23.5/usr/", 0).unwrap();
        let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
        let mut archive = tar::Archive::new(decoder);
        let names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            names.contains(&"eza-0.23.5/usr/bin/eza".to_string()),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.starts_with("./")), "{names:?}");
    }

    /// The real `dpkg-deb` on this host (not our own reader) must accept
    /// the archive, report the right control fields, and list the exact
    /// files we staged -- proof this isn't just internally self-consistent
    /// but actually a valid Debian package. Skips gracefully if dpkg-deb
    /// isn't installed (this crate has no runtime dependency on it).
    #[test]
    fn real_dpkg_deb_accepts_the_archive() {
        let Ok(check) = std::process::Command::new("dpkg-deb")
            .arg("--version")
            .output()
        else {
            eprintln!("skipping: dpkg-deb not on PATH");
            return;
        };
        if !check.status.success() {
            eprintln!("skipping: dpkg-deb not usable");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf-payload").unwrap();
        let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

        let deb_path = root.path().join("hello.deb");
        let control =
            b"Package: hello\nVersion: 1.0-1\nArchitecture: amd64\nMaintainer: t <t@example.com>\nDescription: test\n";
        build(root.path(), control, 1_735_689_600, &deb_path).unwrap();

        let info = std::process::Command::new("dpkg-deb")
            .args(["--info", deb_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            info.status.success(),
            "dpkg-deb --info failed: {}",
            String::from_utf8_lossy(&info.stderr)
        );
        let info_text = String::from_utf8_lossy(&info.stdout);
        assert!(info_text.contains("Package: hello"));
        assert!(info_text.contains("Version: 1.0-1"));

        let contents = std::process::Command::new("dpkg-deb")
            .args(["--contents", deb_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(contents.status.success());
        let contents_text = String::from_utf8_lossy(&contents.stdout);
        // dpkg-deb's own `--build` stores paths with a "./" prefix (and an
        // explicit "./" root entry); the `tar` crate strips leading "./"
        // path components with no raw-bytes escape hatch to preserve them
        // (verified against its source), so ours comes out as bare
        // "usr/bin/hello". Confirmed with a real dpkg-deb-built reference
        // package that this is still fully valid and installable -- dpkg
        // itself doesn't require the "./" convention, just relative paths.
        assert!(contents_text.contains("usr/bin/hello"));
        assert!(
            contents_text.contains("-rwxr-xr-x"),
            "executable bit not preserved: {contents_text}"
        );
    }

    /// A real .deb produced by `dpkg-deb --build` starts with the ar magic
    /// "!<arch>\n" and its first member is always "debian-binary".
    #[test]
    fn produces_a_valid_ar_container_with_debian_binary_first() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
        let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();

        build(root.path(), b"Package: hello\n", 1_735_689_600, &deb_path).unwrap();

        let bytes = std::fs::read(&deb_path).unwrap();
        assert!(bytes.starts_with(b"!<arch>\n"));

        let mut archive = ar::Archive::new(bytes.as_slice());
        let first = archive.next_entry().unwrap().unwrap();
        assert_eq!(first.header().identifier(), b"debian-binary");
    }

    #[test]
    fn same_input_produces_byte_identical_output() {
        let build_once = || {
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
            std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
            let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
            build(root.path(), b"Package: hello\n", 1_735_689_600, &deb_path).unwrap();
            std::fs::read(&deb_path).unwrap()
        };
        assert_eq!(build_once(), build_once());
    }

    #[test]
    fn md5sums_lists_every_regular_file_without_leading_dot_slash() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
        let mut md5sums = String::new();
        build_data_tar_gz(root.path(), 0, &mut md5sums).unwrap();
        let expected = format!("{:x}", md5::compute(b"payload"));
        assert_eq!(md5sums, format!("{expected}  usr/bin/hello\n"));
    }

    #[test]
    fn preserves_symlinks() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/lib/pkg/bin")).unwrap();
        std::fs::write(root.path().join("usr/lib/pkg/bin/real"), b"elf").unwrap();
        std::os::unix::fs::symlink("../lib/pkg/bin/real", root.path().join("usr/bin_link"))
            .unwrap();
        // Just confirm the walk doesn't error on a symlink and still
        // reaches the real file for md5sums.
        let mut md5sums = String::new();
        build_data_tar_gz(root.path(), 0, &mut md5sums).unwrap();
        assert!(md5sums.contains("usr/lib/pkg/bin/real"));
    }
}
