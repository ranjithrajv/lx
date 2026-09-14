// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};

/// GitHub client using blocking `reqwest` against the REST API directly —
/// no async runtime. Mirrors [`crate::gitlab::GitlabClient`]'s shape and
/// shares the 5-minute JSON API cache, so every source-provider client in
/// lx is synchronous.
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
        let resp = crate::http::send_get_with_retry_headers(&self.http, url, &headers)
            .with_context(|| format!("GET {url} failed"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(match status.as_u16() {
                404 => anyhow!("not found on GitHub: {body}"),
                403 => {
                    anyhow!("GitHub API rate limit hit (403). Set GITHUB_TOKEN to raise the limit.")
                }
                _ => anyhow!("GitHub API {status} for {url}: {body}"),
            });
        }
        resp.json::<T>()
            .with_context(|| format!("failed to parse JSON from {url}"))
    }

    pub fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        let resp = crate::http::send_get_with_retry(&self.http, url, self.auth_header())
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {} for {url}", resp.status()));
        }
        Ok(Box::new(resp))
    }

    pub fn release_by_tag(&self, owner: &str, repo: &str, tag: &str) -> Result<Release> {
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

    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        crate::cache::ApiCache::new(self.api_cache_dir.clone()).get_or_fetch(key, fetch)
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
                    name: a.name,
                    size: a.size,
                    browser_download_url: a.browser_download_url,
                })
                .collect(),
            // GitHub reports RFC3339; reuse gitlab.rs's generic ISO8601 parser.
            published_at: r
                .published_at
                .as_deref()
                .and_then(crate::gitlab::parse_gitlab_time),
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

/// Minimal release metadata for suggestions.
#[derive(Debug, Clone)]
pub struct ReleaseMeta {
    pub tag: String,
    pub published_at: Option<String>,
}

/// Repository license metadata (SPDX id + optional full text).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoLicense {
    pub spdx: String,
    pub text: Option<String>,
}

/// Domain model for a GitHub release (the subset lx needs, normalized so
/// every forge source shares one release type).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub prerelease: bool,
    pub draft: bool,
    pub html_url: String,
    pub assets: Vec<Asset>,
    /// Unix epoch seconds the release was published, when GitHub reports
    /// one. Used as the reproducible-build timestamp source for generated
    /// package metadata (changelog date, copyright year) instead of
    /// wall-clock build time, so the same release always produces the same
    /// bytes regardless of when it's built.
    pub published_at: Option<i64>,
    /// The release's own markdown notes, when GitHub reports any. Used as
    /// a ready-made "changelog" for `lx update --diff` instead of
    /// deriving one from commits/tags.
    #[serde(default)]
    pub body: Option<String>,
}

/// Domain model for a release asset (the subset lx needs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: Option<u64>,
    pub browser_download_url: String,
}
