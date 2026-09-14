// SPDX-License-Identifier: GPL-3.0-or-later

use super::repo_forge_source;

pub struct GitlabForgeSource;

repo_forge_source!(
    GitlabForgeSource,
    lx_lib::gitlab::GitlabClient,
    "gitlab",
    "GitLab Releases (gitlab.com / self-hosted) — via GitLab API v4",
    Some("GITLAB_TOKEN"),
    Some("gitlab_host"),
    parse_gitlab_url,
);

/// Parse a `https://gitlab.com/<owner>/<repo>` URL into "owner/repo".
/// Supports `https://gitlab.com/` and custom hosts via `GITLAB_HOST`.
/// Mirrors `parse_github_url` but for GitLab.
pub fn parse_gitlab_url(s: &str) -> Option<String> {
    // Allow https://gitlab.com/ and http://, plus custom host via env.
    // For custom host, we check GITLAB_HOST env first, but also accept any
    // host containing "gitlab" as fallback. Simplest: try github-style strip
    // for gitlab.com, and also for custom host if set.
    if let Some(repo) = super::parse_host_url(s, lx_lib::constants::DEFAULT_GITLAB_HOST) {
        return Some(repo);
    }
    if let Ok(host) = std::env::var("GITLAB_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_GITLAB_HOST {
            if let Some(repo) = super::parse_host_url(s, host) {
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
