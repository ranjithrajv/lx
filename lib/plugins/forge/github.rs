// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use lx_lib::github::{GitHubClient, Release, ReleaseMeta, RepoLicense};

use super::ForgeSource;
use crate::plugins::plugin::plugin_identity;

pub struct GithubForgeSource;

plugin_identity!(
    GithubForgeSource,
    "github",
    "GitHub Releases (api.github.com, blocking reqwest) — github_repo: owner/repo"
);

impl ForgeSource for GithubForgeSource {
    fn token_env(&self) -> Option<&'static str> {
        Some("GITHUB_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        None
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_github_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
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
        let client = lx_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
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
        let client = lx_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
        client.releases(owner, repo_name, per_page)
    }

    fn repo_license(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Option<RepoLicense>> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
        client.repo_license(owner, repo_name)
    }

    fn repo_root(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Vec<String>> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
        client.repo_root(owner, repo_name)
    }

    fn repo_file_text(
        &self,
        repo: &str,
        path: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Option<String>> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
        client.repo_file_text(owner, repo_name, path)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lx_lib::source_client::new_client_for::<GitHubClient>(token, None)?;
        client.raw_get(url)
    }
}

/// Parse a bare `https://github.com/<owner>/<repo>` URL into `"owner/repo"`,
/// ignoring any further path (a `.git` suffix, `/releases`, a tag, etc.).
/// Returns `None` for anything that isn't a github.com URL, so callers can
/// fall through to treating the argument as a package.yaml path.
pub fn parse_github_url(s: &str) -> Option<String> {
    let host = lx_lib::constants::DEFAULT_GITHUB_HOST;
    let rest = s
        .strip_prefix(&format!("https://{host}/"))
        .or_else(|| s.strip_prefix(&format!("http://{host}/")))?;
    let mut parts = rest.trim_end_matches('/').splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}
