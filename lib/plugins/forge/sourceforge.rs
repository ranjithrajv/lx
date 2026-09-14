// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use lx_lib::github::{Release, ReleaseMeta};

use super::ForgeSource;

pub struct SourceForgeForgeSource;

impl ForgeSource for SourceForgeForgeSource {
    fn name(&self) -> &'static str {
        "sourceforge"
    }

    fn description(&self) -> &'static str {
        "SourceForge file releases (project RSS feed as pseudo-releases)"
    }

    fn token_env(&self) -> Option<&'static str> {
        None
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_sourceforge_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let client = lx_lib::source_client::new_client_for::<lx_lib::sourceforge::SourceForgeClient>(
            token, cache_dir,
        )?;
        client.latest_release(repo)
    }

    fn release_by_tag(
        &self,
        repo: &str,
        tag: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let client = lx_lib::source_client::new_client_for::<lx_lib::sourceforge::SourceForgeClient>(
            token, cache_dir,
        )?;
        client.release_by_tag(repo, tag)
    }

    fn releases(
        &self,
        repo: &str,
        per_page: u8,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Vec<ReleaseMeta>> {
        let client = lx_lib::source_client::new_client_for::<lx_lib::sourceforge::SourceForgeClient>(
            token, cache_dir,
        )?;
        client.releases(repo, per_page)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lx_lib::sourceforge::SourceForgeClient::new(token.map(|s| s.to_string()))?;
        client.raw_get(url)
    }
}

/// Parse a SourceForge project URL/reference into a bare project name.
///
/// Accepts `https://sourceforge.net/projects/{project}[/...]`,
/// `https://sf.net/projects/{project}`, and `{project}.sourceforge.net`.
pub fn parse_sourceforge_url(s: &str) -> Option<String> {
    let trimmed = s.trim();

    // https://{project}.sourceforge.net/...
    if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    {
        let mut parts = rest.splitn(2, '/');
        let host = parts.next().unwrap_or("");
        if let Some(project) = host.strip_suffix(".sourceforge.net") {
            if !project.is_empty() {
                return Some(project.to_string());
            }
        }
    }

    // https://sourceforge.net/projects/{project}/...
    for prefix in [
        "https://sourceforge.net/projects/",
        "http://sourceforge.net/projects/",
        "https://sf.net/projects/",
        "http://sf.net/projects/",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let project = rest.trim_end_matches('/').split('/').next().unwrap_or("");
            if !project.is_empty() {
                return Some(project.to_string());
            }
        }
    }

    None
}
