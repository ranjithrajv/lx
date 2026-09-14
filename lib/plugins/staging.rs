// SPDX-License-Identifier: GPL-3.0-or-later

//! Format-agnostic staging engine: place a release payload under the staging
//! root (flat/bundle layouts, ELF exec, man/license ancillary files) and the
//! shared filesystem helpers the contents overlay builds on.

use anyhow::Context;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::config::PackageConfig;

/// Stage the package's installed filesystem tree under `root`.
///
/// Mirrors the old `build.rs::stage_install_tree` — format-agnostic file
/// placement logic that every plugin reuses. Handles `bundle` vs flat mode,
/// `binary_rename`, and ancillary file (man page / license) staging.
///
/// When `cfg.prefix` is non-empty (set via `--prefix` for `--from-dir`/
/// `--from-file` builds), all files are staged flat under `<root><prefix>`
/// instead of the default `/usr/bin` layout — a simple "put these files
/// here" mode for fpm-style "you supply files" builds.
pub fn stage_install_tree(
    cfg: &PackageConfig,
    binary_dir: &Path,
    root: &Path,
    mtime: i64,
) -> anyhow::Result<()> {
    // Custom-prefix mode (fpm-style): dump files at the given absolute path
    // inside the package, with no ELF detection or ancillary staging.
    if !cfg.prefix.is_empty() {
        let dest_dir = root.join(cfg.prefix.trim_start_matches('/'));
        std::fs::create_dir_all(&dest_dir)?;
        copy_dir_recursive(binary_dir, &dest_dir)?;
        return Ok(());
    }

    let usr_bin = root.join("usr").join("bin");
    std::fs::create_dir_all(&usr_bin)?;

    if cfg.bundle {
        let lib_dir = root.join("usr").join("lib").join(&cfg.package_name);
        copy_dir_recursive(binary_dir, &lib_dir)?;

        let bin_subdir = lib_dir.join("bin");
        if bin_subdir.is_dir() {
            symlink_elf_executables(
                &bin_subdir,
                &usr_bin,
                &format!("/usr/lib/{}/bin", cfg.package_name),
            )?;
        }
        symlink_elf_executables(
            &lib_dir,
            &usr_bin,
            &format!("/usr/lib/{}", cfg.package_name),
        )?;
    } else {
        let umask = cfg.effective_umask();
        for entry in std::fs::read_dir(binary_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            if crate::filemeta::is_elf(&path)? {
                let dest = usr_bin.join(entry.file_name());
                std::fs::copy(&path, &dest)?;
                make_executable(&dest)?;
                apply_umask(&dest, umask)?;
            } else {
                stage_ancillary_file(&path, root, &cfg.package_name, mtime, umask)?;
            }
        }
    }

    if !cfg.binary_rename.is_empty() {
        apply_binary_rename(&usr_bin, &cfg.binary_rename)?;
    }

    if std::fs::read_dir(&usr_bin)?.next().is_none() {
        anyhow::bail!(
            "no executables landed in /usr/bin ({}) - check binary_path/bundle config",
            if cfg.bundle {
                "bundle=true"
            } else {
                "flat mode"
            }
        );
    }

    // Make binaries relocatable (RPATH = $ORIGIN/../lib).
    crate::relocatable::make_relocatable(&usr_bin)?;

    // Detect binary dependencies via ELF analysis + distro package lookup.
    let _detected_deps = crate::bindep::detect_binary_deps(root).unwrap_or_default();

    Ok(())
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(&target, &dest_path)?;
        } else if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

fn symlink_elf_executables(src_dir: &Path, usr_bin: &Path, abs_prefix: &str) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file() && crate::filemeta::is_elf(&path)? {
            make_executable(&path)?;
            let name = entry.file_name();
            let target = format!("{abs_prefix}/{}", name.to_string_lossy());
            std::os::unix::fs::symlink(&target, usr_bin.join(&name))?;
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> anyhow::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | lx_lib::constants::EXEC_MODE_MASK);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// Apply umask to a file's permissions: `mode = mode & !umask`.
/// When `umask` is `None`, the file's mode is left unchanged.
fn apply_umask(path: &Path, umask: Option<u32>) -> anyhow::Result<()> {
    let Some(mask) = umask else {
        return Ok(());
    };
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() & !mask);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// Returns true if `s` contains glob meta-characters (`*`, `?`, `[`).
pub(super) fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

fn stage_ancillary_file(
    path: &Path,
    root: &Path,
    pkg_name: &str,
    mtime: i64,
    umask: Option<u32>,
) -> anyhow::Result<()> {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return Ok(()),
    };
    if let Some(section) = man_section(name) {
        let man_dir = root.join("usr/share/man").join(format!("man{section}"));
        std::fs::create_dir_all(&man_dir)?;
        if name.ends_with(".gz") {
            let dest = man_dir.join(name);
            std::fs::copy(path, &dest)?;
            apply_umask(&dest, umask)?;
        } else {
            let dest = man_dir.join(format!("{name}.gz"));
            gzip_file_to(path, &dest, mtime)?;
            apply_umask(&dest, umask)?;
        }
    } else if is_license_like(name) {
        let doc_dir = root.join("usr/share/doc").join(pkg_name);
        std::fs::create_dir_all(&doc_dir)?;
        let dest = doc_dir.join(name);
        std::fs::copy(path, &dest)?;
        apply_umask(&dest, umask)?;
    }
    Ok(())
}

pub fn man_section(name: &str) -> Option<u8> {
    let stem = name.strip_suffix(".gz").unwrap_or(name);
    let ext = Path::new(stem).extension()?.to_str()?;
    if ext.len() == 1 {
        let c = ext.as_bytes()[0];
        if c.is_ascii_digit() && c != b'0' {
            return Some(c - b'0');
        }
    }
    None
}

pub fn is_license_like(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("LICENSE") || upper.starts_with("COPYING") || upper.starts_with("NOTICE")
}

fn gzip_file_to(src: &Path, dest: &Path, mtime: i64) -> anyhow::Result<()> {
    use std::io::Write;
    let data = std::fs::read(src)?;
    let mut out = std::fs::File::create(dest)?;
    // Same deterministic treatment as the tar members (fixed mtime header)
    // via debarchive's shared helper.
    let gz = lx_lib::debarchive::deterministic_gzip_bytes(&data, mtime, 9)?;
    out.write_all(&gz)?;
    Ok(())
}

fn apply_binary_rename(usr_bin: &Path, rename: &str) -> anyhow::Result<()> {
    let files: Vec<_> = std::fs::read_dir(usr_bin)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .collect();
    if files.len() == 1 {
        std::fs::rename(files[0].path(), usr_bin.join(rename))?;
    }
    Ok(())
}

pub(super) fn stage_file(src: &Path, dst: &Path, umask: Option<u32>) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)
        .map(|_| ())
        .with_context(|| format!("failed to copy '{}' to '{}'", src.display(), dst.display()))?;
    apply_umask(dst, umask)?;
    Ok(())
}

/// Join an absolute installed path onto the staging root, rejecting any
/// attempt to escape it (`..` components).
pub fn safe_join(root: &Path, installed_abs: &str) -> anyhow::Result<std::path::PathBuf> {
    if !installed_abs.starts_with('/') {
        anyhow::bail!("contents dst must be absolute: '{installed_abs}'");
    }
    let rel = installed_abs.trim_start_matches('/');
    let joined = root.join(rel);
    if joined.components().any(|c| c.as_os_str() == "..") {
        anyhow::bail!("contents dst escapes the package root: '{installed_abs}'");
    }
    Ok(joined)
}
