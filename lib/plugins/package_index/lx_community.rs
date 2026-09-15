// SPDX-License-Identifier: GPL-3.0-or-later
//! The LX community index backend (`ranjithrajv/lx-index`) — recipes plus
//! prebuilt binaries per release.

use anyhow::{bail, Context, Result};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{PackageIndex, ReadIndex};
use crate::debs::{confirm, detect_dist};
use crate::index::{detect_host_format, IndexHit, InstallOpts, InstallOutcome};
use crate::install_pkg::{detect_arch, install_prebuilt, InstallPolicy};

const RECIPES_DIR: &str = "recipes";
const STALE_HOURS: u64 = 24;

/// A read backend over a git repository of lx recipes with per-release
/// prebuilts. Backs both the built-in LX community index and a user-added
/// `custom` index (`SourceKind::Custom { url }`); they differ only in URL,
/// identity, and cache location.
pub struct GitIndexSource {
    kind: &'static str,
    description: &'static str,
    name: String,
    repo_url: String,
    cache: PathBuf,
}

impl GitIndexSource {
    /// The built-in LX community index.
    pub const DEFAULT_URL: &'static str = "https://github.com/ranjithrajv/lx-index.git";

    pub fn lx_community(name: &str) -> Self {
        let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            kind: "lx-community",
            description: "LX community index (recipes + per-release prebuilts)",
            name: name.to_string(),
            repo_url: Self::DEFAULT_URL.to_string(),
            // Keep the historical cache location for the built-in index.
            cache: base.join("lx").join("index"),
        }
    }

    /// A user-added custom index at `url`.
    pub fn custom(name: &str, url: &str) -> Self {
        let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            kind: "custom",
            description: "Custom git index (recipes + per-release prebuilts)",
            name: name.to_string(),
            repo_url: url.to_string(),
            // Per-name cache dir so a custom source does not share (and
            // overwrite) the built-in index's checkout.
            cache: base.join("lx").join("index").join(name),
        }
    }

    pub fn recipes(&self) -> Result<BTreeMap<String, RecipeEntry>> {
        let root = self.cache.join(RECIPES_DIR);
        if !root.is_dir() {
            return Ok(BTreeMap::new());
        }
        let mut out = BTreeMap::new();
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let yaml_path = entry.path().join("package.yaml");
            if !yaml_path.is_file() {
                continue;
            }
            let yaml = std::fs::read_to_string(&yaml_path)?;
            let readme = std::fs::read_to_string(entry.path().join("README.md")).ok();
            out.insert(name, RecipeEntry { yaml, readme });
        }
        Ok(out)
    }

    fn prebuilt_tags(&self, package: &str) -> Result<Vec<String>> {
        let root = self.cache.join(RECIPES_DIR).join(package).join("releases");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&root)? {
            let e = e?;
            if e.file_type()?.is_dir() {
                out.push(e.file_name().to_string_lossy().to_string());
            }
        }
        out.sort_by(|a, b| b.cmp(a));
        Ok(out)
    }

    fn prebuilt_dir(&self, package: &str, tag: &str) -> PathBuf {
        self.cache
            .join(RECIPES_DIR)
            .join(package)
            .join("releases")
            .join(tag)
    }

    fn checksums(&self, package: &str, tag: &str) -> Result<BTreeMap<String, String>> {
        let path = self.prebuilt_dir(package, tag).join("checksums.sha256");
        let text = std::fs::read_to_string(&path)?;
        let mut out = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(2, "  ");
            let sha = parts.next().context("missing sha")?.to_string();
            let name = parts
                .next()
                .context("missing filename")?
                .trim_matches('*')
                .trim()
                .to_string();
            if sha.len() == 64 && !name.is_empty() {
                out.insert(name, sha);
            }
        }
        Ok(out)
    }

    fn desc_from_yaml(yaml: &str) -> String {
        for line in yaml.lines() {
            let t = line.trim();
            if let Some(v) = t
                .strip_prefix("description:")
                .or_else(|| t.strip_prefix("summary:"))
            {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
        String::new()
    }

    fn run_git(&self, args: &[&str]) -> Result<String> {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&self.cache)
            .output()
            .context("failed to run git")?;
        if !out.status.success() {
            bail!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8(out.stdout)?)
    }

    fn stale(&self) -> Option<u64> {
        let since = self
            .cache
            .join(".git")
            .join("FETCH_HEAD")
            .metadata()
            .ok()?
            .modified()
            .ok()?;
        let age = since.elapsed().ok()?.as_secs() / 3600;
        (age >= STALE_HOURS).then_some(age)
    }
}

/// One recipe in the community index: its parsed package.yaml and optional README.
pub struct RecipeEntry {
    pub yaml: String,
    pub readme: Option<String>,
}

impl crate::plugins::plugin::Plugin for GitIndexSource {
    fn name(&self) -> &'static str {
        self.kind
    }

    fn description(&self) -> &'static str {
        self.description
    }
}

impl PackageIndex for GitIndexSource {
    fn instance_name(&self) -> &str {
        &self.name
    }
}

impl ReadIndex for GitIndexSource {
    fn search(&self, pattern: Option<&str>) -> Result<Vec<IndexHit>> {
        let re = match pattern {
            Some(p) => Some(Regex::new(&format!("(?i){p}"))?),
            None => None,
        };
        let mut hits = Vec::new();
        for (name, entry) in self.recipes()? {
            if let Some(re) = &re {
                let text = format!(
                    "{}\n{}\n{}",
                    name,
                    Self::desc_from_yaml(&entry.yaml),
                    entry.readme.as_deref().unwrap_or("")
                );
                if !re.is_match(&text) {
                    continue;
                }
            }
            let tags = self.prebuilt_tags(&name)?;
            let installed = crate::debs::dpkg_installed_version(&name).is_some();
            let description = Self::desc_from_yaml(&entry.yaml);
            hits.push(IndexHit {
                name,
                description,
                source: self.name.clone(),
                installed,
                available: tags,
                ..Default::default()
            });
        }
        Ok(hits)
    }

    fn info(&self, package: &str) -> Result<Option<IndexHit>> {
        let recipes = self.recipes()?;
        let entry = match recipes.get(package) {
            Some(e) => e,
            None => return Ok(None),
        };
        let tags = self.prebuilt_tags(package)?;
        Ok(Some(IndexHit {
            name: package.to_string(),
            description: Self::desc_from_yaml(&entry.yaml),
            source: self.name.clone(),
            installed: crate::debs::dpkg_installed_version(package).is_some(),
            available: tags,
            ..Default::default()
        }))
    }

    fn install(&self, package: &str, opts: InstallOpts) -> Result<Option<InstallOutcome>> {
        if let Some(age) = self.stale() {
            eprintln!(
                "⚠ index cache is {age}h old (>{STALE_HOURS}h); run `lx index update` for the newest"
            );
        }
        let recipes = self.recipes()?;
        let entry = recipes.get(package).ok_or_else(|| {
            anyhow::anyhow!(
                "'{package}' not found in {} (run `lx index search {package}`)",
                self.name
            )
        })?;
        // The LX community index ships .deb prebuilts; match host arch + dist.
        let pkg_fmt = detect_host_format();
        let arch = detect_arch(pkg_fmt)?;
        let dist = detect_dist().unwrap_or_default();

        if !opts.build {
            let tags = match &opts.tag {
                Some(t) => vec![t.clone()],
                None => self.prebuilt_tags(package)?,
            };
            for tag in &tags {
                let checksums = match self.checksums(package, tag) {
                    Ok(c) if !c.is_empty() => c,
                    _ => continue,
                };
                for (filename, expected_sha) in &checksums {
                    if !filename.ends_with(pkg_fmt.extension()) {
                        continue;
                    }
                    let parsed = match parse_prebuilt_name(filename) {
                        Some(p) => p,
                        None => continue,
                    };
                    if parsed.arch != arch {
                        continue;
                    }
                    if !parsed.dist.is_empty() && parsed.dist != dist {
                        continue;
                    }
                    let path = self.prebuilt_dir(package, tag).join(filename);
                    if path.is_file() {
                        if let Some(dir) = &opts.download_only {
                            std::fs::create_dir_all(dir)?;
                            std::fs::copy(path, dir.join(filename))?;
                            println!("downloaded {filename} → {}", dir.display());
                            return Ok(None);
                        }
                        let installed = install_prebuilt(
                            &path,
                            filename,
                            pkg_fmt,
                            expected_sha,
                            InstallPolicy {
                                assume_yes: opts.yes,
                                no_verify: opts.no_verify,
                                allow_unverified: opts.allow_unverified,
                            },
                        )?;
                        if !installed {
                            return Ok(None);
                        }
                        // Fall back to the release tag when the filename
                        // doesn't spell out a version (non-deb spellings).
                        let version = if parsed.version.is_empty() {
                            tag.clone()
                        } else {
                            parsed.version.clone()
                        };
                        return Ok(Some(InstallOutcome {
                            package: package.to_string(),
                            version,
                            arch: arch.clone(),
                            distribution: dist.clone(),
                            asset: filename.clone(),
                            tag: tag.clone(),
                            format: pkg_fmt.name().to_string(),
                        }));
                    }
                }
            }
            if opts.tag.is_some() {
                bail!(
                    "no prebuilt for '{package}' matching tag '{}' on {dist}/{arch}",
                    opts.tag.as_deref().unwrap()
                );
            }
            println!("no prebuilt for '{package}' on {dist}/{arch}; building from recipe");
        }
        // Recipe builds are installed and recorded, so an AUR/community
        // recipe joins the lx-managed lifecycle like a prebuilt.
        build_from_recipe(&entry.yaml, package, &opts)
    }

    fn update(&self) -> Result<bool> {
        if !self.cache.join(".git").is_dir() {
            std::fs::create_dir_all(&self.cache)?;
            self.run_git(&["clone", "--depth", "1", self.repo_url.as_str(), "."])?;
            return Ok(true);
        }
        let before = self.run_git(&["rev-parse", "HEAD"])?;
        self.run_git(&["pull", "--ff-only"])?;
        let after = self.run_git(&["rev-parse", "HEAD"])?;
        Ok(before.trim() != after.trim())
    }
}

/// Parse `{pkg}_{version}+{dist}_{arch}.ext` -> (version, arch, dist).
pub(crate) fn parse_prebuilt_name(filename: &str) -> Option<ParsedName> {
    let stem = if let Some(s) = filename.strip_suffix(".pkg.tar.zst") {
        s
    } else if let Some(s) = filename.strip_suffix(".deb") {
        s
    } else if let Some(s) = filename.strip_suffix(".rpm") {
        s
    } else {
        let mut parts = filename.rsplitn(2, '.');
        parts.next()?;
        parts.next()?
    };
    let (_, arch) = stem.rsplit_once('_')?;
    // Everything between the first `_` (after the package name) and the
    // trailing `_{arch}` is the version, in the host manager's spelling
    // (Debian `Version`, e.g. `0.20.0-1+bookworm`).
    let version = stem
        .split_once('_')
        .and_then(|(_, rest)| rest.rsplit_once('_'))
        .map(|(v, _)| v)
        .unwrap_or("");
    let dist = match stem.split_once('+') {
        Some((_, after_plus)) => after_plus.rsplit_once('_').map(|(d, _)| d).unwrap_or(""),
        None => "",
    };
    Some(ParsedName {
        version: version.to_string(),
        arch: arch.to_string(),
        dist: dist.to_string(),
    })
}

/// Parsed prebuilt-filename fields.
pub struct ParsedName {
    pub version: String,
    pub arch: String,
    pub dist: String,
}

/// Build a recipe from the index into a package. Shared by the LX community
/// and AUR backends (AUR converts its PKGBUILD to a recipe first).
pub fn build_from_recipe(
    yaml: &str,
    package: &str,
    opts: &InstallOpts,
) -> Result<Option<InstallOutcome>> {
    let tmp = tempfile::tempdir()?;
    let cfg_path = tmp.path().join("package.yaml");
    std::fs::write(&cfg_path, yaml)?;
    if !opts.yes && !confirm(&format!("build '{package}' from recipe?"), false)? {
        bail!("aborted");
    }
    println!("building '{package}' from recipe …");
    // A recipe/AUR build produces the host's native package format by
    // default: emitting a `.deb` on an Arch host (or vice versa) yields an
    // uninstallable artifact. A recipe that pins `package_format:` still wins.
    let recipe_pins_format = yaml
        .lines()
        .any(|l| l.trim_start().starts_with("package_format:"));
    let format = if recipe_pins_format {
        None
    } else {
        Some(detect_host_format().name().to_string())
    };
    let dist_dir = tmp.path().join("dist");
    crate::build::run(
        crate::build::BuildArgs {
            config: cfg_path,
            all: None,
            version: None,
            build_version: "1".into(),
            architectures: None,
            host: true,
            distributions: None,
            output: dist_dir.clone(),
            format,
            provider: None,
            no_verify: false,
            allow_unverified: false,
            lintian: false,
            lintian_fail_on_warnings: false,
            lintian_pedantic: false,
            lintian_suppress: None,
            dry_run: false,
            max_parallel: 1,
            pinned_metadata: None,
            cache_dir: None,
            api_cache_dir: None,
            source: false,
            summary: false,
            telemetry: false,
            save_baseline: false,
            progress: false,
            progress_path: None,
            keep: false,
            sign_key: None,
            sign_key_id: None,
            sign_method: None,
            local: false,
            from_dir: None,
            from_file: None,
            package_name: None,
            prefix: None,
            overlay: None,
            update_lock: false,
            artifact_cache_dir: None,
            verify: false,
            sandbox: false,
            install_build_deps: opts.install_build_deps,
            sbom: false,
            cosign: false,
            cross_target: None,
            bindep: true,
        },
        None,
    )?;

    // Install the freshly built artifact and record it, so a recipe build is
    // a real lx-managed generation rather than build-only.
    let format = detect_host_format();
    let Some((path, filename)) = find_built_artifact(&dist_dir, format)? else {
        bail!(
            "recipe build produced no {} artifact in {}",
            format.name(),
            dist_dir.display()
        );
    };
    let arch = detect_arch(format)?;
    let dist = detect_dist().unwrap_or_default();
    // Install only — the caller (`install_from_active`) records the generation
    // from the returned outcome. Recording here too double-recorded: one
    // recipe install appended two identical manifest generations.
    crate::consumer::install(&path, &filename, format, true)?;
    println!("  ✓ installed {filename}");
    // Record the host manager's own version spelling (Arch `0.9.0-1.arch`,
    // not the recipe's bare `0.9.0`), so `lx list`/`system-check` compare equal
    // to what pacman/dpkg reports instead of flagging a spurious "changed".
    let version = crate::consumer::installed_version(package, format)
        .or_else(|| recipe_version(yaml))
        .unwrap_or_else(|| filename.clone());
    Ok(Some(InstallOutcome {
        package: package.to_string(),
        version,
        arch,
        distribution: dist,
        asset: filename,
        tag: String::new(),
        format: format.name().to_string(),
    }))
}

/// Find the build artifact for `format` in `dir` (deterministic: lowest
/// filename), skipping source RPMs and signatures.
fn find_built_artifact(
    dir: &Path,
    format: crate::index::InstallFormat,
) -> Result<Option<(PathBuf, String)>> {
    let ext = format.extension();
    let mut found: Option<(PathBuf, String)> = None;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(None);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(ext) || name.ends_with(".src.rpm") || name.ends_with(".sig") {
            continue;
        }
        if found.as_ref().is_none_or(|(_, n)| name < *n) {
            found = Some((entry.path(), name));
        }
    }
    Ok(found)
}

/// The `version:` a recipe declares, if any. AUR recipes set it from
/// `pkgver`; when absent, the artifact filename is used instead.
fn recipe_version(yaml: &str) -> Option<String> {
    let cfg: crate::config::PackageConfig = serde_yaml::from_str(yaml).ok()?;
    let v = cfg.version.trim();
    (!v.is_empty()).then(|| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_built_artifact_picks_the_binary_not_source_or_sig() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("foo-1.0-1.x86_64.rpm"), b"x").unwrap();
        std::fs::write(tmp.path().join("foo-1.0-1.src.rpm"), b"x").unwrap();
        std::fs::write(tmp.path().join("foo-1.0-1.x86_64.rpm.sig"), b"x").unwrap();
        let found = find_built_artifact(tmp.path(), crate::index::InstallFormat::Rpm)
            .unwrap()
            .expect("a binary rpm");
        assert_eq!(found.1, "foo-1.0-1.x86_64.rpm");
    }

    #[test]
    fn recipe_version_reads_the_yaml_version() {
        assert_eq!(
            recipe_version("package_name: foo\nversion: \"1.2.3\"\n").as_deref(),
            Some("1.2.3")
        );
        assert!(recipe_version("package_name: foo\n").is_none());
    }
}
