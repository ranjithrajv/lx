// SPDX-License-Identifier: GPL-3.0-or-later

//! `contents:` / scripts / conffiles overlay staging (nfpm parity): stage
//! `contents:` entries, register config files, and render maintainer-script
//! and deb extra control members.

use anyhow::Context;
use std::path::{Path, PathBuf};

use crate::config::{ContentEntry, PackageConfig};
use crate::filemeta::{installed_path, FileMeta, FileMetaMap, RpmFileKind};

use super::BuildContext;

/// Stage `contents:` entries into the staged tree, after the release
/// payload. `src` paths resolve against the current working directory
/// (the build environment); `dst` is an absolute installed path under
/// `root`.
///
/// `format` is the active package format (`deb` / `rpm` / `arch`). Entries
/// with a non-empty `packager` field are skipped unless it matches
/// (case-insensitive), matching nfpm.
///
/// A `contents:` entry staged as a config file. `noreplace` is true for
/// `config|noreplace` (rpm `%config(noreplace)`, pacman backup-preserved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedConfig {
    /// Absolute installed path (e.g. `/etc/foo.conf`).
    pub path: String,
    pub noreplace: bool,
}

/// Returns the absolute installed paths registered as deb conffiles —
/// entries whose type is `config`, `config|noreplace`, or
/// `config|missingok`. Other formats ignore the return value.
pub fn apply_contents(
    cfg: &PackageConfig,
    root: &Path,
    format: &str,
) -> anyhow::Result<Vec<String>> {
    Ok(apply_contents_with_config(cfg, root, format)?
        .into_iter()
        .map(|c| c.path)
        .collect())
}

/// Like [`apply_contents`] but keeps the config kind so rpm can mark
/// `%config(noreplace)` and Arch can build its `backup` list.
///
/// Drops the per-file metadata map; callers that need it (every archive
/// writer, for `file_info` / `disown_subtree`) use [`apply_contents_full`].
pub fn apply_contents_with_config(
    cfg: &PackageConfig,
    root: &Path,
    format: &str,
) -> anyhow::Result<Vec<StagedConfig>> {
    Ok(apply_contents_full(cfg, root, format)?.0)
}

/// Stage every applicable `contents:` entry and return the registered config
/// files plus the per-file metadata overrides (`file_info`, `disown_subtree`)
/// keyed by installed path for the archive writers.
pub fn apply_contents_full(
    cfg: &PackageConfig,
    root: &Path,
    format: &str,
) -> anyhow::Result<(Vec<StagedConfig>, FileMetaMap)> {
    let format = format.trim().to_ascii_lowercase();
    let umask = cfg.effective_umask();
    let mut configs = Vec::new();
    let mut meta = FileMetaMap::new();
    for entry in &cfg.contents {
        let packager = entry.packager.trim().to_ascii_lowercase();
        if !packager.is_empty() && packager != format {
            continue;
        }
        // nfpm's `expand: true` expands `$VAR` / `${VAR}` in src and dst.
        let entry = if entry.expand {
            let mut e = entry.clone();
            e.src = os_expand(&e.src);
            e.dst = os_expand(&e.dst);
            e
        } else {
            entry.clone()
        };
        let dst_abs = super::staging::safe_join(root, &entry.dst)?;
        let staged: Vec<PathBuf> = match entry.kind.as_str() {
            "" | "file" => stage_contents_entry(
                Path::new(&entry.src),
                &dst_abs,
                &entry.kind,
                umask,
                cfg.disable_globbing,
                false,
            )?,
            "config" | "config|noreplace" | "config|missingok" => {
                configs.push(StagedConfig {
                    path: entry.dst.clone(),
                    noreplace: entry.kind == "config|noreplace",
                });
                stage_contents_entry(
                    Path::new(&entry.src),
                    &dst_abs,
                    &entry.kind,
                    umask,
                    cfg.disable_globbing,
                    false,
                )?
            }
            "tree" => stage_contents_entry(
                Path::new(&entry.src),
                &dst_abs,
                &entry.kind,
                umask,
                cfg.disable_globbing,
                true,
            )?,
            // nfpm's "config tree" types: copy the tree, then register every
            // regular file as a config file.
            "config|tree" | "config|noreplace|tree" | "config|missingok|tree" => {
                let staged = stage_contents_entry(
                    Path::new(&entry.src),
                    &dst_abs,
                    "tree",
                    umask,
                    cfg.disable_globbing,
                    true,
                )?;
                let noreplace = entry.kind == "config|noreplace|tree";
                for p in &staged {
                    for f in walk_paths(p)? {
                        if f.is_file() {
                            configs.push(StagedConfig {
                                path: installed_for(root, &f)?,
                                noreplace,
                            });
                        }
                    }
                }
                staged
            }
            // RPM doc/license/readme classification; a plain file elsewhere.
            "doc" | "license" | "licence" | "readme" => stage_contents_entry(
                Path::new(&entry.src),
                &dst_abs,
                &entry.kind,
                umask,
                cfg.disable_globbing,
                false,
            )?,
            // nfpm symlink semantics: both src and dst are paths *inside*
            // the package; nothing is read from the build environment.
            "symlink" => {
                if let Some(parent) = dst_abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let _ = std::fs::remove_file(&dst_abs);
                #[cfg(unix)]
                std::os::unix::fs::symlink(&entry.src, &dst_abs)?;
                vec![dst_abs.clone()]
            }
            "dir" => {
                std::fs::create_dir_all(&dst_abs)?;
                vec![dst_abs.clone()]
            }
            // RPM-only directive; a no-op for deb/arch (matches nfpm,
            // which ignores ghost files for non-rpm packagers).
            "ghost" => Vec::new(),
            other => anyhow::bail!(
                "unsupported contents type '{other}' (validate() should have caught this)"
            ),
        };
        record_entry_meta(&mut meta, root, &entry, &staged)?;
    }
    configs.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((configs, meta))
}

/// Record the `file_info` / `disown_subtree` overrides for one entry's staged
/// paths. `tree` and `dir` entries apply to their whole subtree; a symlink
/// carries ownership but never a mode (matching nfpm).
fn record_entry_meta(
    meta: &mut FileMetaMap,
    root: &Path,
    entry: &ContentEntry,
    staged: &[PathBuf],
) -> anyhow::Result<()> {
    if staged.is_empty() {
        return Ok(());
    }
    let base = FileMeta {
        mode: entry.file_info.parsed_mode()?,
        owner: non_empty(&entry.file_info.owner),
        group: non_empty(&entry.file_info.group),
        mtime: entry.file_info.parsed_mtime()?,
        lang: non_empty(&entry.file_info.lang),
        disown: false,
        rpm_kind: match entry.kind.as_str() {
            "doc" => Some(RpmFileKind::Doc),
            "license" | "licence" => Some(RpmFileKind::License),
            "readme" => Some(RpmFileKind::Readme),
            _ => None,
        },
    };
    let is_symlink = entry.kind == "symlink";
    let recurse = matches!(
        entry.kind.as_str(),
        "tree" | "dir" | "config|tree" | "config|noreplace|tree" | "config|missingok|tree"
    );
    for path in staged {
        if recurse || path.is_dir() {
            for p in walk_paths(path)? {
                let installed = installed_for(root, &p)?;
                let mut m = base.clone();
                if p.is_dir()
                    && !entry.disown_subtree.is_empty()
                    && matches_any(&installed, &entry.disown_subtree)
                {
                    m.disown = true;
                }
                if !m.is_empty() {
                    meta.insert(installed, m);
                }
            }
        } else {
            let mut m = base.clone();
            if is_symlink {
                m.mode = None;
            }
            if !m.is_empty() {
                meta.insert(installed_for(root, path)?, m);
            }
        }
    }
    Ok(())
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn installed_for(root: &Path, path: &Path) -> anyhow::Result<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("staged path '{}' escaped the root", path.display()))?;
    Ok(installed_path(&rel.to_string_lossy()))
}

/// `path` plus every descendant, for applying a tree-wide `file_info`.
fn walk_paths(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = vec![path.to_path_buf()];
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            out.extend(walk_paths(&entry.path())?);
        }
    }
    Ok(out)
}

fn matches_any(installed: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        let p = if p.starts_with('/') {
            p.clone()
        } else {
            format!("/{p}")
        };
        installed == p
            || glob::Pattern::new(&p)
                .map(|pat| pat.matches(installed))
                .unwrap_or(false)
    })
}

/// Go `os.Expand` semantics: `$NAME` / `${NAME}` become the environment
/// value, or the empty string when unset. Used by `contents[].expand: true`.
fn os_expand(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('{') => {
                chars.next();
                let mut name = String::new();
                let mut closed = false;
                for c2 in chars.by_ref() {
                    if c2 == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c2);
                }
                if closed {
                    out.push_str(&std::env::var(&name).unwrap_or_default());
                } else {
                    out.push_str("${");
                    out.push_str(&name);
                }
            }
            Some(c2) if c2.is_ascii_alphanumeric() || c2 == '_' => {
                let mut name = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_alphanumeric() || next == '_' {
                        name.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push_str(&std::env::var(&name).unwrap_or_default());
            }
            _ => out.push('$'),
        }
    }
    out
}

/// Stage a single contents entry, expanding glob patterns when applicable.
///
/// For `file`/`config*` types: if `src` contains glob characters and
/// `disable_globbing` is false, expand the pattern and stage each matching
/// file into `dst` (treated as a directory). Otherwise, stage `src` directly
/// to `dst`.
///
/// For `tree` type: if `src` contains glob characters and `disable_globbing`
/// is false, expand the pattern and copy each matching directory as a tree
/// into `dst`. Otherwise, copy `src` recursively to `dst`.
fn stage_contents_entry(
    src: &Path,
    dst: &Path,
    kind: &str,
    umask: Option<u32>,
    disable_globbing: bool,
    is_tree: bool,
) -> anyhow::Result<Vec<PathBuf>> {
    let src_str = src.to_string_lossy();
    let should_glob = !disable_globbing && super::staging::is_glob_pattern(&src_str);

    if !should_glob {
        // Non-glob path: original behavior.
        if is_tree {
            super::staging::copy_dir_recursive(src, dst)?;
        } else {
            super::staging::stage_file(src, dst, umask)?;
        }
        return Ok(vec![dst.to_path_buf()]);
    }

    // Glob expansion path.
    let is_dir_type = is_tree || kind == "dir";
    let matches: Vec<_> = glob::glob(&src_str)
        .with_context(|| format!("invalid glob pattern '{src_str}'"))?
        .filter_map(|r| r.ok())
        .collect();

    if matches.is_empty() {
        anyhow::bail!("glob pattern '{src_str}' matched no files");
    }

    let mut staged = Vec::new();
    if is_dir_type {
        // For tree type: copy each matching directory as a subtree.
        std::fs::create_dir_all(dst)?;
        for matched in matches {
            if matched.is_dir() {
                let name = matched.file_name().ok_or_else(|| {
                    anyhow::anyhow!("glob match '{}' has no file name", matched.display())
                })?;
                let dest = dst.join(name);
                super::staging::copy_dir_recursive(&matched, &dest)?;
                staged.push(dest);
            }
        }
    } else {
        // For file/config type: stage each matching file into dst (as directory).
        std::fs::create_dir_all(dst)?;
        for matched in matches {
            if matched.is_file() {
                let name = matched.file_name().ok_or_else(|| {
                    anyhow::anyhow!("glob match '{}' has no file name", matched.display())
                })?;
                let dest = dst.join(name);
                super::staging::stage_file(&matched, &dest, umask)?;
                staged.push(dest);
            }
        }
    }
    Ok(staged)
}

/// Read the configured maintainer scripts from the build environment and
/// render them as deb control members (`preinst`/`postinst`/`prerm`/
/// `postrm`, mode 0755). Empty when none are configured.
///
/// When `cfg.template_scripts` is true, each script file is processed
/// through the template engine before being staged, replacing `<%= key %>`
/// expressions with package values from the build context.
pub fn maintainer_script_members(
    ctx: &BuildContext,
) -> anyhow::Result<Vec<lx_lib::debarchive::ControlMember>> {
    let cfg = ctx.cfg;
    let pairs = [
        ("preinst", cfg.scripts.preinstall.trim()),
        ("postinst", cfg.scripts.postinstall.trim()),
        ("prerm", cfg.scripts.preremove.trim()),
        ("postrm", cfg.scripts.postremove.trim()),
        ("preupgrade", cfg.scripts.preupgrade_script.trim()),
        ("postupgrade", cfg.scripts.postupgrade_script.trim()),
    ];
    let template_context = if cfg.template_scripts {
        Some(lx_lib::templating::build_context(cfg, ctx.job))
    } else {
        None
    };
    let mut members = Vec::new();
    for (name, path) in pairs {
        if path.is_empty() {
            continue;
        }
        let content = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("failed to read {} script '{}': {e}", name, path))?;
        let content = if let Some(ref tmpl_ctx) = template_context {
            let script_text = String::from_utf8_lossy(&content);
            let rendered = lx_lib::templating::render_template(&script_text, tmpl_ctx);
            rendered.into_bytes()
        } else {
            content
        };
        members.push(lx_lib::debarchive::ControlMember {
            name: name.to_string(),
            content,
            mode: 0o755,
        });
    }
    Ok(members)
}

/// Read a maintainer-script file from the build environment, applying
/// `template_scripts` rendering when enabled. Returns `None` for an empty
/// path so callers can skip unset hooks.
///
/// Shared by the rpm and Arch plugins; the deb plugin's
/// [`maintainer_script_members`] does the same inline because it also has to
/// emit the members as a multi-file control set.
pub fn render_script_body(
    cfg: &PackageConfig,
    job: &crate::build::ResolvedJob,
    path: &str,
) -> anyhow::Result<Option<String>> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read script '{path}': {e}"))?;
    let content = if cfg.template_scripts {
        let ctx = lx_lib::templating::build_context(cfg, job);
        lx_lib::templating::render_template(&content, &ctx)
    } else {
        content
    };
    Ok(Some(content))
}

/// Read the deb-specific control members from the build environment: debconf
/// `templates` (mode 0644) and `config` (mode 0755), `rules` (mode 0755),
/// and the `triggers` file (mode 0644) listing `interest`/`activate`
/// lines. Mirrors nfpm's `deb.scripts.rules`, `deb.scripts.templates`,
/// `deb.scripts.config`, and `deb.triggers.{interest,activate}`.
///
/// Returns an empty vec when none of these are configured, so callers can
/// skip this call entirely for non-deb formats.
pub fn deb_extra_members(
    cfg: &PackageConfig,
) -> anyhow::Result<Vec<lx_lib::debarchive::ControlMember>> {
    let deb = &cfg.deb;
    let mut members = Vec::new();

    // rules (mode 0755)
    if !deb.rules.trim().is_empty() {
        let content = std::fs::read(&deb.rules).map_err(|e| {
            anyhow::anyhow!(
                "failed to read deb rules script '{}': {e}",
                deb.rules.trim()
            )
        })?;
        members.push(lx_lib::debarchive::ControlMember {
            name: "rules".to_string(),
            content,
            mode: 0o755,
        });
    }

    // debconf templates (mode 0644)
    if !deb.templates.trim().is_empty() {
        let content = std::fs::read(&deb.templates).map_err(|e| {
            anyhow::anyhow!(
                "failed to read debconf templates '{}': {e}",
                deb.templates.trim()
            )
        })?;
        members.push(lx_lib::debarchive::ControlMember {
            name: "templates".to_string(),
            content,
            mode: 0o644,
        });
    }

    // debconf config (mode 0755)
    if !deb.config.trim().is_empty() {
        let content = std::fs::read(&deb.config).map_err(|e| {
            anyhow::anyhow!(
                "failed to read debconf config script '{}': {e}",
                deb.config.trim()
            )
        })?;
        members.push(lx_lib::debarchive::ControlMember {
            name: "config".to_string(),
            content,
            mode: 0o755,
        });
    }

    // triggers (mode 0644)
    let mut trigger_lines: Vec<String> = Vec::new();
    for t in &deb.triggers_interest {
        trigger_lines.push(format!("interest {}", t.trim()));
    }
    for t in &deb.triggers_interest_await {
        trigger_lines.push(format!("interest_await {}", t.trim()));
    }
    for t in &deb.triggers_interest_noawait {
        trigger_lines.push(format!("interest_noawait {}", t.trim()));
    }
    for t in &deb.triggers_activate {
        trigger_lines.push(format!("activate {}", t.trim()));
    }
    for t in &deb.triggers_activate_await {
        trigger_lines.push(format!("activate_await {}", t.trim()));
    }
    for t in &deb.triggers_activate_noawait {
        trigger_lines.push(format!("activate_noawait {}", t.trim()));
    }
    if !trigger_lines.is_empty() {
        trigger_lines.push(String::new()); // trailing newline
        members.push(lx_lib::debarchive::ControlMember {
            name: "triggers".to_string(),
            content: trigger_lines.join("\n").into_bytes(),
            mode: 0o644,
        });
    }

    Ok(members)
}
