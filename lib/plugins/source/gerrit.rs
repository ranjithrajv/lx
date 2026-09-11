use anyhow::Result;
use std::path::Path;

use lx_lib::github::{Release, ReleaseMeta};

use super::SourcePlugin;

pub struct GerritSourcePlugin;

impl SourcePlugin for GerritSourcePlugin {
    fn name(&self) -> &'static str {
        "gerrit"
    }

    fn description(&self) -> &'static str {
        "Gerrit Code Review (review.gerrithub.io / self-hosted, Gerrit API)"
    }

    fn token_env(&self) -> Option<&'static str> {
        Some("GERRIT_TOKEN")
    }

    fn host_config_key(&self) -> Option<&'static str> {
        Some("gerrit_host")
    }

    fn parse_url(&self, url: &str) -> Option<String> {
        parse_gerrit_url(url)
    }

    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let client = lx_lib::gerrit::GerritClient::with_cache(
            token.map(|s| s.to_string()),
            cache_dir.map(|p| p.to_path_buf()),
        )?;
        // Gerrit project is repo (may contain slashes like platform/system/core)
        // For Gerrit, we pass the full repo string as project
        client.latest_release(repo)
    }

    fn release_by_tag(
        &self,
        repo: &str,
        tag: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release> {
        let client = lx_lib::gerrit::GerritClient::with_cache(
            token.map(|s| s.to_string()),
            cache_dir.map(|p| p.to_path_buf()),
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
        let client = lx_lib::gerrit::GerritClient::with_cache(
            token.map(|s| s.to_string()),
            cache_dir.map(|p| p.to_path_buf()),
        )?;
        client.releases(repo, per_page)
    }

    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>> {
        let client = lx_lib::gerrit::GerritClient::new(token.map(|s| s.to_string()))?;
        client.raw_get(url)
    }
}

pub fn parse_gerrit_url(s: &str) -> Option<String> {
    // Gerrit URLs: https://{host}/{project} where project may contain slashes
    // Example: https://review.gerrithub.io/a/android/platform/build
    // For simplicity, treat host containing "gerrit" as Gerrit
    // Also support https://{host}/p/{project} (Gerrit with /p/ prefix) and https://{host}/admin/repos/{project}
    // We also handle https://{host}/projects/{project} style

    // Check for gerrit host via env or default
    let default_host = lx_lib::constants::DEFAULT_GERRIT_HOST;
    if let Some(repo) = try_parse_for_host(s, default_host) {
        return Some(repo);
    }
    if let Ok(host) = std::env::var("GERRIT_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != default_host {
            if let Some(repo) = try_parse_for_host(s, host) {
                return Some(repo);
            }
        }
    }
    // Heuristic: any URL with host containing "gerrit" is considered Gerrit
    if s.contains("gerrit")
        && !s.contains("github.com")
        && !s.contains("gitlab.com")
        && !s.contains("bitbucket.org")
        && !s.contains("codeberg.org")
    {
        if let Some(rest) = s.split("://").nth(1) {
            let mut parts = rest.splitn(2, '/');
            let _host = parts.next()?;
            let mut project = parts.next()?.trim_end_matches(".git").trim_end_matches('/');
            // Strip Gerrit's auth/view prefix segments ("a/", "p/") which
            // are not part of the project name.
            for marker in ["a/", "p/"] {
                let stripped = project.strip_prefix(marker).unwrap_or(project);
                if stripped != project {
                    project = stripped;
                    break;
                }
            }
            if !project.is_empty() && project.contains('/') {
                // Gerrit project may be like "a/b/c" – return as is
                return Some(project.to_string());
            }
        }
    }
    // Also handle Gerrit with /a/ prefix: https://host/a/{project}
    if s.contains("/a/") {
        if let Some(rest) = s.split("/a/").nth(1) {
            let project = rest
                .split('?')
                .next()?
                .split('#')
                .next()?
                .trim_end_matches('/')
                .trim_end_matches(".git");
            if !project.is_empty() && project.contains('/') {
                return Some(project.to_string());
            }
        }
    }
    None
}

fn try_parse_for_host(s: &str, host: &str) -> Option<String> {
    // Longer, more specific prefixes first: the bare-host prefix would
    // otherwise match an /a/ or /p/ URL first and leave the marker segment
    // ("a/...") inside the parsed project name.
    let prefixes = [
        format!("https://{host}/a/"),
        format!("http://{host}/a/"),
        format!("https://{host}/p/"),
        format!("http://{host}/p/"),
        format!("https://{host}/"),
        format!("http://{host}/"),
    ];
    for prefix in &prefixes {
        if let Some(rest) = s.strip_prefix(prefix.as_str()) {
            // For Gerrit, project may be like "platform/build" or "a/b/c"
            // Take up to 3 segments? For now take full path until next `?` or `#` or `/releases` etc
            // Strip query and fragment
            let rest = rest.split('?').next()?.split('#').next()?;
            // For Gerrit, we want the project name which is the first 2-3 segments before any extra like "/+/..."
            // Simplify: take first two slashes as owner/repo, but Gerrit projects can be deeper
            // For now, take the whole rest until first extra like "/+/"
            let project = rest
                .split("/+/")
                .next()?
                .split("/releases")
                .next()?
                .split("/tags")
                .next()?
                .trim_end_matches('/')
                .trim_end_matches(".git");
            if project.is_empty() || !project.contains('/') {
                // For Gerrit, single segment project is also valid? But we require at least one slash
                // To keep consistent with owner/repo, require one slash
                return None;
            }
            return Some(project.to_string());
        }
    }
    None
}
