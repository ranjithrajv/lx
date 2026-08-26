use anyhow::Result;
use std::path::Path;

use lpt_lib::github::{GitHubClient, Release, ReleaseMeta, RepoLicense};

use super::SourcePlugin;

pub struct GithubSourcePlugin;

impl SourcePlugin for GithubSourcePlugin {
    fn name(&self) -> &'static str {
        "github"
    }

    fn description(&self) -> &'static str {
        "GitHub Releases (api.github.com / octocrab) — github_repo: owner/repo"
    }

    fn token_env(&self) -> Option<&'static str> {
        Some("GITHUB_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        None
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        // Mirrors build.rs::parse_github_url but returns repo.
        crate::build::parse_github_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lpt_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
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
        let client = lpt_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
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
        let client = lpt_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
        client.releases(owner, repo_name, per_page)
    }

    fn repo_license(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Option<RepoLicense>> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lpt_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
        client.repo_license(owner, repo_name)
    }

    fn repo_root(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Vec<String>> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lpt_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
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
        let client = lpt_lib::source_client::new_client_for::<GitHubClient>(token, cache_dir)?;
        client.repo_file_text(owner, repo_name, path)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lpt_lib::source_client::new_client_for::<GitHubClient>(token, None)?;
        client.raw_get(url)
    }
}
