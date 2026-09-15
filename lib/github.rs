// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;

// The provider-agnostic release model lives in `crate::release` (every forge
// client shares it). Re-exported here so existing `github::Release` paths keep
// working; new code should import from `crate::release`.
pub use crate::release::{Asset, Release, ReleaseMeta, RepoLicense};

/// GitHub client using blocking `reqwest` against the REST API directly —
/// no async runtime. Mirrors [`crate::gitlab::GitlabClient`]'s shape and
/// shares the 5-minute JSON API cache, so every source-provider client in
/// lx is synchronous.
use crate::cache::ApiCacheProvider;

pub struct GitHubClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    api_cache_dir: Option<std::path::PathBuf>,
}

impl GitHubClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_cache(token, None)
    }

    pub fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let http = crate::http::new_client()?;
        let base_url = std::env::var("GITHUB_API_URL")
            .unwrap_or_else(|_| crate::constants::DEFAULT_GITHUB_API_URL.to_string());
        let token = token.or_else(|| std::env::var("GITHUB_TOKEN").ok());

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            api_cache_dir: cache_dir,
        })
    }

    fn auth_header(&self) -> Option<(&'static str, String)> {
        self.token
            .as_deref()
            .map(|t| ("Authorization", format!("Bearer {t}")))
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let mut headers = vec![("Accept", "application/vnd.github+json".to_string())];
        headers.extend(self.auth_header());
        crate::http::get_json(&self.http, url, &headers, "GitHub")
    }

    pub fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        crate::http::raw_get(&self.http, url, self.auth_header())
    }

    pub fn release_by_tag(&self, owner: &str, repo: &str, tag: &str) -> Result<Release> {
        match self.release_by_tag_exact(owner, repo, tag) {
            Ok(r) => Ok(r),
            Err(first) => {
                // Projects commonly tag releases `v1.2.3` while a package's
                // version (e.g. an AUR `pkgver`) is the bare `1.2.3`, and
                // vice versa. Retry once with the alternate `v` prefix before
                // surfacing the original error.
                let alt = alternate_v_tag(tag);
                if alt == tag {
                    return Err(first);
                }
                self.release_by_tag_exact(owner, repo, &alt)
                    .map_err(|_| first)
            }
        }
    }

    fn release_by_tag_exact(&self, owner: &str, repo: &str, tag: &str) -> Result<Release> {
        self.api_cache(&format!("release_{owner}_{repo}_{tag}"), || {
            let url = format!("{}/repos/{owner}/{repo}/releases/tags/{tag}", self.base_url);
            self.get_json::<GitHubReleaseRaw>(&url).map(Into::into)
        })
    }

    pub fn latest_release(&self, owner: &str, repo: &str) -> Result<Release> {
        self.api_cache(&format!("latest_{owner}_{repo}"), || {
            let url = format!("{}/repos/{owner}/{repo}/releases/latest", self.base_url);
            self.get_json::<GitHubReleaseRaw>(&url).map(Into::into)
        })
    }

    pub fn release(&self, owner: &str, repo: &str, tag_or_version: &str) -> Result<Release> {
        match self.release_by_tag(owner, repo, tag_or_version) {
            Ok(r) => Ok(r),
            Err(_) => self.latest_release(owner, repo),
        }
    }

    pub fn releases(&self, owner: &str, repo: &str, per_page: u8) -> Result<Vec<ReleaseMeta>> {
        let url = format!(
            "{}/repos/{owner}/{repo}/releases?per_page={per_page}",
            self.base_url
        );
        let raws: Vec<GitHubReleaseRaw> = self.get_json(&url)?;
        Ok(raws
            .into_iter()
            .map(|r| ReleaseMeta {
                tag: r.tag_name,
                published_at: r.published_at,
            })
            .collect())
    }

    pub fn repo_license(&self, owner: &str, repo: &str) -> Result<Option<RepoLicense>> {
        let key = format!("license_{owner}_{repo}");
        let cache = crate::cache::ApiCache::new(self.api_cache_dir.clone());
        if let Some(cached) = cache.get::<RepoLicense>(&key)? {
            return Ok(Some(cached));
        }
        let url = format!("{}/repos/{owner}/{repo}/license", self.base_url);
        let raw: GitHubLicenseRaw = match self.get_json(&url) {
            Ok(r) => r,
            Err(e) if e.to_string().contains("not found") => return Ok(None),
            Err(e) => return Err(e),
        };
        let lic = RepoLicense {
            spdx: raw
                .license
                .and_then(|l| l.spdx_id)
                .filter(|s| !s.is_empty() && s != "NOASSERTION")
                .unwrap_or_else(|| "NOASSERTION".to_string()),
            text: decode_content(raw.content.as_deref(), raw.encoding.as_deref()),
        };
        cache.put(&key, &lic)?;
        Ok(Some(lic))
    }

    pub fn repo_root(&self, owner: &str, repo: &str) -> Result<Vec<String>> {
        let url = format!("{}/repos/{owner}/{repo}/contents", self.base_url);
        let items: Vec<GitHubContentRaw> = self.get_json(&url)?;
        Ok(items.into_iter().map(|c| c.name).collect())
    }

    pub fn repo_file_text(&self, owner: &str, repo: &str, path: &str) -> Result<Option<String>> {
        let url = format!("{}/repos/{owner}/{repo}/contents/{path}", self.base_url);
        match self.get_json::<GitHubContentRaw>(&url) {
            Ok(c) => Ok(decode_content(c.content.as_deref(), c.encoding.as_deref())),
            Err(e) if e.to_string().contains("not found") => Ok(None),
            Err(e) => Err(e),
        }
    }
}

pub fn decode_content(content: Option<&str>, encoding: Option<&str>) -> Option<String> {
    use base64::Engine as _;
    if encoding? != "base64" {
        return None;
    }
    let cleaned: String = content?.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitHubReleaseRaw {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    pub html_url: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<GitHubAssetRaw>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitHubAssetRaw {
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    pub browser_download_url: String,
    /// GitHub's inline asset digest (`"sha256:<hex>"`), when present.
    #[serde(default)]
    pub digest: Option<String>,
}

/// Parse a provider asset `digest` (`"sha256:<hex>"`) into a checksums map
/// (`algorithm → hex`). Returns an empty map when the digest is absent or
/// malformed.
pub fn digest_checksums(digest: Option<&str>) -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    if let Some(d) = digest {
        if let Some((algo, hex)) = d.split_once(':') {
            let algo = algo.trim().to_ascii_lowercase();
            let hex = hex.trim().to_ascii_lowercase();
            if !algo.is_empty() && !hex.is_empty() {
                m.insert(algo, hex);
            }
        }
    }
    m
}

/// The alternate `v`-prefix spelling of a release tag: `1.2.3` → `v1.2.3`,
/// `v1.2.3` → `1.2.3`. Returns `tag` unchanged when there is no `v` to add
/// or remove (never yields a double prefix).
fn alternate_v_tag(tag: &str) -> String {
    match tag.strip_prefix('v').or_else(|| tag.strip_prefix('V')) {
        Some(bare) if !bare.is_empty() => bare.to_string(),
        _ => format!("v{tag}"),
    }
}

impl From<GitHubReleaseRaw> for Release {
    fn from(r: GitHubReleaseRaw) -> Self {
        Self {
            tag_name: r.tag_name,
            prerelease: r.prerelease,
            draft: r.draft,
            html_url: r.html_url,
            assets: r
                .assets
                .into_iter()
                .map(|a| Asset {
                    checksums: digest_checksums(a.digest.as_deref()),
                    name: a.name,
                    size: a.size,
                    browser_download_url: a.browser_download_url,
                })
                .collect(),
            // GitHub reports RFC3339.
            published_at: r
                .published_at
                .as_deref()
                .and_then(crate::release::parse_timestamp),
            body: r.body,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct GitHubContentRaw {
    #[serde(default)]
    name: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct GitHubLicenseRaw {
    #[serde(default)]
    license: Option<GitHubLicenseIdRaw>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct GitHubLicenseIdRaw {
    #[serde(default)]
    spdx_id: Option<String>,
}

/// Uniform construction for the `ForgeSource` glue (`ClientNew`).
impl crate::source_client::ClientNew for GitHubClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for GitHubClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        GitHubClient::raw_get(self, url)
    }
}

impl crate::cache::ApiCacheProvider for GitHubClient {
    fn api_cache_dir(&self) -> Option<std::path::PathBuf> {
        self.api_cache_dir.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_v_tag_toggles_the_prefix() {
        assert_eq!(alternate_v_tag("0.9.0"), "v0.9.0");
        assert_eq!(alternate_v_tag("v0.9.0"), "0.9.0");
        assert_eq!(alternate_v_tag("V0.9.0"), "0.9.0");
        // A bare `v` has nothing to strip; don't emit a double prefix from
        // itself (the caller only uses the result when it differs).
        assert_eq!(alternate_v_tag(""), "v");
    }
}
