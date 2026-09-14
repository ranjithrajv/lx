// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::release::{Asset, Release, ReleaseMeta};

/// Gerrit Code Review API client.
/// Gerrit does not have GitHub-style releases, so we map tags → pseudo-releases.
/// Assets are synthesized as `https://{host}/{project}/-/archive/{tag}.tar.gz`
/// (common for Gerrit via plugins, e.g. `download-commands`). This is sufficient
/// for `lx`'s `match_assets` which only needs a filename containing the arch.
pub struct GerritClient {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    host: String,
    api_cache_dir: Option<PathBuf>,
}

impl GerritClient {
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

        let host = host_override
            .or_else(|| std::env::var("GERRIT_HOST").ok())
            .or_else(|| {
                std::env::var("GERRIT_API_URL").ok().map(|u| {
                    // If GERRIT_API_URL is like https://host/a, extract host
                    u.trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .split('/')
                        .next()
                        .unwrap_or(&u)
                        .to_string()
                })
            })
            .unwrap_or_else(|| crate::constants::DEFAULT_GERRIT_HOST.to_string());
        let host = host
            .trim_end_matches('/')
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();

        let base_url = std::env::var("GERRIT_API_URL")
            .ok()
            .unwrap_or_else(|| format!("https://{host}/a"));

        // Token precedence: explicit > GERRIT_TOKEN > GERRIT_PASSWORD
        let token = token
            .or_else(|| std::env::var("GERRIT_TOKEN").ok())
            .or_else(|| std::env::var("GERRIT_PASSWORD").ok())
            .or_else(|| std::env::var("GERRIT_HTTP_PASSWORD").ok());

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            host,
            api_cache_dir: cache_dir,
        })
    }

    fn auth_header(&self) -> Option<(&'static str, String)> {
        self.token.as_deref().map(|t| {
            // Gerrit a/ endpoint uses Basic auth with `Authorization: Basic base64(user:pass)`
            // For token, use `Authorization: Bearer` or `Basic` with `git:<token>`
            // Simplest: use `Authorization: Bearer <token>` which Gerrit also accepts for HTTP passwords
            if t.contains(':') {
                // Already user:pass
                let encoded = base64_encode(t);
                ("Authorization", format!("Basic {encoded}"))
            } else {
                ("Authorization", format!("Bearer {t}"))
            }
        })
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        // Gerrit prefixes JSON with `)]}'\n` for XSSI protection – strip it
        let resp = crate::http::send_get_with_retry(&self.http, url, self.auth_header())
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            if status.as_u16() == 404 {
                return Err(anyhow!("not found on Gerrit: {body}"));
            }
            return Err(anyhow!("Gerrit API {status} for {url}: {body}"));
        }
        let text = resp.text().context("failed to read Gerrit response")?;
        let json_str = text.strip_prefix(")]}'").unwrap_or(&text).trim();
        serde_json::from_str::<T>(json_str)
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

    pub fn release_by_tag(&self, project: &str, tag: &str) -> Result<Release> {
        let full = project.to_string();
        self.api_cache(&format!("gerrit_release_{full}_{tag}"), || {
            // Gerrit: GET /a/projects/{project}/tags/{tag}
            // Tag endpoint returns a single tag object; we synthesize a Release
            let encoded_project = Self::project_encode(project);
            let encoded_tag = crate::http::urlencode(tag);
            let url = format!(
                "{}/projects/{}/tags/{}",
                self.base_url, encoded_project, encoded_tag
            );
            let tag_info: GerritTagInfo = self.get_json(&url)?;
            Ok(Self::map_tag_to_release(tag_info, project, &self.host))
        })
    }

    pub fn latest_release(&self, project: &str) -> Result<Release> {
        let full = project.to_string();
        self.api_cache(&format!("gerrit_latest_{full}"), || {
            // Gerrit: GET /a/projects/{project}/tags/ – list tags, most recent first
            // We sort by revision or tag name; for now take first
            let encoded_project = Self::project_encode(project);
            let url = format!("{}/projects/{}/tags/", self.base_url, encoded_project);
            let tags: Vec<GerritTagInfo> = self.get_json(&url)?;
            let tag_info = tags.into_iter().next().ok_or_else(|| {
                anyhow!(
                    "no tags found for {project} on Gerrit (host: {})",
                    self.host
                )
            })?;
            Ok(Self::map_tag_to_release(tag_info, project, &self.host))
        })
    }

    pub fn release(&self, project: &str, tag_or_version: &str) -> Result<Release> {
        match self.release_by_tag(project, tag_or_version) {
            Ok(r) => Ok(r),
            Err(_) => self.latest_release(project),
        }
    }

    pub fn releases(&self, project: &str, per_page: u8) -> Result<Vec<ReleaseMeta>> {
        let encoded_project = Self::project_encode(project);
        let url = format!(
            "{}/projects/{}/tags/?n={}",
            self.base_url, encoded_project, per_page
        );
        let tags: Vec<GerritTagInfo> = self.get_json(&url)?;
        Ok(tags
            .into_iter()
            .map(|t| ReleaseMeta {
                tag: t
                    .ref_tag
                    .strip_prefix("refs/tags/")
                    .unwrap_or(&t.ref_tag)
                    .to_string(),
                published_at: t.created,
            })
            .collect())
    }

    pub fn project_encode(project: &str) -> String {
        crate::http::encode_path_segment(project)
    }

    pub fn map_tag_to_release(tag: GerritTagInfo, project: &str, host: &str) -> Release {
        let tag_name = tag
            .ref_tag
            .strip_prefix("refs/tags/")
            .unwrap_or(&tag.ref_tag)
            .to_string();
        let tag_name = if tag_name.is_empty() {
            tag.tag.unwrap_or(tag_name)
        } else {
            tag_name
        };
        // Gerrit doesn't have release assets, synthesize one archive per tag
        // Common Gerrit archive URL: https://host/plugins/gitiles/{project}/+archive/{tag}.tar.gz
        // Fallback to https://host/{project}/-/archive/{tag}.tar.gz
        let asset_name = format!("{}-{}.tar.gz", project.replace('/', "-"), tag_name);
        let download_url = format!("https://{host}/{project}/-/archive/{tag_name}.tar.gz");
        // Alternative gitiles URL as sidecar? Keep one.

        Release {
            tag_name: tag_name.clone(),
            prerelease: false,
            draft: false,
            html_url: format!("https://{host}/{project}"),
            assets: vec![Asset {
                name: asset_name,
                size: None,
                browser_download_url: download_url,
                checksums: Default::default(),
            }],
            published_at: tag
                .created
                .as_deref()
                .and_then(crate::release::parse_timestamp)
                .or_else(|| {
                    tag.tagger_date
                        .as_deref()
                        .and_then(crate::release::parse_timestamp)
                }),
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

fn base64_encode(s: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(s)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GerritTagInfo {
    #[serde(rename = "ref")]
    pub ref_tag: String,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub object: Option<serde_json::Value>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub tagger_date: Option<String>,
}

/// Uniform construction for the `ForgeSource` glue (`ClientNew`).
impl crate::source_client::ClientNew for GerritClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for GerritClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        GerritClient::raw_get(self, url)
    }
}
