// SPDX-License-Identifier: GPL-3.0-or-later

use super::repo_forge_source;

pub struct GiteaForgeSource;

repo_forge_source!(
    GiteaForgeSource,
    lx_lib::gitea::GiteaClient,
    "gitea",
    "Gitea Releases (codeberg.org / self-hosted, Gitea API v1)",
    Some("GITEA_TOKEN"),
    Some("GITEA_HOST"),
    |cfg: &crate::config::PackageConfig| cfg.gitea_host.clone(),
    |_s, cfg: &crate::config::PackageConfig| {
        lx_lib::constants::homepage_for_gitea(
            &cfg.github_repo,
            cfg.gitea_host.as_deref().unwrap_or(""),
        )
    },
    parse_gitea_url,
);

pub fn parse_gitea_url(s: &str) -> Option<String> {
    // Gitea default host is codeberg.org, but also supports custom via GITEA_HOST
    if let Some(repo) = super::parse_host_url(s, lx_lib::constants::DEFAULT_GITEA_HOST) {
        return Some(repo);
    }
    if let Ok(host) = std::env::var("GITEA_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_GITEA_HOST {
            if let Some(repo) = super::parse_host_url(s, host) {
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
