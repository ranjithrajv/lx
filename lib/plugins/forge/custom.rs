// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use lx_lib::github::{Release, ReleaseMeta, RepoLicense};

use super::ForgeSource;

/// Direct-URL provider (`source: custom`).
///
/// No forge API: `upstream_url` in package.yaml is a URL template expanded
/// per build with `{version}`, `{arch}`, and `{package_name}` placeholders
/// (e.g. `https://example.com/releases/{version}/app-{arch}.tar.gz`).
/// Each architecture resolves to exactly one download URL, so there is no
/// release listing or auto-discovery — `architectures:` must name the archs
/// (a plain list is enough) and every entry maps to its expanded URL.
///
/// Checksum: no sidecar probing (there is no release to probe); verification
/// is via `--pinned-metadata` / `package.lock` exactly like other providers,
/// falling back to `--allow-unverified` / `--no-verify`.
pub struct CustomForgeSource;

fn expand_template(template: &str, version: &str, arch: &str, package: &str) -> String {
    template
        .replace("{version}", version)
        .replace("{arch}", arch)
        .replace("{package_name}", package)
}

/// Expand `template` for one arch and split into (asset_name, url).
pub fn expand_for_arch(
    template: &str,
    version: &str,
    arch: &str,
    package: &str,
) -> (String, String) {
    let url = expand_template(template, version, arch, package);
    let name = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("payload")
        .split('?')
        .next()
        .unwrap_or("payload")
        .to_string();
    (name, url)
}

/// Build a synthetic release for `version`, one asset per arch, from the
/// template. `archs` comes from the caller's effective architecture list.
pub fn synthetic_release(
    template: &str,
    version: &str,
    package: &str,
    archs: &[String],
) -> Release {
    use lx_lib::github::Asset;
    let assets = archs
        .iter()
        .map(|arch| {
            let (name, url) = expand_for_arch(template, version, arch, package);
            Asset {
                name,
                size: None,
                browser_download_url: url,
                checksums: Default::default(),
            }
        })
        .collect();
    Release {
        tag_name: version.to_string(),
        html_url: template.to_string(),
        assets,
        published_at: None,
        prerelease: false,
        draft: false,
        body: None,
    }
}

impl ForgeSource for CustomForgeSource {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn description(&self) -> &'static str {
        "Direct URL template (upstream_url with {version}/{arch}) — no forge API"
    }

    fn token_env(&self) -> Option<&'static str> {
        None
    }

    fn parse_url(&self, _url: &str) -> Option<String> {
        None
    }

    fn latest_release(
        &self,
        _repo: &str,
        _token: Option<&str>,
        _cache_dir: Option<&Path>,
    ) -> Result<Release> {
        Err(anyhow::anyhow!(
            "source 'custom' has no latest release: set version: in package.yaml or --version (upstream_url template is expanded with it)"
        ))
    }

    fn release_by_tag(
        &self,
        _repo: &str,
        tag: &str,
        _token: Option<&str>,
        _cache_dir: Option<&Path>,
    ) -> Result<Release> {
        // Tag known, but arch list lives in the caller (build.rs expands per
        // arch). Return an empty-asset release carrying the tag so callers
        // that only need the tag still work; build.rs builds the real asset
        // map itself via `synthetic_release`.
        Ok(Release {
            tag_name: tag.to_string(),
            html_url: String::new(),
            assets: Vec::new(),
            published_at: None,
            prerelease: false,
            draft: false,
            body: None,
        })
    }

    fn releases(
        &self,
        _repo: &str,
        _per_page: u8,
        _token: Option<&str>,
        _cache_dir: Option<&Path>,
    ) -> Result<Vec<ReleaseMeta>> {
        Ok(Vec::new())
    }

    fn repo_license(
        &self,
        _repo: &str,
        _token: Option<&str>,
        _cache_dir: Option<&Path>,
    ) -> Result<Option<RepoLicense>> {
        Ok(None)
    }

    fn raw_get(&self, url: &str, _token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = crate::http::new_client()?;
        let resp = crate::http::send_get_with_retry(&client, url, None)
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            anyhow::bail!("HTTP {} for {url}", resp.status());
        }
        Ok(Box::new(resp))
    }
}

use anyhow::Context;
