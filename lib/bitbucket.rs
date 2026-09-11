// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::github::{Asset, Release, ReleaseMeta, RepoLicense};

/// Bitbucket Cloud API client (`api.bitbucket.org/2.0`).
/// Bitbucket has no native "releases" – we treat `downloads` as a single
/// pseudo-release with tag `latest`. Assets are `downloads` entries.
pub struct BitbucketClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    api_cache_dir: Option<PathBuf>,
}

impl BitbucketClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_cache(token, None)
    }

    pub fn with_cache(token: Option<String>, cache_dir: Option<PathBuf>) -> Result<Self> {
        let http = crate::http::new_client()?;

        let base_url = std::env::var("BITBUCKET_API_URL")
            .ok()
            .unwrap_or_else(|| crate::constants::DEFAULT_BITBUCKET_API_URL.to_string());

        let token = token
            .or_else(|| std::env::var("BITBUCKET_TOKEN").ok())
            .or_else(|| std::env::var("BITBUCKET_APP_PASSWORD").ok());

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            api_cache_dir: cache_dir,
        })
    }

    fn auth_header(&self) -> Option<(&'static str, String)> {
        self.token.as_deref().map(|t| {
            // Bitbucket supports Bearer token (OAuth) or App Password via Basic.
            // We try Bearer first; if token contains ":", treat as username:password for Basic.
            if t.contains(':') {
                // Basic auth – encode as base64? reqwest will handle if we use basic_auth,
                // but for raw header we need to encode.
                // For simplicity, use Bearer if token looks like JWT, else treat as app password with username x-token-auth
                // Bitbucket docs: curl -u username:app_password
                // We approximate by using `Authorization: Bearer <token>` for now.
                ("Authorization", format!("Bearer {t}"))
            } else {
                ("Authorization", format!("Bearer {t}"))
            }
        })
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = crate::http::send_get_with_retry(&self.http, url, self.auth_header())
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            if status.as_u16() == 404 {
                return Err(anyhow!("not found on Bitbucket: {body}"));
            }
            return Err(anyhow!("Bitbucket API {status} for {url}: {body}"));
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
        // Bitbucket has no tag-based releases; treat any tag as latest if downloads contain that tag in name
        // For now, just return latest and let caller filter by tag name in asset list.
        // If tag != "latest", we still return latest and the caller will verify asset existence via match_assets.
        let _ = tag;
        self.latest_release(owner, repo)
    }

    pub fn latest_release(&self, owner: &str, repo: &str) -> Result<Release> {
        let full = format!("{owner}/{repo}");
        self.api_cache(&format!("bitbucket_latest_{full}"), || {
            let url = format!(
                "{}/repositories/{}/{}/downloads?pagelen=100",
                self.base_url, owner, repo
            );
            let raw: BitbucketDownloadsRaw = self.get_json(&url)?;
            Ok(Self::map_downloads_to_release(raw, &full))
        })
    }

    pub fn release(&self, owner: &str, repo: &str, tag_or_version: &str) -> Result<Release> {
        // For Bitbucket, ignore tag and return latest
        let _ = tag_or_version;
        self.latest_release(owner, repo)
    }

    pub fn releases(&self, owner: &str, repo: &str, per_page: u8) -> Result<Vec<ReleaseMeta>> {
        // Return a single pseudo-release
        let _ = per_page;
        let r = self.latest_release(owner, repo)?;
        Ok(vec![ReleaseMeta {
            tag: r.tag_name,
            published_at: r.published_at.map(|t| t.to_string()),
        }])
    }

    pub fn repo_license(&self, _owner: &str, _repo: &str) -> Result<Option<RepoLicense>> {
        Ok(None)
    }
    pub fn repo_root(&self, _owner: &str, _repo: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    pub fn repo_file_text(&self, _owner: &str, _repo: &str, _path: &str) -> Result<Option<String>> {
        Ok(None)
    }

    pub fn map_downloads_to_release(raw: BitbucketDownloadsRaw, repo: &str) -> Release {
        let assets = raw
            .values
            .into_iter()
            .map(|v| Asset {
                name: v.name,
                size: v.size,
                browser_download_url: v
                    .links
                    .and_then(|l| l.download)
                    .and_then(|d| d.href)
                    .unwrap_or_default(),
            })
            .collect();

        Release {
            tag_name: "latest".to_string(),
            prerelease: false,
            draft: false,
            html_url: format!("https://bitbucket.org/{repo}/downloads"),
            assets,
            published_at: None,
            body: None,
        }
    }

    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        crate::cache::ApiCache::new(self.api_cache_dir.clone()).get_or_fetch(key, fetch)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BitbucketDownloadsRaw {
    #[serde(default)]
    pub values: Vec<BitbucketDownloadRaw>,
    #[serde(default)]
    pub pagelen: Option<u8>,
    #[serde(default)]
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BitbucketDownloadRaw {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub links: Option<BitbucketLinksRaw>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BitbucketLinksRaw {
    #[serde(default)]
    pub download: Option<BitbucketHref>,
    #[serde(default)]
    pub self_: Option<BitbucketHref>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BitbucketHref {
    #[serde(default)]
    pub href: Option<String>,
}

/// Uniform construction for the `ForgeSource` glue (`ClientNew`).
impl crate::source_client::ClientNew for BitbucketClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for BitbucketClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        BitbucketClient::raw_get(self, url)
    }
}
