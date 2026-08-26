use anyhow::Result;
use std::path::Path;

use lpt_lib::github::{Release, ReleaseMeta};

use super::SourcePlugin;

pub struct GiteaSourcePlugin;

impl SourcePlugin for GiteaSourcePlugin {
    fn name(&self) -> &'static str {
        "gitea"
    }

    fn description(&self) -> &'static str {
        "Gitea Releases (codeberg.org / self-hosted, Gitea API v1)"
    }

    fn token_env(&self) -> Option<&'static str> {
        Some("GITEA_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        Some("gitea_host")
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_gitea_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lpt_lib::source_client::new_client_for::<lpt_lib::gitea::GiteaClient>(
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
        let client = lpt_lib::source_client::new_client_for::<lpt_lib::gitea::GiteaClient>(
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
        let client = lpt_lib::source_client::new_client_for::<lpt_lib::gitea::GiteaClient>(
            token, cache_dir,
        )?;
        client.releases(owner, repo_name, per_page)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lpt_lib::gitea::GiteaClient::new(token.map(|s| s.to_string()))?;
        client.raw_get(url)
    }
}

pub fn parse_gitea_url(s: &str) -> Option<String> {
    // Gitea default host is codeberg.org, but also supports custom via GITEA_HOST
    if let Some(repo) = try_parse_for_host(s, lpt_lib::constants::DEFAULT_GITEA_HOST) {
        return Some(repo);
    }
    if let Ok(host) = std::env::var("GITEA_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lpt_lib::constants::DEFAULT_GITEA_HOST {
            if let Some(repo) = try_parse_for_host(s, host) {
                return Some(repo);
            }
        }
    }
    // Heuristic for any gitea-like host containing "gitea"
    if s.contains("gitea") && !s.contains("github.com") && !s.contains("gitlab.com") {
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
    None
}

fn try_parse_for_host(s: &str, host: &str) -> Option<String> {
    let prefixes = [format!("https://{host}/"), format!("http://{host}/")];
    for prefix in &prefixes {
        if let Some(rest) = s.strip_prefix(prefix.as_str()) {
            let mut parts = rest.trim_end_matches('/').splitn(3, '/');
            let owner = parts.next()?;
            let repo = parts.next()?.trim_end_matches(".git");
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            return Some(format!("{owner}/{repo}"));
        }
    }
    None
}
