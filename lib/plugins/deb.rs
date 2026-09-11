//! Debian `.deb` plugin.
//!
//! Extracted from the original `build.rs` / `debarchive.rs` / `pkgmeta.rs`
//! pipeline so that `.deb` is one plugin among many, not the hard-coded
//! only format.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::{BuildContext, Plugin, PACKAGED_FROM_LINE};
use crate::config::PackageConfig;

pub struct DebPlugin;

impl Plugin for DebPlugin {
    fn name(&self) -> &'static str {
        "deb"
    }

    fn file_extension(&self) -> &'static str {
        "deb"
    }

    fn description(&self) -> &'static str {
        "Debian package (.deb) — ar + control.tar.gz + data.tar.gz"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_DEBIAN_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool {
        // Delegate to PackageConfig's matrix; construct a dummy config to
        // avoid duplicating the logic. The matrix is static.
        PackageConfig::default().arch_supported_for_dist(arch, dist)
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        let cfg = ctx.cfg;

        // Stage install tree under ctx.staging_root, then layer the
        // `contents:` overlay (completions, units, desktop files, ...).
        super::stage_install_tree(cfg, ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        archive_staged_tree(ctx)
    }
}

/// Archive an already-populated `staging_root` into a `.deb`.
///
/// Shared tail of [`DebPlugin::build`]: layers `contents:`, renders
/// control/changelog/copyright, and writes the ar container. Source-mode
/// builds populate the staging root from a `DESTDIR` install tree instead
/// of `stage_install_tree` and reuse this directly.
pub(crate) fn archive_staged_tree(ctx: &BuildContext) -> Result<PathBuf> {
    let cfg = ctx.cfg;
    let job = ctx.job;
    let conffiles = super::apply_contents(cfg, ctx.staging_root, "deb")?;

    // Render control/changelog/copyright.
    let control = render_control(cfg, job, ctx.debian_version, ctx.build_version);
    let changelog = render_changelog(cfg, job, ctx.debian_version, ctx.build_version);
    let doc_dir = ctx
        .staging_root
        .join("usr")
        .join("share")
        .join("doc")
        .join(&cfg.package_name);
    std::fs::create_dir_all(&doc_dir)?;
    write_changelog_gz(&doc_dir, &changelog, ctx.mtime)?;
    write_copyright(&doc_dir, cfg, ctx.license, job.published_at)?;

    // Archive via debarchive. Maintainer scripts and the conffiles
    // list ride along as extra control members.
    let full_version = format!(
        "{}-{}+{dist}_{arch}",
        ctx.debian_version,
        ctx.build_version,
        dist = job.dist,
        arch = job.arch
    );
    let full_version_epoch = lx_lib::pkgmeta::with_epoch(&cfg.epoch, &full_version);
    // Filename never includes epoch (':' not filename-safe).
    let deb_name = format!("{}_{}.deb", cfg.package_name, full_version);

    let _ = full_version_epoch; // already in control

    let mut extras = super::maintainer_script_members(cfg)?;
    if !conffiles.is_empty() {
        extras.push(lx_lib::debarchive::ControlMember {
            name: "conffiles".to_string(),
            content: format!("{}\n", conffiles.join("\n")).into_bytes(),
            mode: 0o644,
        });
    }

    // We need a temp out dir for the .deb file; create under staging_root's
    // parent temp.
    let out_dir = ctx.staging_root.join("__out");
    std::fs::create_dir_all(&out_dir)?;
    let deb_dest = out_dir.join(&deb_name);

    let comp = cfg.effective_compression();
    #[allow(clippy::type_complexity)]
    let origin_signer: Option<Box<lx_lib::debarchive::OriginSigner>> =
        if ctx.sign_method == "debsign" {
            if let Some(key) = ctx.sign_key {
                let key = key.to_path_buf();
                let key_id = ctx.sign_key_id.to_string();
                let passphrase = ctx.sign_passphrase.map(|s| s.to_string());
                Some(Box::new(move |payload: &[u8]| {
                    let req = lx_lib::sign::SignRequest {
                        key_file: &key,
                        key_id: &key_id,
                        passphrase: passphrase.as_deref(),
                    };
                    lx_lib::sign::clearsign(payload, &req)
                }))
            } else {
                None
            }
        } else {
            None
        };
    let signer_ref = origin_signer.as_ref().map(|f| f.as_ref());
    let sig_type = cfg.effective_sign_type();
    lx_lib::debarchive::build_full_signed(
        ctx.staging_root,
        control.as_bytes(),
        ctx.mtime,
        &deb_dest,
        &comp,
        &extras,
        signer_ref.map(|s| (s, sig_type.as_str())),
    )
    .with_context(|| format!("failed to build {}", deb_dest.display()))?;

    Ok(deb_dest)
}

// ---------------------------------------------------------------------------
// Helpers moved from build.rs (deb-specific rendering)
// ---------------------------------------------------------------------------

pub fn render_control(
    cfg: &PackageConfig,
    job: &crate::build::ResolvedJob,
    version: &str,
    build_version: &str,
) -> String {
    let full_version = lx_lib::pkgmeta::with_epoch(
        &cfg.epoch,
        &format!("{version}-{build_version}+{dist}", dist = job.dist),
    );
    // Per-format overrides applied for "deb".
    let relations = cfg.effective_relations("deb").render();
    let homepage = if cfg.effective_source() == "gitlab" {
        lx_lib::constants::homepage_for_gitlab(
            &cfg.github_repo,
            cfg.gitlab_host.as_deref().unwrap_or(""),
        )
    } else {
        lx_lib::constants::homepage_for_github(&cfg.github_repo)
    };
    let extra_fields = super::render_extra_fields(&cfg.fields);
    format!(
        "Section: {}\nPriority: {}\nPackage: {pkg}\nVersion: {full_version}\nArchitecture: {arch}\nMaintainer: {maintainer}\nHomepage: {homepage}\nDescription: {desc}\n{PACKAGED_FROM_LINE}\n{relations}{extra_fields}",
        cfg.effective_section(),
        cfg.effective_priority(),
        pkg = cfg.package_name,
        arch = job.arch,
        maintainer = cfg.effective_maintainer(),
        desc = cfg.effective_description(),
    )
}

pub fn render_changelog(
    cfg: &PackageConfig,
    job: &crate::build::ResolvedJob,
    version: &str,
    build_version: &str,
) -> String {
    let full_version = lx_lib::pkgmeta::with_epoch(
        &cfg.epoch,
        &format!("{version}-{build_version}+{dist}", dist = job.dist),
    );
    lx_lib::pkgmeta::render_changelog_entry(
        &cfg.package_name,
        &full_version,
        &job.dist,
        version,
        &cfg.effective_maintainer(),
        job.published_at,
    )
}

fn write_changelog_gz(doc_dir: &Path, changelog: &str, mtime: i64) -> Result<()> {
    use std::io::Write;
    let out = std::fs::File::create(doc_dir.join("changelog.Debian.gz"))?;
    // Same deterministic treatment as the tar members (fixed mtime header)
    // via debarchive's shared helper.
    let gz = lx_lib::debarchive::deterministic_gzip_bytes(changelog.as_bytes(), mtime, 9)?;
    let mut out = out;
    out.write_all(&gz)?;
    Ok(())
}

pub fn write_copyright(
    output_dir: &Path,
    cfg: &PackageConfig,
    license: Option<&lx_lib::github::RepoLicense>,
    published_at: Option<i64>,
) -> Result<()> {
    use std::io::Write;
    let text = lx_lib::pkgmeta::render_copyright(
        &cfg.package_name,
        &cfg.github_repo,
        license,
        &cfg.license_spdx,
        published_at,
    );
    let mut f = std::fs::File::create(output_dir.join("copyright"))?;
    f.write_all(text.as_bytes())?;
    Ok(())
}
