// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use lx_lib::github::{Release, ReleaseMeta};

use super::ForgeSource;

pub struct BitbucketForgeSource;

impl ForgeSource for BitbucketForgeSource {
    fn name(&self) -> &'static str {
        "bitbucket"
    }

    fn description(&self) -> &'static str {
        "Bitbucket Cloud downloads (api.bitbucket.org, downloads as pseudo-releases)"
    }

    fn token_env(&self) -> Option<&'static str> {
        Some("BITBUCKET_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        Some("bitbucket_host")
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_bitbucket_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<lx_lib::bitbucket::BitbucketClient>(
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
        let client = lx_lib::source_client::new_client_for::<lx_lib::bitbucket::BitbucketClient>(
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
        let client = lx_lib::source_client::new_client_for::<lx_lib::bitbucket::BitbucketClient>(
            token, cache_dir,
        )?;
        client.releases(owner, repo_name, per_page)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lx_lib::bitbucket::BitbucketClient::new(token.map(|s| s.to_string()))?;
        client.raw_get(url)
    }
}

pub fn parse_bitbucket_url(s: &str) -> Option<String> {
    // Bitbucket Cloud: https://bitbucket.org/{workspace}/{repo_slug}
    // Also support https://api.bitbucket.org/2.0/repositories/{workspace}/{repo_slug}
    if let Some(repo) = try_parse_for_host(s, lx_lib::constants::DEFAULT_BITBUCKET_HOST) {
        return Some(repo);
    }
    if let Some(repo) = try_parse_for_host(s, "api.bitbucket.org") {
        // Handle API URL: https://api.bitbucket.org/2.0/repositories/{workspace}/{repo}
        // Extract workspace/repo from path after /2.0/repositories/
        if let Some(rest) = s.split("/2.0/repositories/").nth(1) {
            let mut parts = rest.trim_end_matches('/').splitn(3, '/');
            let owner = parts.next()?;
            let repo = parts.next()?.trim_end_matches(".git");
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            return Some(format!("{owner}/{repo}"));
        }
        return Some(repo);
    }
    // Custom host via BITBUCKET_HOST? Check env
    if let Ok(host) = std::env::var("BITBUCKET_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_BITBUCKET_HOST {
            if let Some(repo) = try_parse_for_host(s, host) {
                return Some(repo);
            }
        }
    }
    None
}

fn try_parse_for_host(s: &str, host: &str) -> Option<String> {
    let prefixes = [format!("https://{host}/"), format!("http://{host}/")];
    for prefix in &prefixes {
        if let Some(rest) = s.strip_prefix(prefix.as_str()) {
            // For bitbucket.org, path is {workspace}/{repo} possibly with /downloads or /src etc
            let mut parts = rest.trim_end_matches('/').splitn(3, '/');
            let owner = parts.next()?;
            let repo = parts.next()?.trim_end_matches(".git");
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            // Ensure not capturing extra path like "workspace/repo/downloads" – we already trimmed to 3 parts, repo is second
            return Some(format!("{owner}/{repo}"));
        }
    }
    None
}
