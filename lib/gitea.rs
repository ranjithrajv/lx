// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::release::{Asset, Release, ReleaseMeta};

/// Gitea API client (also used for Forgejo – API compatible).
/// Mirrors `GitHubClient`/`GitlabClient` surface, uses blocking `reqwest`.
pub struct GiteaClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    api_cache_dir: Option<PathBuf>,
}

impl GiteaClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_cache(token, None)
    }

    pub fn with_cache(token: Option<String>, cache_dir: Option<PathBuf>) -> Result<Self> {
        Self::with_host(token, cache_dir, None)
    }

    pub fn with_host(
        token: Option<String>,
        cache_dir: Option<PathBuf>,
        host_override: Option<String>,
    ) -> Result<Self> {
        let http = crate::http::new_client()?;

        // Base URL precedence: explicit host_override > GITEA_API_URL > GITEA_HOST > default
        // For Forgejo, caller passes DEFAULT_FORGEJO_API_URL via host_override or env.
        let base_url = if let Some(h) = host_override {
            h.trim_end_matches('/').to_string()
        } else if let Ok(url) = std::env::var("GITEA_API_URL") {
            url.trim_end_matches('/').to_string()
        } else if let Ok(host) = std::env::var("GITEA_HOST") {
            format!("https://{}/api/v1", host.trim_end_matches('/'))
        } else {
            crate::constants::DEFAULT_GITEA_API_URL.to_string()
        };

        // Token precedence: explicit > GITEA_TOKEN env
        let token = token.or_else(|| std::env::var("GITEA_TOKEN").ok());

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
            .map(|t| ("Authorization", format!("token {t}")))
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = crate::http::send_get_with_retry(&self.http, url, self.auth_header())
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            if status.as_u16() == 404 {
                return Err(anyhow!("not found on Gitea: {body}"));
            }
            return Err(anyhow!("Gitea API {status} for {url}: {body}"));
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
        self.api_cache(&format!("gitea_release_{full}_{tag}"), || {
            let url = format!(
                "{}/repos/{}/{}/releases/tags/{}",
                self.base_url,
                owner,
                repo,
                urlencoding_encode(tag)
            );
            let raw: GiteaReleaseRaw = self.get_json(&url)?;
            Ok(Self::map_release(raw, &full))
        })
    }

    pub fn latest_release(&self, owner: &str, repo: &str) -> Result<Release> {
        let full = format!("{owner}/{repo}");
        self.api_cache(&format!("gitea_latest_{full}"), || {
            let url = format!(
                "{}/repos/{}/{}/releases?limit=1",
                self.base_url, owner, repo
            );
            let mut raws: Vec<GiteaReleaseRaw> = self.get_json(&url)?;
            let raw = raws
                .drain(..)
                .next()
                .ok_or_else(|| anyhow!("no releases found for {full} on Gitea"))?;
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
        let url = format!(
            "{}/repos/{}/{}/releases?limit={}",
            self.base_url, owner, repo, per_page
        );
        let raws: Vec<GiteaReleaseRaw> = self.get_json(&url)?;
        Ok(raws
            .into_iter()
            .map(|r| ReleaseMeta {
                tag: r.tag_name.clone(),
                published_at: r.created_at.clone().or(r.published_at.clone()),
            })
            .collect())
    }

    pub fn map_release(raw: GiteaReleaseRaw, repo: &str) -> Release {
        let assets = raw
            .assets
            .into_iter()
            .map(|a| Asset {
                checksums: crate::github::digest_checksums(a.digest.as_deref()),
                name: a.name,
                size: a.size,
                browser_download_url: a.browser_download_url,
            })
            .collect();

        Release {
            tag_name: raw.tag_name.clone(),
            prerelease: raw.prerelease,
            draft: raw.draft,
            html_url: raw.html_url.unwrap_or_else(|| {
                // Fallback: construct from base_url host + repo
                format!("{}/{}", repo, raw.tag_name)
            }),
            assets,
            published_at: raw
                .created_at
                .as_deref()
                .or(raw.published_at.as_deref())
                .and_then(parse_gitea_time),
            body: raw.body,
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

fn parse_gitea_time(s: &str) -> Option<i64> {
    if let Ok(ts) = s.parse::<jiff::Timestamp>() {
        return Some(ts.as_second());
    }
    jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%SZ", s)
        .ok()
        .map(|t| t.as_second())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GiteaReleaseRaw {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<GiteaAssetRaw>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GiteaAssetRaw {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// Gitea/Forgejo inline asset digest (`"sha256:<hex>"`), when present.
    #[serde(default)]
    pub digest: Option<String>,
}

/// Uniform construction for the `ForgeSource` glue (`ClientNew`).
impl crate::source_client::ClientNew for GiteaClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for GiteaClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        GiteaClient::raw_get(self, url)
    }
}
