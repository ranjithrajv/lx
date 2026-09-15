// SPDX-License-Identifier: GPL-3.0-or-later

//! `deb-get` catalog as a read index backend.
//!
//! Reads the catalog via [`crate::debget`] (safe static parse) and installs
//! each package by its deb-get method:
//!
//! * `direct`  — a literal `.deb` URL
//! * `github`  — the latest GitHub release's arch-matching `.deb` asset
//! * `gitlab`  — the latest GitLab release's arch-matching `.deb` asset
//! * `website` — a `.deb` link scraped from the project page
//! * `apt`     — a third-party apt repository (key + `sources.list.d`), then
//!   the host `apt-get`
//! * `ppa`     — a Launchpad PPA via `add-apt-repository`, then `apt-get`
//!
//! Integrity follows lx: provider/definition checksums and sidecars are
//! verified, and a package with none requires `--allow-unverified` (fail
//! closed). Definitions that need shell execution (`post_download`, a computed
//! `URL`) are reported as unsupported rather than run.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::{PackageIndex, ReadIndex};
use crate::debget::{Catalog, DebGetKind, DebGetPackage};
use crate::index::{detect_host_format, IndexHit, InstallFormat, InstallOpts, InstallOutcome};
use crate::install_pkg::detect_arch;
use crate::plugins::plugin::plugin_identity;
use crate::release::Asset;

/// A read backend over a deb-get catalog directory.
pub struct DebGetSource {
    name: String,
    root: PathBuf,
}

impl DebGetSource {
    /// The built-in `deb-get` catalog at [`Catalog::default_root`].
    pub fn new(name: &str) -> Self {
        Self::with_root(name, Catalog::default_root())
    }

    /// A catalog at an explicit root (used by tests and `custom` catalogs).
    pub fn with_root(name: &str, root: PathBuf) -> Self {
        Self {
            name: name.to_string(),
            root,
        }
    }

    fn catalog(&self) -> Catalog {
        Catalog::load(&self.root)
    }

    fn installed(&self, package: &str) -> bool {
        crate::debs::dpkg_installed_version(package).is_some()
    }

    fn hit(&self, p: &DebGetPackage) -> IndexHit {
        let mut description = p.summary.clone();
        if !p.is_supported() {
            description = format!("{} [lx: {}]", description, p.unsupported.join("; "));
        }
        IndexHit {
            name: p.name.clone(),
            description,
            source: self.name.clone(),
            installed: self.installed(&p.name),
            available: vec![p.kind_label().to_string()],
            ..Default::default()
        }
    }

    /// Fetch and install, returning the recorded generation. `Ok(None)` for a
    /// download-only run (nothing was installed). The caller
    /// (`lx index`/`lx install`) records the returned outcome.
    fn install_pkg(
        &self,
        pkg: &DebGetPackage,
        opts: &InstallOpts,
    ) -> Result<Option<InstallOutcome>> {
        match pkg.kind {
            DebGetKind::Direct => {
                let url = pkg
                    .url
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("'{}' has no URL", pkg.name))?;
                self.install_url(pkg, url, opts)
            }
            DebGetKind::Github => self.install_forge(pkg, Forge::Github, opts),
            DebGetKind::Gitlab => self.install_forge(pkg, Forge::Gitlab, opts),
            DebGetKind::Website => self.install_website(pkg, opts),
            DebGetKind::Apt => self.install_apt_repo(pkg, opts),
            DebGetKind::Ppa => self.install_ppa(pkg, opts),
            DebGetKind::Unknown => bail!(
                "'{}' has no recognizable deb-get method (unsupported: {})",
                pkg.name,
                pkg.unsupported.join("; ")
            ),
        }
    }

    /// Download a `.deb` URL, verify it, install it, and record the generation.
    fn install_url(
        &self,
        pkg: &DebGetPackage,
        url: &str,
        opts: &InstallOpts,
    ) -> Result<Option<InstallOutcome>> {
        let filename = url.rsplit('/').next().unwrap_or(&pkg.name).to_string();
        let asset = Asset {
            name: filename.clone(),
            size: None,
            browser_download_url: url.to_string(),
            checksums: Default::default(),
        };
        let getter = PlainGetter::new()?;
        let dir = std::env::temp_dir();
        let dest = dir.join(&filename);
        println!("  ↓ downloading {url}");
        crate::debs::download(&getter, &asset, &dest)?;

        if !opts.no_verify {
            // Fail closed: a sidecar/inline checksum, or an explicit opt-out.
            crate::debs::verify_sidecar_or_require_flag(
                &getter,
                &asset,
                &dest,
                opts.allow_unverified,
            )?;
        }

        if let Some(out_dir) = &opts.download_only {
            std::fs::create_dir_all(out_dir)?;
            std::fs::copy(&dest, out_dir.join(&filename))?;
            println!("downloaded {filename} → {}", out_dir.display());
            return Ok(None);
        }

        let version = version_from_name(&filename).unwrap_or_else(|| pkg.name.clone());
        let arch = detect_arch(InstallFormat::Deb)?;
        let dist = crate::debs::detect_dist().unwrap_or_default();
        crate::consumer::install(&dest, &filename, InstallFormat::Deb, opts.yes)?;
        Ok(Some(InstallOutcome {
            package: pkg.name.clone(),
            version,
            arch,
            distribution: dist,
            asset: filename,
            tag: String::new(),
            format: "deb".to_string(),
        }))
    }

    /// GitHub/GitLab: resolve the latest release, pick the arch `.deb`, install.
    fn install_forge(
        &self,
        pkg: &DebGetPackage,
        forge: Forge,
        opts: &InstallOpts,
    ) -> Result<Option<InstallOutcome>> {
        let repo = pkg
            .forge_repo
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("'{}' has no owner/repo", pkg.name))?;
        let (owner, name) = repo
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("'{repo}' is not owner/repo"))?;
        let arch = detect_arch(InstallFormat::Deb)?;

        let release = match forge {
            Forge::Github => {
                let client = crate::source_client::new_client_for::<lx_lib::github::GitHubClient>(
                    github_token().as_deref(),
                    None,
                )?;
                match &opts.tag {
                    Some(t) => client.release_by_tag(owner, name, t)?,
                    None => client.latest_release(owner, name)?,
                }
            }
            Forge::Gitlab => {
                let client = crate::source_client::new_client_for::<lx_lib::gitlab::GitlabClient>(
                    None, None,
                )?;
                match &opts.tag {
                    Some(t) => client.release_by_tag(owner, name, t)?,
                    None => client.latest_release(owner, name)?,
                }
            }
        };

        let asset = pick_deb_asset(&release.assets, &arch).ok_or_else(|| {
            anyhow::anyhow!(
                "no .deb for arch '{arch}' in {repo} release '{}'",
                release.tag_name
            )
        })?;
        println!(
            "  ↓ {} ({})",
            asset.name,
            crate::debs::human_size(asset.size.unwrap_or(0))
        );

        let client: Box<dyn lx_lib::checksum::RawGetter> = match forge {
            Forge::Github => Box::new(crate::source_client::new_client_for::<
                lx_lib::github::GitHubClient,
            >(github_token().as_deref(), None)?),
            Forge::Gitlab => Box::new(crate::source_client::new_client_for::<
                lx_lib::gitlab::GitlabClient,
            >(None, None)?),
        };
        let dest = std::env::temp_dir().join(&asset.name);
        crate::debs::download(client.as_ref(), asset, &dest)?;
        if !opts.no_verify {
            crate::debs::verify_sidecar_or_require_flag(
                client.as_ref(),
                asset,
                &dest,
                opts.allow_unverified,
            )?;
        }
        if let Some(out_dir) = &opts.download_only {
            std::fs::create_dir_all(out_dir)?;
            std::fs::copy(&dest, out_dir.join(&asset.name))?;
            println!("downloaded {} → {}", asset.name, out_dir.display());
            return Ok(None);
        }

        let dist = crate::debs::detect_dist().unwrap_or_default();
        crate::consumer::install(&dest, &asset.name, InstallFormat::Deb, opts.yes)?;
        Ok(Some(InstallOutcome {
            package: pkg.name.clone(),
            version: release.tag_name.clone(),
            arch,
            distribution: dist,
            asset: asset.name.clone(),
            tag: release.tag_name,
            format: "deb".to_string(),
        }))
    }

    /// Website: scrape `.deb` links from the project page, pick the arch match.
    fn install_website(
        &self,
        pkg: &DebGetPackage,
        opts: &InstallOpts,
    ) -> Result<Option<InstallOutcome>> {
        let url = pkg.website.as_str();
        if url.is_empty() {
            bail!("'{}' has no WEBSITE", pkg.name);
        }
        let html = fetch_text(url)?;
        let arch = detect_arch(InstallFormat::Deb)?;
        let candidate = scrape_deb_links(&html)
            .into_iter()
            .find(|u| deb_matches_arch(u, &arch))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no .deb for arch '{arch}' found on {url} (website scraping is best-effort)"
                )
            })?;
        self.install_url(pkg, &candidate, opts)
    }

    /// apt: add the repo's signing key + source, then install via `apt-get`.
    fn install_apt_repo(
        &self,
        pkg: &DebGetPackage,
        opts: &InstallOpts,
    ) -> Result<Option<InstallOutcome>> {
        if detect_host_format() != InstallFormat::Deb {
            bail!("deb-get `apt` packages are Debian/Ubuntu-only");
        }
        let repo_url = pkg
            .apt_repo_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("'{}' has no APT_REPO_URL", pkg.name))?;
        let list = pkg
            .apt_list_name
            .as_deref()
            .unwrap_or(&pkg.name)
            .to_string();

        let keyring = format!("/usr/share/keyrings/{list}.gpg");
        // Definitions with a `post_download` hook are marked unsupported and
        // rejected above, so the key is always fetched here.
        let key_url = pkg
            .asc_key_url
            .as_deref()
            .or(pkg.gpg_key_url.as_deref())
            .ok_or_else(|| anyhow::anyhow!("'{}' has no key URL", pkg.name))?;
        let key_body = fetch_bytes(key_url)?;
        // `.asc`/armored keys are dearmored into a keyring; `.gpg` binary
        // keys are already in keyring form.
        let bytes = if key_url.ends_with(".asc") || key_body.starts_with(b"-----BEGIN") {
            dearmor(&key_body)?
        } else {
            key_body
        };
        write_sudo(Path::new(&keyring), &bytes)?;

        let options = match pkg.apt_repo_options.as_deref() {
            Some(o) if !o.trim().is_empty() => format!(" {}", o.trim()),
            _ => String::new(),
        };
        let line = format!("deb [signed-by={keyring}{options}] {repo_url}");
        write_sudo(
            Path::new(&format!("/etc/apt/sources.list.d/{list}.list")),
            line.as_bytes(),
        )?;
        run_sudo(&["apt-get", "update"])?;
        if opts.download_only.is_some() {
            return Ok(None);
        }
        run_sudo(&["apt-get", "install", "-y", &pkg.name])?;
        let version = crate::debs::dpkg_installed_version(&pkg.name).unwrap_or_default();
        let arch = detect_arch(InstallFormat::Deb)?;
        let dist = crate::debs::detect_dist().unwrap_or_default();
        Ok(Some(InstallOutcome {
            package: pkg.name.clone(),
            version,
            arch,
            distribution: dist,
            asset: list,
            tag: String::new(),
            format: "deb".to_string(),
        }))
    }

    /// ppa: `add-apt-repository` the PPA, then install via `apt-get`.
    fn install_ppa(
        &self,
        pkg: &DebGetPackage,
        opts: &InstallOpts,
    ) -> Result<Option<InstallOutcome>> {
        if detect_host_format() != InstallFormat::Deb {
            bail!("deb-get `ppa` packages are Ubuntu-only");
        }
        let ppa = pkg
            .ppa
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("'{}' has no PPA", pkg.name))?;
        run_sudo(&["add-apt-repository", "-y", ppa])?;
        run_sudo(&["apt-get", "update"])?;
        if opts.download_only.is_some() {
            return Ok(None);
        }
        run_sudo(&["apt-get", "install", "-y", &pkg.name])?;
        let version = crate::debs::dpkg_installed_version(&pkg.name).unwrap_or_default();
        let arch = detect_arch(InstallFormat::Deb)?;
        let dist = crate::debs::detect_dist().unwrap_or_default();
        Ok(Some(InstallOutcome {
            package: pkg.name.clone(),
            version,
            arch,
            distribution: dist,
            asset: ppa.to_string(),
            tag: String::new(),
            format: "deb".to_string(),
        }))
    }
}

plugin_identity!(
    DebGetSource,
    "debget",
    "deb-get catalog (apt/PPA/GitHub/GitLab/direct .debs)"
);

impl PackageIndex for DebGetSource {
    fn instance_name(&self) -> &str {
        &self.name
    }
}

impl ReadIndex for DebGetSource {
    fn search(&self, pattern: Option<&str>) -> Result<Vec<IndexHit>> {
        let re = match pattern {
            Some(p) => Some(regex::Regex::new(&format!("(?i){p}"))?),
            None => None,
        };
        let catalog = self.catalog();
        let mut hits = Vec::new();
        for p in catalog.packages.values() {
            if let Some(re) = &re {
                let text = format!("{}\n{}\n{}", p.name, p.pretty_name, p.summary);
                if !re.is_match(&text) {
                    continue;
                }
            }
            hits.push(self.hit(p));
        }
        Ok(hits)
    }

    fn info(&self, package: &str) -> Result<Option<IndexHit>> {
        Ok(self.catalog().get(package).map(|p| self.hit(p)))
    }

    fn update(&self) -> Result<bool> {
        refresh_catalog(&self.root)
    }

    fn install(&self, package: &str, opts: InstallOpts) -> Result<Option<InstallOutcome>> {
        let catalog = self.catalog();
        let pkg = catalog.get(package).ok_or_else(|| {
            anyhow::anyhow!(
                "'{package}' not found in the deb-get catalog (run `lx index search {package}`)"
            )
        })?;
        if !pkg.is_supported() {
            bail!(
                "'{package}' cannot be installed: {}",
                pkg.unsupported.join("; ")
            );
        }
        self.install_pkg(pkg, &opts)
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Forge {
    Github,
    Gitlab,
}

/// A `RawGetter` over plain HTTPS (for direct/website downloads).
struct PlainGetter {
    http: reqwest::blocking::Client,
}

impl PlainGetter {
    fn new() -> Result<Self> {
        Ok(Self {
            http: crate::http::new_client()?,
        })
    }
}

impl lx_lib::checksum::RawGetter for PlainGetter {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        crate::http::raw_get(&self.http, url, None)
    }
}

/// Pick the first `.deb` asset whose name matches `arch`.
fn pick_deb_asset<'a>(assets: &'a [Asset], arch: &str) -> Option<&'a Asset> {
    assets
        .iter()
        .find(|a| a.name.ends_with(".deb") && deb_matches_arch(&a.name, arch))
}

/// True when a `.deb` filename looks like it targets `arch`.
fn deb_matches_arch(name: &str, arch: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let aliases: &[&str] = match arch {
        "amd64" => &["amd64", "x86_64", "x64"],
        "arm64" => &["arm64", "aarch64"],
        "armhf" => &["armhf", "armv7"],
        "i386" => &["i386", "386"],
        other => return n.contains(other),
    };
    aliases.iter().any(|a| n.contains(a))
}

/// Best-effort: pull `.deb` links out of an HTML page.
fn scrape_deb_links(html: &str) -> Vec<String> {
    let re = regex::Regex::new(r#"https?://[^"'\s<>]+\.deb"#).expect("valid regex");
    let mut out: Vec<String> = re.find_iter(html).map(|m| m.as_str().to_string()).collect();
    out.sort();
    out.dedup();
    out
}

/// Best-effort version from a `.deb` filename (`name_1.2.3-1_amd64.deb`).
fn version_from_name(filename: &str) -> Option<String> {
    let stem = filename.strip_suffix(".deb")?;
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() >= 2 {
        Some(parts[1].to_string())
    } else {
        None
    }
}

fn fetch_text(url: &str) -> Result<String> {
    let http = crate::http::new_client()?;
    let mut reader = crate::http::raw_get(&http, url, None)?;
    let mut s = String::new();
    std::io::Read::read_to_string(&mut reader, &mut s)?;
    Ok(s)
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let http = crate::http::new_client()?;
    let mut reader = crate::http::raw_get(&http, url, None)?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut buf)?;
    Ok(buf)
}

/// Dearmor an ASCII-armored PGP key into binary keyring bytes.
fn dearmor(asc: &[u8]) -> Result<Vec<u8>> {
    let text = String::from_utf8_lossy(asc);
    let mut out = Vec::new();
    let mut in_body = false;
    for line in text.lines() {
        if line.starts_with("-----BEGIN PGP") {
            in_body = true;
            continue;
        }
        if line.starts_with("-----END PGP") {
            break;
        }
        if in_body {
            out.push(line.trim());
        }
    }
    if out.is_empty() {
        bail!("could not dearmor the repository signing key");
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(out.concat())
        .context("decoding the armored signing key")
}

fn write_sudo(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = std::env::temp_dir().join(
        path.file_name()
            .map(|n| n.to_owned())
            .unwrap_or_else(|| std::ffi::OsString::from("lx-debget")),
    );
    std::fs::write(&tmp, bytes)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    run_sudo(&[
        "install",
        "-m",
        "0644",
        tmp.to_str().unwrap_or_default(),
        path.to_str().unwrap_or_default(),
    ])
}

fn run_sudo(args: &[&str]) -> Result<()> {
    let status = Command::new("sudo")
        .args(args)
        .status()
        .with_context(|| format!("failed to run `sudo {}`", args.join(" ")))?;
    if !status.success() {
        bail!("`sudo {}` failed", args.join(" "));
    }
    Ok(())
}

/// GitHub token for deb-get installs/refreshes: `GITHUB_TOKEN`, falling back
/// to deb-get's own `DEBGET_TOKEN` alias.
fn github_token() -> Option<String> {
    for var in ["GITHUB_TOKEN", "DEBGET_TOKEN"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Refresh the whole catalog under `root`:
///
/// * the built-in `01-main` catalog from `wimpysworld/deb-get`;
/// * every external `<root>/<name>.repo` manifest (deb-get's external-repo
///   protocol: first line = `https://` base URL, body = package paths, or a
///   GitHub `tree` URL).
///
/// `99-local.d` is user-managed and never touched. Returns `true` when any
/// definition changed.
fn refresh_catalog(root: &Path) -> Result<bool> {
    if !root.is_dir() {
        bail!(
            "deb-get catalog '{}' does not exist; run `lx index update` first \
             (or set LX_DEBGET_DIR to a writable catalog dir)",
            root.display()
        );
    }
    let mut changed = refresh_builtin(root)?;
    changed |= refresh_external_repos(root)?;
    Ok(changed)
}

/// The built-in catalog: a shallow clone of `wimpysworld/deb-get`, with
/// `01-main/packages/*` copied into `<root>/01-main.d/`.
fn refresh_builtin(root: &Path) -> Result<bool> {
    const REPO: &str = "https://github.com/wimpysworld/deb-get.git";
    let cache = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lx")
        .join("debget");
    std::fs::create_dir_all(&cache)?;
    let checkout = cache.join("deb-get");
    if checkout.join(".git").is_dir() {
        git(&checkout, &["fetch", "--depth", "1", "origin", "main"])?;
        git(&checkout, &["reset", "--hard", "FETCH_HEAD"])?;
    } else {
        git(&cache, &["clone", "--depth", "1", REPO, "deb-get"])?;
    }

    let src = checkout.join("01-main").join("packages");
    if !src.is_dir() {
        bail!("deb-get checkout has no 01-main/packages directory");
    }
    let dst = root.join("01-main.d");
    std::fs::create_dir_all(&dst)
        .with_context(|| format!("cannot write deb-get catalog to '{}'", dst.display()))?;
    let mut changed = false;
    for entry in std::fs::read_dir(&src)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let bytes = std::fs::read(entry.path())?;
            if write_if_changed(&dst.join(entry.file_name()), &bytes)? {
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// Refresh every external `<root>/<name>.repo` repository into `<name>.d/`.
fn refresh_external_repos(root: &Path) -> Result<bool> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Ok(false);
    };
    let mut changed = false;
    for entry in rd.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("repo") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // deb-get owns 00-builtin (script-internal) and 99-local (user).
        if matches!(stem, "00-builtin" | "99-local") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let mut lines = text.lines();
        let url = lines.next().unwrap_or("").trim().to_string();
        if !url.starts_with("https://") {
            eprintln!("  ⚠ skipping {stem}: repository URL is not https ({url})");
            continue;
        }
        let dst = root.join(format!("{stem}.d"));
        std::fs::create_dir_all(&dst).ok();
        if url.contains("github.com") {
            changed |= refresh_from_github(&url, stem, &dst)?;
        } else {
            let base = url.trim_end_matches('/');
            for rel in lines
                .map(|l| l.trim().trim_start_matches('#'))
                // Absolute URLs in the manifest body are refused: only
                // relative package paths are fetched from the https base.
                .filter(|l| !l.is_empty() && !l.contains("://"))
            {
                let file_url = format!("{base}/packages/{rel}");
                let name = rel.rsplit('/').next().unwrap_or(rel);
                let bytes = fetch_bytes(&file_url)?;
                if write_if_changed(&dst.join(name), &bytes)? {
                    changed = true;
                }
            }
        }
    }
    Ok(changed)
}

/// Fetch a GitHub-hosted deb-get repo's `packages/*` from its tarball.
fn refresh_from_github(url: &str, repo_name: &str, dst: &Path) -> Result<bool> {
    // Layout: https://github.com/<owner>/<repo>/tree/<branch>[/...]
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    if parts.len() < 7 || parts[5] != "tree" {
        bail!("invalid GitHub repo URL: {url}");
    }
    let (owner, repo, branch) = (parts[3], parts[4], parts[6]);
    for seg in [owner, repo, branch] {
        if !seg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            bail!("unsafe GitHub URL segment '{seg}' in {url}");
        }
    }
    let api = format!("https://api.github.com/repos/{owner}/{repo}/tarball/{branch}");
    let reader = fetch_reader(&api)?;
    let gz = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(gz);
    let mut changed = false;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let comps: Vec<std::ffi::OsString> = path
            .components()
            .map(|c| c.as_os_str().to_owned())
            .collect();
        // `<top>/<repo_name>/packages/<file>` — nothing deeper.
        let Some(i) = comps.iter().position(|c| c == repo_name) else {
            continue;
        };
        if comps.get(i + 1).map(|c| c == "packages") != Some(true) || i + 3 != comps.len() {
            continue;
        }
        let Some(name) = comps.get(i + 2) else {
            continue;
        };
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf)?;
        if write_if_changed(&dst.join(name), &buf)? {
            changed = true;
        }
    }
    Ok(changed)
}

/// Write `bytes` only when they differ from what's on disk; `Ok(true)` when
/// the file was (re)written.
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<bool> {
    if std::fs::read(path).map(|old| old == bytes).unwrap_or(false) {
        return Ok(false);
    }
    match std::fs::write(path, bytes) {
        Ok(()) => Ok(true),
        // deb-get's `/etc/deb-get` is root-owned; elevate like deb-get does.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            write_sudo(path, bytes)?;
            Ok(true)
        }
        Err(e) => Err(e).with_context(|| format!("writing '{}'", path.display())),
    }
}

fn fetch_reader(url: &str) -> Result<Box<dyn std::io::Read + Send>> {
    lx_lib::checksum::RawGetter::raw_get(&PlainGetter::new()?, url)
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("failed to run `git {}`", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_matching_handles_aliases() {
        assert!(deb_matches_arch("app_1.0_amd64.deb", "amd64"));
        assert!(deb_matches_arch("app_1.0_x86_64.deb", "amd64"));
        assert!(deb_matches_arch("app_1.0_arm64.deb", "arm64"));
        assert!(deb_matches_arch("app_1.0_aarch64.deb", "arm64"));
        assert!(!deb_matches_arch("app_1.0_arm64.deb", "amd64"));
        // i386 must not match x86_64's "x86".
        assert!(!deb_matches_arch("app_1.0_x86_64.deb", "i386"));
        assert!(deb_matches_arch("app_1.0_i386.deb", "i386"));
    }

    #[test]
    fn version_from_deb_name() {
        assert_eq!(
            version_from_name("app_1.2.3-1_amd64.deb").as_deref(),
            Some("1.2.3-1")
        );
        assert_eq!(version_from_name("app.deb"), None);
    }

    #[test]
    fn scrape_finds_deb_links() {
        let html = r#"<a href="https://x/app_1.0_amd64.deb">d</a>
                      <a href="https://x/nope.txt">t</a>"#;
        assert_eq!(scrape_deb_links(html), vec!["https://x/app_1.0_amd64.deb"]);
    }

    #[test]
    fn dearmor_strips_armor() {
        // "hello" base64, wrapped in fake armor.
        let asc =
            b"-----BEGIN PGP PUBLIC KEY BLOCK-----\naGVsbG8=\n-----END PGP PUBLIC KEY BLOCK-----\n";
        assert_eq!(dearmor(asc).unwrap(), b"hello");
    }

    #[test]
    fn search_reads_a_catalog_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("01-main.d")).unwrap();
        std::fs::write(
            root.join("01-main.d/example"),
            "DEFVER=1\nURL=\"https://example.com/example_1.0_amd64.deb\"\nPRETTY_NAME=\"Example\"\nWEBSITE=\"https://example.com\"\nSUMMARY=\"An example app\"\n",
        )
        .unwrap();
        let src = DebGetSource::with_root("debget-test", root.to_path_buf());
        let hits = src.search(Some("example")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "example");
        assert!(hits[0].description.contains("example app"));
        let hit = src.info("example").unwrap().unwrap();
        assert_eq!(hit.available, vec!["direct"]);
    }
}
