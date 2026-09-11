use anyhow::Result;
use std::path::Path;

use lx_lib::github::{Release, ReleaseMeta};

use super::SourcePlugin;

pub struct GitlabSourcePlugin;

impl SourcePlugin for GitlabSourcePlugin {
    fn name(&self) -> &'static str {
        "gitlab"
    }

    fn description(&self) -> &'static str {
        "GitLab Releases (gitlab.com / self-hosted) — via GitLab API v4"
    }

    fn token_env(&self) -> Option<&'static str> {
        Some("GITLAB_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        Some("gitlab_host")
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_gitlab_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let (owner, repo_name) = crate::discovery::split_repo(repo)?;
        let client = lx_lib::source_client::new_client_for::<lx_lib::gitlab::GitlabClient>(
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
        let client = lx_lib::source_client::new_client_for::<lx_lib::gitlab::GitlabClient>(
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
        let client = lx_lib::source_client::new_client_for::<lx_lib::gitlab::GitlabClient>(
            token, cache_dir,
        )?;
        client.releases(owner, repo_name, per_page)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lx_lib::gitlab::GitlabClient::new(token.map(|s| s.to_string()))?;
        let r = client.raw_get(url)?;
        Ok(Box::new(r))
    }
}

/// Parse a `https://gitlab.com/<owner>/<repo>` URL into "owner/repo".
/// Supports `https://gitlab.com/` and custom hosts via `GITLAB_HOST`.
/// Mirrors `parse_github_url` but for GitLab.
pub fn parse_gitlab_url(s: &str) -> Option<String> {
    // Allow https://gitlab.com/ and http://, plus custom host via env.
    // For custom host, we check GITLAB_HOST env first, but also accept any
    // host containing "gitlab" as fallback. Simplest: try github-style strip
    // for gitlab.com, and also for custom host if set.
    if let Some(repo) = try_parse_for_host(s, lx_lib::constants::DEFAULT_GITLAB_HOST) {
        return Some(repo);
    }
    if let Ok(host) = std::env::var("GITLAB_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_GITLAB_HOST {
            if let Some(repo) = try_parse_for_host(s, host) {
                return Some(repo);
            }
        }
    }
    // Also try generic "https://<host>/<owner>/<repo>" where host contains "gitlab"
    // as heuristic for self-hosted without env? Try extracting host from URL and checking.
    // Fallback: if URL contains "gitlab" and has at least 2 path parts, treat as gitlab.
    if s.contains("gitlab") {
        // Try to extract owner/repo from any gitlab-ish URL
        if let Some(rest) = s.split("://").nth(1) {
            let mut parts = rest.splitn(4, '/');
            let _host = parts.next()?;
            let owner = parts.next()?;
            let repo = parts.next()?.trim_end_matches(".git").trim_end_matches('/');
            if !owner.is_empty() && !repo.is_empty() && !repo.contains('/') {
                // Avoid treating github URLs as gitlab; already handled
                if !s.contains("github.com") {
                    return Some(format!("{owner}/{repo}"));
                }
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
