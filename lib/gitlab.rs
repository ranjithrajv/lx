use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::github::{Asset, Release, ReleaseMeta, RepoLicense};

/// GitLab API client mirroring `GitHubClient`'s surface.
/// Uses blocking `reqwest` (no tokio/octocrab) and the same 5-minute JSON cache.
pub struct GitlabClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    api_cache_dir: Option<PathBuf>,
}

impl GitlabClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_cache(token, None)
    }

    pub fn with_cache(token: Option<String>, cache_dir: Option<PathBuf>) -> Result<Self> {
        let http = crate::http::new_client()?;

        // GITHUB_API_URL equivalent for GitLab: GITLAB_API_URL or GITLAB_HOST.
        let base_url = std::env::var("GITLAB_API_URL")
            .ok()
            .or_else(|| {
                std::env::var("GITLAB_HOST")
                    .ok()
                    .map(|h| format!("https://{}/api/v4", h.trim_end_matches('/')))
            })
            .unwrap_or_else(|| crate::constants::DEFAULT_GITLAB_API_URL.to_string());

        // Token precedence: explicit arg > GITLAB_TOKEN env > None
        let token = token.or_else(|| std::env::var("GITLAB_TOKEN").ok());

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            api_cache_dir: cache_dir,
        })
    }

    pub fn project_encode(repo: &str) -> String {
        // repo is "owner/repo" -> encode "/" as %2F and other chars
        // We avoid extra crate by simple percent-encoding for "/" and use
        // urlencoding for safety: manual replace is sufficient for owner/repo.
        // For correctness, also encode any other special chars via naive approach.
        repo.replace('/', "%2F")
    }

    fn auth_header(&self) -> Option<(&'static str, String)> {
        self.token
            .as_deref()
            .map(|t| ("PRIVATE-TOKEN", t.to_string()))
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = crate::http::send_get_with_retry(&self.http, url, self.auth_header())
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            // Map 404 to "not found" similar to GitHub
            if status.as_u16() == 404 {
                return Err(anyhow!("not found on GitLab: {body}"));
            }
            if status.as_u16() == 403 || status.as_u16() == 401 {
                return Err(anyhow!(
                    "GitLab API authentication failed ({status}). Set GITLAB_TOKEN."
                ));
            }
            return Err(anyhow!("GitLab API {status} for {url}: {body}"));
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
        let full = format!("{owner}/{repo}");
        self.api_cache(&format!("gitlab_release_{full}_{tag}"), || {
            let id = Self::project_encode(&full);
            let encoded_tag = urlencoding_encode(tag);
            let url = format!("{}/projects/{}/releases/{}", self.base_url, id, encoded_tag);
            let raw: GitlabReleaseRaw = self.get_json(&url)?;
            Ok(Self::map_release(raw, &full))
        })
    }

    pub fn latest_release(&self, owner: &str, repo: &str) -> Result<Release> {
        let full = format!("{owner}/{repo}");
        self.api_cache(&format!("gitlab_latest_{full}"), || {
            let id = Self::project_encode(&full);
            let url = format!("{}/projects/{}/releases?per_page=1", self.base_url, id);
            let raws: Vec<GitlabReleaseRaw> = self.get_json(&url)?;
            let raw = raws
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("no releases found for {full} on GitLab"))?;
            Ok(Self::map_release(raw, &full))
        })
    }

    pub fn release(&self, owner: &str, repo: &str, tag_or_version: &str) -> Result<Release> {
        match self.release_by_tag(owner, repo, tag_or_version) {
            Ok(r) => Ok(r),
            Err(_) => self.latest_release(owner, repo),
        }
    }

    pub fn releases(&self, owner: &str, repo: &str, per_page: u8) -> Result<Vec<ReleaseMeta>> {
        let full = format!("{owner}/{repo}");
        let id = Self::project_encode(&full);
        let url = format!(
            "{}/projects/{}/releases?per_page={}",
            self.base_url, id, per_page
        );
        let raws: Vec<GitlabReleaseRaw> = self.get_json(&url)?;
        Ok(raws
            .into_iter()
            .map(|r| ReleaseMeta {
                tag: r.tag_name.clone(),
                published_at: r.released_at.clone().or(r.created_at.clone()),
            })
            .collect())
    }

    // Stubs for license/root/file – GitLab has different endpoints but we return None/empty for now.
    // This keeps dual-license detection benign (falls back to config's license_spdx).
    pub fn repo_license(&self, _owner: &str, _repo: &str) -> Result<Option<RepoLicense>> {
        Ok(None)
    }
    pub fn repo_root(&self, _owner: &str, _repo: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    pub fn repo_file_text(&self, _owner: &str, _repo: &str, _path: &str) -> Result<Option<String>> {
        Ok(None)
    }

    pub fn map_release(raw: GitlabReleaseRaw, repo: &str) -> Release {
        let assets: Vec<Asset> = raw
            .assets
            .links
            .into_iter()
            .map(|l| Asset {
                name: l.name,
                size: None,
                browser_download_url: l.direct_asset_url.unwrap_or(l.url),
            })
            .collect();

        Release {
            tag_name: raw.tag_name.clone(),
            prerelease: false,
            draft: false,
            html_url: format!("https://gitlab.com/{repo}/-/releases/{}", raw.tag_name),
            assets,
            published_at: raw
                .released_at
                .or(raw.created_at)
                .and_then(|s| parse_gitlab_time(&s)),
            body: raw.description,
        }
    }

    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        crate::cache::ApiCache::new(self.api_cache_dir.clone()).get_or_fetch(key, fetch)
    }
}

fn urlencoding_encode(s: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet};
    // RFC 3986 unreserved: alphanumerics plus -_.~ are left as-is.
    const UNRESERVED: &AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'!')
        .add(b'"')
        .add(b'#')
        .add(b'$')
        .add(b'%')
        .add(b'&')
        .add(b'\'')
        .add(b'(')
        .add(b')')
        .add(b'*')
        .add(b'+')
        .add(b',')
        .add(b'/')
        .add(b':')
        .add(b';')
        .add(b'<')
        .add(b'=')
        .add(b'>')
        .add(b'?')
        .add(b'@')
        .add(b'[')
        .add(b'\\')
        .add(b']')
        .add(b'^')
        .add(b'`')
        .add(b'{')
        .add(b'|')
        .add(b'}');
    utf8_percent_encode(s, UNRESERVED).to_string()
}

pub fn parse_gitlab_time(s: &str) -> Option<i64> {
    // GitLab times are ISO8601 like 2025-01-01T00:00:00.000Z
    // Try jiff's RFC3339 parser first (handles both with and without fractional).
    if let Ok(ts) = s.parse::<jiff::Timestamp>() {
        return Some(ts.as_second());
    }
    // Fallback to explicit strptime patterns
    jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%S%.fZ", s)
        .or_else(|_| jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%SZ", s))
        .ok()
        .map(|t| t.as_second())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitlabReleaseRaw {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub upcoming_release: Option<bool>,
    #[serde(default)]
    pub assets: GitlabAssets,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GitlabAssets {
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub sources: Vec<serde_json::Value>,
    #[serde(default)]
    pub links: Vec<GitlabLink>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitlabLink {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub direct_asset_url: Option<String>,
    #[serde(default)]
    pub link_type: Option<String>,
}

/// Uniform construction for the `SourcePlugin` glue (`ClientNew`).
impl crate::source_client::ClientNew for GitlabClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for GitlabClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        GitlabClient::raw_get(self, url)
    }
}
