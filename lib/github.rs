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

        let http = reqwest::blocking::Client::builder()
            .user_agent("lpt/0.1 (latest package tool)")
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .context("failed to build HTTP client")?;

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

    /// List all releases (for auto-discovery across recent versions).
    #[allow(dead_code)] // used by discover --versions
    pub fn list_releases(&self, owner: &str, repo: &str, per_page: usize) -> Result<Vec<Release>> {
        let crab = Arc::clone(&self.crab);
        let (o, r) = (owner.to_string(), repo.to_string());
        self.api_cache(&format!("list_{o}_{r}_{per_page}"), || {
            let crab = Arc::clone(&crab);
            let (o, r) = (o.clone(), r.clone());
            let page = self.block(async move {
                crab.repos(&o, &r)
                    .releases()
                    .list()
                    .per_page(per_page.min(100) as u8)
                    .send()
                    .await
            })?;
            Ok(page.items.into_iter().map(Into::into).collect())
        })
    }

    /// Raw (non-JSON) GET for streaming a download, returning a readable body.
    pub fn raw_get(&self, url: &str) -> Result<impl std::io::Read> {
        let resp = self
            .http
            .get(url)
            .send()
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {} for {url}", resp.status()));
        }
        Ok(resp)
    }

    /// Get the git ref of the repository default branch (used for
    /// zero-config discovery).
    #[allow(dead_code)] // reserved for zero-config / --ad parity
    pub fn repo_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let crab = Arc::clone(&self.crab);
        let repo = self.block(async move { crab.repos(owner, repo).get().await })?;
        repo.default_branch
            .ok_or_else(|| anyhow!("repository has no default branch"))
    }

    /// Fetch the repo's license metadata (SPDX id + full license text),
    /// mirroring the action's `fetch_upstream_license`. Returns None when
    /// GitHub can't detect a license (404 — a normal, non-fatal case).
    pub fn repo_license(&self, owner: &str, repo: &str) -> Result<Option<RepoLicense>> {
        let crab = Arc::clone(&self.crab);
        let (o, r) = (owner.to_string(), repo.to_string());
        let key = format!("license_{o}_{r}");
        // Cache the Some case only (a 404 is cheap and is the common miss).
        if let Some(cached) = self.api_cache_get(&key)? {
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
        self.api_cache_put(&key, &lic)?;
        Ok(Some(lic))
    }

    /// Read a cached API response (5-minute TTL) or run `fetch`, caching the
    /// result. Mirrors the action's github-api.sh cache semantics.
    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        if let Some(hit) = self.api_cache_get(key)? {
            return Ok(hit);
        }
        let value = fetch()?;
        self.api_cache_put(key, &value)?;
        Ok(value)
    }

    /// Read a cached value if present and fresh (<300s), else None.
    fn api_cache_get<T>(&self, key: &str) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let Some(dir) = &self.api_cache_dir else {
            return Ok(None);
        };
        let path = dir.join(format!("{key}.json"));
        let Ok(meta) = std::fs::metadata(&path) else {
            return Ok(None);
        };
        let Ok(modified) = meta.modified() else {
            return Ok(None);
        };
        let age = std::time::SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default();
        if age > std::time::Duration::from_secs(300) {
            return Ok(None);
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        match serde_json::from_str(&text) {
            Ok(v) => Ok(Some(v)),
            Err(_) => Ok(None),
        }
    }

    /// Write a value to the API cache (best-effort; a full cache dir is not
    /// fatal).
    fn api_cache_put<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let Some(dir) = &self.api_cache_dir else {
            return Ok(());
        };
        let _ = std::fs::create_dir_all(dir);
        let json = serde_json::to_string(value)?;
        let path = dir.join(format!("{key}.json"));
        let tmp = dir.join(format!("{key}.json.tmp"));
        let _ = std::fs::write(&tmp, json);
        let _ = std::fs::rename(&tmp, &path);
        Ok(())
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
    #[allow(dead_code)] // surfaced in --full discovery output
    pub name: Option<String>,
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
}

impl From<octocrab::models::repos::Release> for Release {
    fn from(r: octocrab::models::repos::Release) -> Self {
        Self {
            tag_name: r.tag_name,
            name: r.name,
            prerelease: r.prerelease,
            draft: r.draft,
            html_url: r.html_url.to_string(),
            assets: r.assets.into_iter().map(Into::into).collect(),
            published_at: r.published_at.map(|t| t.timestamp()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_octocrab_release() {
        let json = r#"{
            "url": "https://api.github.com/repos/eza-community/eza/releases/1",
            "html_url": "https://github.com/eza-community/eza/releases/tag/v0.24.0",
            "assets_url": "https://api.github.com/repos/eza-community/eza/releases/1/assets",
            "upload_url": "https://uploads.github.com/...",
            "tarball_url": null,
            "zipball_url": null,
            "id": 1,
            "node_id": "RE_1",
            "tag_name": "v0.24.0",
            "target_commitish": "main",
            "name": "eza v0.24.0",
            "body": null,
            "draft": false,
            "prerelease": false,
            "created_at": "2025-01-01T00:00:00Z",
            "published_at": "2025-01-01T00:00:00Z",
            "assets": [
                {
                    "url": "https://api.github.com/.../assets/10",
                    "browser_download_url": "https://github.com/.../eza_x86_64-unknown-linux-gnu.tar.gz",
                    "id": 10,
                    "node_id": "RA_10",
                    "name": "eza_x86_64-unknown-linux-gnu.tar.gz",
                    "label": null,
                    "state": "uploaded",
                    "content_type": "application/gzip",
                    "size": 123,
                    "download_count": 5,
                    "created_at": "2025-01-01T00:00:00Z",
                    "updated_at": "2025-01-01T00:00:00Z"
                }
            ]
        }"#;
        let octo: octocrab::models::repos::Release = serde_json::from_str(json).unwrap();
        let r: Release = octo.into();
        assert_eq!(r.tag_name, "v0.24.0");
        assert_eq!(r.assets.len(), 1);
        assert_eq!(r.assets[0].name, "eza_x86_64-unknown-linux-gnu.tar.gz");
        assert_eq!(r.assets[0].size, Some(123));
        assert_eq!(r.published_at, Some(1_735_689_600)); // 2025-01-01T00:00:00Z
        assert!(!r.prerelease);
        assert!(!r.draft);
    }

    #[test]
    fn release_from_octocrab_maps_prerelease_and_draft() {
        let json = r#"{
            "url": "https://api.github.com/repos/eza-community/eza/releases/2",
            "html_url": "https://github.com/eza-community/eza/releases/tag/v0.25.0-rc1",
            "assets_url": "https://api.github.com/repos/eza-community/eza/releases/2/assets",
            "upload_url": "https://uploads.github.com/...",
            "tarball_url": null,
            "zipball_url": null,
            "id": 2,
            "node_id": "RE_2",
            "tag_name": "v0.25.0-rc1",
            "target_commitish": "main",
            "name": "eza v0.25.0-rc1",
            "body": null,
            "draft": true,
            "prerelease": true,
            "created_at": "2025-01-01T00:00:00Z",
            "published_at": null,
            "assets": []
        }"#;
        let octo: octocrab::models::repos::Release = serde_json::from_str(json).unwrap();
        let r: Release = octo.into();
        assert!(r.prerelease);
        assert!(r.draft);
    }
}
