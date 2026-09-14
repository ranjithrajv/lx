// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use lx_lib::github::{Release, ReleaseMeta};

use super::ForgeSource;
use crate::plugins::plugin::plugin_identity;

pub struct GiteeForgeSource;

plugin_identity!(
    GiteeForgeSource,
    "gitee",
    "Gitee Releases (gitee.com / self-hosted, API v5)"
);

impl ForgeSource for GiteeForgeSource {
    fn token_env(&self) -> Option<&'static str> {
        Some("GITEE_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        Some("gitee_host")
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_gitee_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client =
            lx_lib::source_client::new_client_for::<lx_lib::gitee::GiteeClient>(token, cache_dir)?;
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
        let client =
            lx_lib::source_client::new_client_for::<lx_lib::gitee::GiteeClient>(token, cache_dir)?;
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
        let client =
            lx_lib::source_client::new_client_for::<lx_lib::gitee::GiteeClient>(token, cache_dir)?;
        client.releases(owner, repo_name, per_page)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lx_lib::gitee::GiteeClient::new(token.map(|s| s.to_string()))?;
        client.raw_get(url)
    }
}

pub fn parse_gitee_url(s: &str) -> Option<String> {
    if let Some(repo) = super::parse_host_url(s, lx_lib::constants::DEFAULT_GITEE_HOST) {
        return Some(repo);
    }
    if let Ok(host) = std::env::var("GITEE_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_GITEE_HOST {
            if let Some(repo) = super::parse_host_url(s, host) {
                return Some(repo);
            }
        }
    }
    None
}
