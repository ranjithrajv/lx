// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use lx_lib::github::{Release, ReleaseMeta};

use super::ForgeSource;

pub struct ForgejoForgeSource;

impl ForgeSource for ForgejoForgeSource {
    fn name(&self) -> &'static str {
        "forgejo"
    }

    fn description(&self) -> &'static str {
        "Forgejo Releases (codeberg.org / self-hosted, Forgejo/Gitea API v1)"
    }

    fn token_env(&self) -> Option<&'static str> {
        Some("FORGEJO_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        Some("forgejo_host")
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_forgejo_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<lx_lib::forgejo::ForgejoClient>(
            token, cache_dir,
        )?;
        client.latest_release(owner, repo_name)
    }

    fn release_by_tag(
        &self,
        repo: &str,
        tag: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<lx_lib::forgejo::ForgejoClient>(
            token, cache_dir,
        )?;
        client.release_by_tag(owner, repo_name, tag)
    }

    fn releases(
        &self,
        repo: &str,
        per_page: u8,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Vec<ReleaseMeta>> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<lx_lib::forgejo::ForgejoClient>(
            token, cache_dir,
        )?;
        client.releases(owner, repo_name, per_page)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lx_lib::forgejo::ForgejoClient::new(token.map(|s| s.to_string()))?;
        client.raw_get(url)
    }
}

pub fn parse_forgejo_url(s: &str) -> Option<String> {
    if let Some(repo) = super::parse_host_url(s, lx_lib::constants::DEFAULT_FORGEJO_HOST) {
        return Some(repo);
    }
    // Also accept FORGEJO_HOST env
    if let Ok(host) = std::env::var("FORGEJO_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_FORGEJO_HOST {
            if let Some(repo) = super::parse_host_url(s, host) {
                return Some(repo);
            }
        }
    }
    // Heuristic for forgejo-like hosts
    if s.contains("forgejo") && !s.contains("github.com") && !s.contains("gitlab.com") {
        if let Some(rest) = s.split("://").nth(1) {
            let mut parts = rest.splitn(4, '/');
            let _host = parts.next()?;
            let owner = parts.next()?;
            let repo = parts.next()?.trim_end_matches(".git").trim_end_matches('/');
            if !owner.is_empty() && !repo.is_empty() && !repo.contains('/') {
                return Some(format!("{owner}/{repo}"));
            }
        }
    }
    // Also accept codeberg.org for forgejo (since codeberg now runs forgejo)
    if let Some(repo) = super::parse_host_url(s, "codeberg.org") {
        // Only claim codeberg for forgejo if gitea didn't already claim it?
        // For parse_any_forge_url, gitea is tried before forgejo, so codeberg will be gitea first.
        // We still handle it here for direct --source forgejo with codeberg URL.
        return Some(repo);
    }
    None
}
