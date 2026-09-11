use anyhow::{anyhow, Context, Result};
use octocrab::Octocrab;
use std::sync::Arc;
use tokio::runtime::Runtime;

/// GitHub client backed by octocrab (the ecosystem-standard GitHub API
/// client), exposing the small slice of release/asset metadata the CLI needs.
///
/// octocrab is async, so a single-threaded tokio runtime is created up front
/// and each call is `block_on`'d. A blocking reqwest client is retained only
/// for streaming raw asset/checksum downloads.
pub struct GitHubClient {
    crab: Arc<Octocrab>,
    http: reqwest::blocking::Client,
    runtime: Runtime,
    /// Optional JSON API cache dir (mirrors the action's
    /// `/tmp/github_api_cache`): release/license responses are cached for
    /// 5 minutes to avoid GitHub rate limits on repeated runs.
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
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to create tokio runtime")?;

        // octocrab spawns a background tower-buffer worker at build time, so
        // the client must be constructed from within the runtime context.
        let crab = runtime
            .block_on(async {
                let mut builder = Octocrab::builder();
                if let Some(token) = token {
                    builder = builder.personal_token(token);
                }
                if let Ok(base) = std::env::var("GITHUB_API_URL") {
                    builder = builder
                        .base_uri(&base)
                        .with_context(|| format!("invalid GITHUB_API_URL: {base}"))?;
                }
                builder.build().context("failed to build octocrab client")
            })
            .map_err(|e: anyhow::Error| e)?;

        let http = crate::http::new_client()?;

        Ok(Self {
            crab: Arc::new(crab),
            http,
            runtime,
            api_cache_dir: cache_dir,
        })
    }

    fn map_error(e: octocrab::Error) -> anyhow::Error {
        match &e {
            octocrab::Error::GitHub { source, .. } => match source.status_code.as_u16() {
                404 => anyhow!("not found on GitHub: {}", source.message),
                403 => {
                    anyhow!("GitHub API rate limit hit (403). Set GITHUB_TOKEN to raise the limit.")
                }
                _ => anyhow!("GitHub API {}: {}", source.status_code, source.message),
            },
            _ => anyhow!("GitHub API error: {e}"),
        }
    }

    fn block<T>(&self, fut: impl std::future::Future<Output = octocrab::Result<T>>) -> Result<T> {
        self.runtime.block_on(fut).map_err(Self::map_error)
    }

    /// Fetch a specific release by tag.
    pub fn release_by_tag(&self, owner: &str, repo: &str, tag: &str) -> Result<Release> {
        let crab = Arc::clone(&self.crab);
        let (o, r, t) = (owner.to_string(), repo.to_string(), tag.to_string());
        self.api_cache(&format!("release_{o}_{r}_{t}"), || {
            let crab = Arc::clone(&crab);
            let (o, r, t) = (o.clone(), r.clone(), t.clone());
            self.block(async move { crab.repos(&o, &r).releases().get_by_tag(&t).await })
                .map(Into::into)
        })
    }

    /// Fetch the latest (non-draft) release.
    pub fn latest_release(&self, owner: &str, repo: &str) -> Result<Release> {
        let crab = Arc::clone(&self.crab);
        let (o, r) = (owner.to_string(), repo.to_string());
        self.api_cache(&format!("latest_{o}_{r}"), || {
            let crab = Arc::clone(&crab);
            let (o, r) = (o.clone(), r.clone());
            self.block(async move { crab.repos(&o, &r).releases().get_latest().await })
                .map(Into::into)
        })
    }

    /// Fetch release by tag, falling back to the latest release if the tag
    /// resolves to a version rather than a tag (heuristic).
    /// Resolve a release by exact tag/version, or the latest release when the
    /// tag is not found. Callers that need to distinguish a version miss
    /// (to suggest recent releases) should use `release_by_tag` directly.
    pub fn release(&self, owner: &str, repo: &str, tag_or_version: &str) -> Result<Release> {
        match self.release_by_tag(owner, repo, tag_or_version) {
            Ok(r) => Ok(r),
            Err(_) => self.latest_release(owner, repo),
        }
    }

    /// Raw (non-JSON) GET for streaming a download, returning a readable body.
    pub fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        let resp = crate::http::send_get_with_retry(&self.http, url, None)
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {} for {url}", resp.status()));
        }
        Ok(Box::new(resp))
    }

    /// Fetch the repo's license metadata (SPDX id + full license text),
    /// mirroring the action's `fetch_upstream_license`. Returns None when
    /// GitHub can't detect a license (404 — a normal, non-fatal case).
    pub fn repo_license(&self, owner: &str, repo: &str) -> Result<Option<RepoLicense>> {
        let crab = Arc::clone(&self.crab);
        let (o, r) = (owner.to_string(), repo.to_string());
        let key = format!("license_{o}_{r}");
        // Cache the Some case only (a 404 is cheap and is the common miss).
        let cache = crate::cache::ApiCache::new(self.api_cache_dir.clone());
        if let Some(cached) = cache.get::<RepoLicense>(&key)? {
            return Ok(Some(cached));
        }
        let content = match self.block(async move { crab.repos(&o, &r).license().await }) {
            Ok(c) => c,
            Err(e) if e.to_string().contains("404") => return Ok(None),
            Err(e) => return Err(e),
        };
        let lic = RepoLicense {
            spdx: content
                .license
                .as_ref()
                .map(|l| l.spdx_id.clone())
                .filter(|s| !s.is_empty() && s != "NOASSERTION")
                .unwrap_or_else(|| "NOASSERTION".to_string()),
            text: content.decoded_content(),
        };
        cache.put(&key, &lic)?;
        Ok(Some(lic))
    }

    /// Read a cached API response (5-minute TTL) or run `fetch`, caching the
    /// result. Mirrors the action's github-api.sh cache semantics.
    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        crate::cache::ApiCache::new(self.api_cache_dir.clone()).get_or_fetch(key, fetch)
    }

    /// List the repository root (for the dual-license detection used to
    /// render a multi-license copyright file).
    pub fn repo_root(&self, owner: &str, repo: &str) -> Result<Vec<String>> {
        let crab = Arc::clone(&self.crab);
        let owner = owner.to_string();
        let repo = repo.to_string();
        let items =
            self.block(async move { crab.repos(&owner, &repo).get_content().send().await })?;
        Ok(items.items.into_iter().map(|c| c.name).collect())
    }

    /// Fetch a single file's raw text from the repo root (for LICENSE-*
    /// contents used by the dual-license detection).
    pub fn repo_file_text(&self, owner: &str, repo: &str, path: &str) -> Result<Option<String>> {
        let crab = Arc::clone(&self.crab);
        let owner = owner.to_string();
        let repo = repo.to_string();
        let path = path.to_string();
        let mut items = self.block(async move {
            crab.repos(&owner, &repo)
                .get_content()
                .path(&path)
                .send()
                .await
        })?;
        Ok(items.take_items().pop().and_then(|c| c.decoded_content()))
    }
    /// List the latest `per_page` releases (tag_name, published_at),
    /// used for version-miss suggestions.
    pub fn releases(&self, owner: &str, repo: &str, per_page: u8) -> Result<Vec<ReleaseMeta>> {
        let crab = Arc::clone(&self.crab);
        let owner = owner.to_string();
        let repo = repo.to_string();
        let page = self.block(async move {
            crab.repos(&owner, &repo)
                .releases()
                .list()
                .per_page(per_page)
                .send()
                .await
        })?;
        Ok(page
            .items
            .into_iter()
            .map(|r| ReleaseMeta {
                tag: r.tag_name,
                published_at: r.published_at.map(|t| t.to_string()),
            })
            .collect())
    }
}

/// GitHub client using plain blocking `reqwest` against the REST API
/// directly — no octocrab, no tokio runtime. Second option alongside
/// [`GitHubClient`] for callers that don't want an async runtime pulled in
/// just to talk to GitHub; mirrors [`crate::gitlab::GitlabClient`]'s shape.
pub struct GitHubSyncClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    api_cache_dir: Option<std::path::PathBuf>,
}

impl GitHubSyncClient {
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

/// Uniform construction for the `SourcePlugin` glue (`ClientNew`).
impl crate::source_client::ClientNew for GitHubSyncClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for GitHubSyncClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        GitHubSyncClient::raw_get(self, url)
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

/// Domain model for a GitHub release (subset of octocrab's `Release`).
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

impl From<octocrab::models::repos::Release> for Release {
    fn from(r: octocrab::models::repos::Release) -> Self {
        Self {
            tag_name: r.tag_name,
            prerelease: r.prerelease,
            draft: r.draft,
            html_url: r.html_url.to_string(),
            assets: r.assets.into_iter().map(Into::into).collect(),
            published_at: r.published_at.map(|t| t.timestamp()),
            body: r.body,
        }
    }
}

/// Domain model for a release asset (subset of octocrab's `Asset`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: Option<u64>,
    pub browser_download_url: String,
}

impl From<octocrab::models::repos::Asset> for Asset {
    fn from(a: octocrab::models::repos::Asset) -> Self {
        Self {
            name: a.name,
            size: Some(a.size.max(0) as u64),
            browser_download_url: a.browser_download_url.to_string(),
        }
    }
}

/// Uniform construction for the `SourcePlugin` glue (`ClientNew`).
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
