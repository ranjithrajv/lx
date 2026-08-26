//! Plugin architecture for `lpt`.
//!
//! Each package format (`.deb`, `.rpm`, …) is a plugin implementing the
//! [`Plugin`] trait. The build pipeline is format-agnostic: it resolves
//! assets, stages the install tree via shared helpers, then delegates the
//! actual archive creation to the selected plugin.
//!
//! Registration is static and explicit — no dynamic loading — so adding a new
//! format is just implementing `Plugin` and registering it in
//! [`registry`] / [`all_plugins`].

pub mod arch;
pub mod deb;
pub mod rpm;
pub mod source;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;
use lpt_lib::github::RepoLicense;

/// Context passed to a plugin's build method. Contains everything the plugin
/// needs to render its control/spec metadata and archive the staged tree.
pub struct BuildContext<'a> {
    /// The full package config (package_name, github_repo, epoch, etc.).
    pub cfg: &'a PackageConfig,
    /// Resolved job (dist, arch, asset, tag, published_at).
    pub job: &'a crate::build::ResolvedJob,
    /// Directory containing the extracted binary tree to stage (binary_dir).
    pub binary_dir: &'a Path,
    /// Staging root that the plugin should populate (e.g. `root/usr/bin/...`).
    /// Already created; plugin stages files under it then archives it.
    pub staging_root: &'a Path,
    /// Detected upstream license, if any.
    pub license: Option<&'a RepoLicense>,
    /// Debian-style version string (upstream prefix stripped).
    pub debian_version: &'a str,
    /// Build revision (e.g. "1").
    pub build_version: &'a str,
    /// Reproducible mtime (SOURCE_DATE_EPOCH or published_at).
    pub mtime: i64,
    /// Signing key file for formats that embed signatures natively
    /// (rpm). `None` when signing is disabled. deb signs post-build
    /// instead (detached `.sig`), so it ignores this.
    pub sign_key: Option<&'a Path>,
    /// Passphrase for the embedded-signature key (rpm), resolved from env.
    pub sign_passphrase: Option<&'a str>,
}

/// A package-format plugin.
///
/// Implementors are stateless; they are registered once in [`registry`].
pub trait Plugin: Send + Sync {
    /// Canonical name used in `package.yaml` (`package_format`) and
    /// `--format` (`deb` / `rpm`).
    fn name(&self) -> &'static str;

    /// File extension without dot (e.g. `"deb"`, `"rpm"`).
    fn file_extension(&self) -> &'static str;

    /// Human-readable description.
    fn description(&self) -> &'static str;

    /// Default distributions when `debian_distributions` is empty and the
    /// plugin is selected. For `deb` this is Debian suites; for `rpm` this
    /// is RPM-based distros.
    // Exercised by `default_distributions_differ_by_format` below; not yet
    // called by the live distribution-resolution path, which still reads
    // the DEFAULT_*_DISTRIBUTIONS constants directly (see config.rs).
    #[allow(dead_code)]
    fn default_distributions(&self) -> &'static [&'static str];

    /// Whether an architecture is supported for a given distribution for this
    /// plugin. Mirrors `PackageConfig::arch_supported_for_dist` for deb and
    /// is permissive for rpm (which is arch-independent in our matrix).
    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool;

    /// Build a single package archive.
    ///
    /// The plugin stages the install tree under `ctx.staging_root` (using
    /// shared helpers like [`stage_install_tree`]) and then creates the
    /// archive at the returned `PathBuf` (typically inside a temp dir).
    fn build(&self, ctx: &BuildContext) -> Result<PathBuf>;
}

/// All known plugins, in registration order.
pub fn all_plugins() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(deb::DebPlugin),
        Box::new(rpm::RpmPlugin),
        Box::new(arch::ArchPlugin),
    ]
}

/// Look up a plugin by name (case-insensitive). Returns `None` for unknown.
pub fn get_plugin(name: &str) -> Option<Box<dyn Plugin>> {
    let lower = name.to_ascii_lowercase();
    all_plugins().into_iter().find(|p| p.name() == lower)
}

/// Available plugin names for error messages / help text.
pub fn available_names() -> Vec<&'static str> {
    all_plugins().iter().map(|p| p.name()).collect()
}

// ---------------------------------------------------------------------------
// Shared staging helpers (format-agnostic, used by all plugins)
// ---------------------------------------------------------------------------

/// Stage the package's installed filesystem tree under `root`.
///
/// Mirrors the old `build.rs::stage_install_tree` — format-agnostic file
/// placement logic that every plugin reuses. Handles `bundle` vs flat mode,
/// `binary_rename`, and ancillary file (man page / license) staging.
pub fn stage_install_tree(
    cfg: &PackageConfig,
    binary_dir: &Path,
    root: &Path,
    mtime: i64,
) -> anyhow::Result<()> {
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
        for entry in std::fs::read_dir(binary_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            if is_elf(&path)? {
                let dest = usr_bin.join(entry.file_name());
                std::fs::copy(&path, &dest)?;
                make_executable(&dest)?;
            } else {
                stage_ancillary_file(&path, root, &cfg.package_name, mtime)?;
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
        if entry.file_type()?.is_file() && is_elf(&path)? {
            make_executable(&path)?;
            let name = entry.file_name();
            let target = format!("{abs_prefix}/{}", name.to_string_lossy());
            std::os::unix::fs::symlink(&target, usr_bin.join(&name))?;
        }
    }
    Ok(())
}

pub(crate) fn is_elf(path: &Path) -> anyhow::Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(&magic == b"\x7fELF")
}

fn make_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | lpt_lib::constants::EXEC_MODE_MASK);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

fn stage_ancillary_file(
    path: &Path,
    root: &Path,
    pkg_name: &str,
    mtime: i64,
) -> anyhow::Result<()> {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return Ok(()),
    };
    if let Some(section) = man_section(name) {
        let man_dir = root.join("usr/share/man").join(format!("man{section}"));
        std::fs::create_dir_all(&man_dir)?;
        if name.ends_with(".gz") {
            std::fs::copy(path, man_dir.join(name))?;
        } else {
            gzip_file_to(path, &man_dir.join(format!("{name}.gz")), mtime)?;
        }
    } else if is_license_like(name) {
        let doc_dir = root.join("usr/share/doc").join(pkg_name);
        std::fs::create_dir_all(&doc_dir)?;
        std::fs::copy(path, doc_dir.join(name))?;
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
    let gz = lpt_lib::debarchive::deterministic_gzip_bytes(&data, mtime, 9)?;
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

// ---------------------------------------------------------------------------
// contents / scripts / conffiles (nfpm-parity staging overlays)
// ---------------------------------------------------------------------------

/// Stage `contents:` entries into the staged tree, after the release
/// payload. `src` paths resolve against the current working directory
/// (the build environment); `dst` is an absolute installed path under
/// `root`.
///
/// Returns the absolute installed paths registered as deb conffiles —
/// entries whose type is `config`, `config|noreplace`, or
/// `config|missingok`. Other formats ignore the return value.
pub fn apply_contents(cfg: &PackageConfig, root: &Path) -> anyhow::Result<Vec<String>> {
    let mut conffiles = Vec::new();
    for entry in &cfg.contents {
        let dst_abs = safe_join(root, &entry.dst)?;
        match entry.kind.as_str() {
            "" | "file" => {
                stage_file(Path::new(&entry.src), &dst_abs)?;
            }
            "config" | "config|noreplace" | "config|missingok" => {
                stage_file(Path::new(&entry.src), &dst_abs)?;
                conffiles.push(entry.dst.clone());
            }
            "tree" => {
                copy_dir_recursive(Path::new(&entry.src), &dst_abs)?;
            }
            // nfpm symlink semantics: both src and dst are paths *inside*
            // the package; nothing is read from the build environment.
            "symlink" => {
                if let Some(parent) = dst_abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let _ = std::fs::remove_file(&dst_abs);
                #[cfg(unix)]
                std::os::unix::fs::symlink(&entry.src, &dst_abs)?;
            }
            "dir" => {
                std::fs::create_dir_all(&dst_abs)?;
            }
            // RPM-only directive; a no-op for deb/arch (matches nfpm,
            // which ignores ghost files for non-rpm packagers).
            "ghost" => {}
            other => anyhow::bail!(
                "unsupported contents type '{other}' (validate() should have caught this)"
            ),
        }
    }
    conffiles.sort();
    Ok(conffiles)
}

/// Read the configured maintainer scripts from the build environment and
/// render them as deb control members (`preinst`/`postinst`/`prerm`/
/// `postrm`, mode 0755). Empty when none are configured.
pub fn maintainer_script_members(
    cfg: &PackageConfig,
) -> anyhow::Result<Vec<lpt_lib::debarchive::ControlMember>> {
    let pairs = [
        ("preinst", cfg.scripts.preinstall.trim()),
        ("postinst", cfg.scripts.postinstall.trim()),
        ("prerm", cfg.scripts.preremove.trim()),
        ("postrm", cfg.scripts.postremove.trim()),
    ];
    let mut members = Vec::new();
    for (name, path) in pairs {
        if path.is_empty() {
            continue;
        }
        let content = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("failed to read {} script '{}': {e}", name, path))?;
        members.push(lpt_lib::debarchive::ControlMember {
            name: name.to_string(),
            content,
            mode: 0o755,
        });
    }
    Ok(members)
}

fn stage_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)
        .map(|_| ())
        .with_context(|| format!("failed to copy '{}' to '{}'", src.display(), dst.display()))
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

/// The extended-description continuation line shared by every rendered
/// control file (binary `DEBIAN/control`, source `debian/control`). One
/// constant so the wording can't drift between renderers again — it had
/// already split into "upstream GitHub release" vs "the upstream release"
/// variants before this was centralized.
pub(crate) const PACKAGED_FROM_LINE: &str = " Packaged from the upstream release for Debian.";

/// Render sorted extra control fields (skipping core fields already
/// rendered explicitly). Shared by the deb plugin's binary control file
/// and the source package's `debian/control`.
pub(crate) fn render_extra_fields(fields: &std::collections::HashMap<String, String>) -> String {
    if fields.is_empty() {
        return String::new();
    }
    let mut keys: Vec<_> = fields.keys().collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        let v = &fields[k];
        if k.trim().is_empty() || v.trim().is_empty() {
            continue;
        }
        // Avoid duplicating core fields if user accidentally sets them via fields.
        let lower = k.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "package"
                | "version"
                | "architecture"
                | "maintainer"
                | "description"
                | "section"
                | "priority"
                | "homepage"
                | "depends"
                | "recommends"
                | "suggests"
                | "conflicts"
                | "replaces"
                | "provides"
                | "breaks"
                | "pre-depends"
                | "pre_depends"
                | "predepends"
                | "source"
                | "standards-version"
                | "build-depends"
        ) {
            continue;
        }
        out.push_str(&format!("{}: {}\n", k.trim(), v.trim()));
    }
    out
}
