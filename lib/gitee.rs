// SPDX-License-Identifier: GPL-3.0-or-later

//! Gitee API v5 client (`gitee.com`, self-hosted/enterprise).
//!
//! Mirrors `GiteaClient`/`GitlabClient`: blocking `reqwest`, the shared
//! 5-minute JSON cache, and the same `Release`/`Asset` types so the
//! discovery pipeline stays provider-agnostic.
//!
//! Gitee has no release "asset upload" concept separate from the release
//! object: attached files (when present) arrive under `assets[]` with a
//! `browser_download_url`, exactly like GitHub's schema. The API also emits
//! auto-generated source archives, which `match_assets` simply skips for
//! architecture matching.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::release::{Asset, Release, ReleaseMeta};

/// Gitee API v5 client.
pub struct GiteeClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    api_cache_dir: Option<PathBuf>,
}

impl GiteeClient {
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

        // Base URL precedence: explicit host_override > GITEE_API_URL >
        // GITEE_HOST > default (gitee.com/api/v5).
        let base_url = if let Some(h) = host_override {
            h.trim_end_matches('/').to_string()
        } else if let Ok(url) = std::env::var("GITEE_API_URL") {
            url.trim_end_matches('/').to_string()
        } else if let Ok(host) = std::env::var("GITEE_HOST") {
            let host = host
                .trim()
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            format!("https://{host}/api/v5")
        } else {
            crate::constants::DEFAULT_GITEE_API_URL.to_string()
        };

        let token = token.or_else(|| std::env::var("GITEE_TOKEN").ok());

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
        let headers: Vec<(&'static str, String)> = self.auth_header().into_iter().collect();
        crate::http::get_json(&self.http, url, &headers, "Gitee")
    }

    pub fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        crate::http::raw_get(&self.http, url, self.auth_header())
    }

    pub fn latest_release(&self, owner: &str, repo: &str) -> Result<Release> {
        let full = format!("{owner}/{repo}");
        self.api_cache(&format!("gitee_latest_{full}"), || {
            let url = format!(
                "{}/repos/{}/{}/releases?per_page=1",
                self.base_url, owner, repo
            );
            let mut raws: Vec<GiteeReleaseRaw> = self.get_json(&url)?;
            let raw = raws
                .drain(..)
                .next()
                .ok_or_else(|| anyhow!("no releases found for {full} on Gitee"))?;
            Ok(Self::map_release(raw, owner, repo))
        })
    }

    pub fn release_by_tag(&self, owner: &str, repo: &str, tag: &str) -> Result<Release> {
        let full = format!("{owner}/{repo}");
        self.api_cache(&format!("gitee_release_{full}_{tag}"), || {
            let url = format!(
                "{}/repos/{}/{}/releases/tags/{}",
                self.base_url,
                owner,
                repo,
                crate::http::urlencode(tag)
            );
            match self.get_json::<GiteeReleaseRaw>(&url) {
                Ok(raw) => Ok(Self::map_release(raw, owner, repo)),
                // Some instances lack the tags endpoint; fall back to
                // scanning the release list for an exact tag match.
                Err(_) => {
                    let list_url = format!(
                        "{}/repos/{}/{}/releases?per_page=100",
                        self.base_url, owner, repo
                    );
                    let raws: Vec<GiteeReleaseRaw> = self.get_json(&list_url)?;
                    raws.into_iter()
                        .find(|r| r.tag_name == tag)
                        .map(|r| Self::map_release(r, owner, repo))
                        .ok_or_else(|| anyhow!("release '{tag}' not found for {full} on Gitee"))
                }
            }
        })
    }

    pub fn releases(&self, owner: &str, repo: &str, per_page: u8) -> Result<Vec<ReleaseMeta>> {
        let url = format!(
            "{}/repos/{}/{}/releases?per_page={}",
            self.base_url, owner, repo, per_page
        );
        let raws: Vec<GiteeReleaseRaw> = self.get_json(&url)?;
        Ok(raws
            .into_iter()
            .map(|r| ReleaseMeta {
                tag: r.tag_name.clone(),
                published_at: r.created_at.clone(),
            })
            .collect())
    }

    pub fn map_release(raw: GiteeReleaseRaw, owner: &str, repo: &str) -> Release {
        // Prefer `assets` (GitHub-shaped attach files); fall back to
        // `attach_files` for instances that only populate that field.
        let mut raw_assets = raw.assets;
        if raw_assets.is_empty() {
            raw_assets = raw.attach_files;
        }
        let assets = raw_assets
            .into_iter()
            .filter(|a| !a.browser_download_url.trim().is_empty())
            .map(|a| Asset {
                name: a.name,
                size: a.size,
                browser_download_url: a.browser_download_url,
                checksums: Default::default(),
            })
            .collect();

        Release {
            tag_name: raw.tag_name.clone(),
            prerelease: raw.prerelease,
            draft: false,
            html_url: format!(
                "https://gitee.com/{owner}/{repo}/releases/tag/{}",
                raw.tag_name
            ),
            assets,
            published_at: raw
                .created_at
                .as_deref()
                .and_then(crate::release::parse_timestamp),
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GiteeReleaseRaw {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<GiteeAssetRaw>,
    #[serde(default)]
    pub attach_files: Vec<GiteeAssetRaw>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GiteeAssetRaw {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub size: Option<u64>,
}

/// Uniform construction for the `ForgeSource` glue (`ClientNew`).
impl crate::source_client::ClientNew for GiteeClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for GiteeClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        GiteeClient::raw_get(self, url)
    }
}
