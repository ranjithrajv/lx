// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

use crate::gitea::GiteaReleaseRaw;
use crate::release::{Release, ReleaseMeta};

/// Forgejo API client – API compatible with Gitea.
/// Uses `FORGEJO_*` env vars, falls back to `GITEA_*` for compat.
pub struct ForgejoClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    api_cache_dir: Option<PathBuf>,
}

impl ForgejoClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_cache(token, None)
    }

    pub fn with_cache(token: Option<String>, cache_dir: Option<PathBuf>) -> Result<Self> {
        let http = crate::http::new_client()?;

        let base_url = std::env::var("FORGEJO_API_URL")
            .ok()
            .or_else(|| std::env::var("GITEA_API_URL").ok())
            .or_else(|| {
                std::env::var("FORGEJO_HOST")
                    .ok()
                    .or_else(|| std::env::var("GITEA_HOST").ok())
                    .map(|h| format!("https://{}/api/v1", h.trim_end_matches('/')))
            })
            .unwrap_or_else(|| crate::constants::DEFAULT_FORGEJO_API_URL.to_string());

        let token = token
            .or_else(|| std::env::var("FORGEJO_TOKEN").ok())
            .or_else(|| std::env::var("GITEA_TOKEN").ok());

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
                return Err(anyhow!("not found on Forgejo: {body}"));
            }
            return Err(anyhow!("Forgejo API {status} for {url}: {body}"));
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
        self.api_cache(&format!("forgejo_release_{full}_{tag}"), || {
            let url = format!(
                "{}/repos/{}/{}/releases/tags/{}",
                self.base_url,
                owner,
                repo,
                crate::http::urlencode(tag)
            );
            let raw: GiteaReleaseRaw = self.get_json(&url)?;
            Ok(crate::gitea::GiteaClient::map_release(raw, &full))
        })
    }

    pub fn latest_release(&self, owner: &str, repo: &str) -> Result<Release> {
        let full = format!("{owner}/{repo}");
        self.api_cache(&format!("forgejo_latest_{full}"), || {
            let url = format!(
                "{}/repos/{}/{}/releases?limit=1",
                self.base_url, owner, repo
            );
            let mut raws: Vec<GiteaReleaseRaw> = self.get_json(&url)?;
            let raw = raws
                .drain(..)
                .next()
                .ok_or_else(|| anyhow!("no releases found for {full} on Forgejo"))?;
            Ok(crate::gitea::GiteaClient::map_release(raw, &full))
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

    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        crate::cache::ApiCache::new(self.api_cache_dir.clone()).get_or_fetch(key, fetch)
    }
}

/// Uniform construction for the `ForgeSource` glue (`ClientNew`).
impl crate::source_client::ClientNew for ForgejoClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for ForgejoClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        ForgejoClient::raw_get(self, url)
    }
}
