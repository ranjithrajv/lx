// SPDX-License-Identifier: GPL-3.0-or-later

//! SourceForge file-release client.
//!
//! SourceForge has no public "releases" API, but every project exposes a
//! file index RSS feed at
//! `https://sourceforge.net/projects/{project}/rss?limit=N`. Each `<item>`
//! is one downloadable file (recursively, across version folders), with a
//! `/download` link and a `filesize` attribute — enough to synthesize a
//! pseudo-`Release` for the shared `match_assets` pipeline.
//!
//! Feed items look like:
//!
//! ```xml
//! <item>
//!   <title><![CDATA[/7-Zip/26.03/7z2603-linux-x64.tar.xz]]></title>
//!   <link>https://sourceforge.net/projects/sevenzip/files/7-Zip/26.03/7z2603-linux-x64.tar.xz/download</link>
//!   <media:content url="…" filesize="1572504">…</media:content>
//! </item>
//! ```
//!
//! SourceForge publishes no `.sha256` sidecars, so builds from this source
//! are fail-closed like any other unverified asset: pass `--allow-unverified`
//! or a pinned `package.lock`.

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use std::path::PathBuf;

use crate::release::{Asset, Release, ReleaseMeta};

/// SourceForge RSS client.
pub struct SourceForgeClient {
    http: reqwest::blocking::Client,
    base_url: String,
    #[allow(dead_code)]
    token: Option<String>,
    api_cache_dir: Option<PathBuf>,
}

impl SourceForgeClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_cache(token, None)
    }

    pub fn with_cache(token: Option<String>, cache_dir: Option<PathBuf>) -> Result<Self> {
        let http = crate::http::new_client()?;
        let base_url = std::env::var("SOURCEFORGE_API_URL")
            .ok()
            .unwrap_or_else(|| crate::constants::DEFAULT_SOURCEFORGE_API_URL.to_string());
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            api_cache_dir: cache_dir,
        })
    }

    fn get_text(&self, url: &str) -> Result<String> {
        let resp = crate::http::send_get_with_retry(&self.http, url, None)
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {} for {url}", resp.status()));
        }
        resp.text()
            .with_context(|| format!("failed to read body from {url}"))
    }

    pub fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        crate::http::raw_get(&self.http, url, None)
    }

    fn feed_url(&self, project: &str, limit: u16) -> String {
        format!(
            "{}/projects/{}/rss?limit={limit}",
            self.base_url,
            project.trim_matches('/')
        )
    }

    /// All files for `project`, as a pseudo-release tagged `latest`.
    pub fn latest_release(&self, project: &str) -> Result<Release> {
        let project = project.trim_matches('/');
        self.api_cache(&format!("sourceforge_files_{project}"), || {
            let url = self.feed_url(project, 100);
            let xml = self.get_text(&url)?;
            let assets = parse_rss_assets(&xml)?;
            if assets.is_empty() {
                return Err(anyhow!(
                    "no files found for SourceForge project '{project}' (feed: {url})"
                ));
            }
            Ok(Release {
                tag_name: "latest".to_string(),
                prerelease: false,
                draft: false,
                html_url: crate::constants::homepage_for_sourceforge(project),
                assets,
                published_at: None,
                body: None,
            })
        })
    }

    /// SourceForge has no release tags: return every file, tagged with the
    /// requested version, and let `match_assets` select by filename.
    pub fn release_by_tag(&self, project: &str, tag: &str) -> Result<Release> {
        let mut release = self.latest_release(project)?;
        release.tag_name = tag.to_string();
        Ok(release)
    }

    pub fn releases(&self, project: &str, _per_page: u8) -> Result<Vec<ReleaseMeta>> {
        let release = self.latest_release(project)?;
        Ok(vec![ReleaseMeta {
            tag: release.tag_name,
            published_at: None,
        }])
    }

    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        crate::cache::ApiCache::new(self.api_cache_dir.clone()).get_or_fetch(key, fetch)
    }
}

/// Parse a SourceForge project RSS feed into release assets.
///
/// The feed is simple, fixed-shape XML; a couple of anchored regexes avoid
/// pulling in an XML dependency.
pub fn parse_rss_assets(xml: &str) -> Result<Vec<Asset>> {
    let item_re = Regex::new(r"(?s)<item>(.*?)</item>").expect("valid item regex");
    let title_re = Regex::new(r"(?s)<title>(.*?)</title>").expect("valid title regex");
    let link_re = Regex::new(r"(?s)<link>(.*?)</link>").expect("valid link regex");
    let size_re = Regex::new(r#"filesize="(\d+)""#).expect("valid filesize regex");
    // The feed publishes an inline digest: `<media:hash algo="md5">…</media:hash>`.
    let hash_re = Regex::new(r#"<media:hash algo="([^"]+)">([0-9a-fA-F]+)</media:hash>"#)
        .expect("valid media:hash regex");

    let mut assets = Vec::new();
    for cap in item_re.captures_iter(xml) {
        let item = &cap[1];
        let Some(title) = title_re.captures(item).map(|c| strip_cdata(&c[1])) else {
            continue;
        };
        let Some(link) = link_re
            .captures(item)
            .map(|c| c[1].trim().replace("&amp;", "&"))
        else {
            continue;
        };
        if link.is_empty() {
            continue;
        }
        // Title is a path like `/7-Zip/26.03/7z2603-x64.msi`; the asset
        // name is its basename.
        let name = title
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        let size = size_re
            .captures(item)
            .and_then(|c| c[1].parse::<u64>().ok());
        let mut checksums = std::collections::BTreeMap::new();
        if let Some(c) = hash_re.captures(item) {
            checksums.insert(c[1].to_ascii_lowercase(), c[2].to_ascii_lowercase());
        }
        assets.push(Asset {
            checksums,
            name,
            size,
            browser_download_url: link,
        });
    }
    // Stable, feed-order-independent output.
    assets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(assets)
}

fn strip_cdata(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("<![CDATA[")
        .and_then(|x| x.strip_suffix("]]>"))
        .unwrap_or(s);
    s.trim().to_string()
}

/// Uniform construction for the `ForgeSource` glue (`ClientNew`).
impl crate::source_client::ClientNew for SourceForgeClient {
    fn with_cache(
        token: Option<String>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::with_cache(token, cache_dir)
    }
}

impl crate::checksum::RawGetter for SourceForgeClient {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
        SourceForgeClient::raw_get(self, url)
    }
}
