// SPDX-License-Identifier: GPL-3.0-or-later

use super::repo_forge_source;

pub struct GiteeForgeSource;

repo_forge_source!(
    GiteeForgeSource,
    lx_lib::gitee::GiteeClient,
    "gitee",
    "Gitee Releases (gitee.com / self-hosted, API v5)",
    Some("GITEE_TOKEN"),
    Some("GITEE_HOST"),
    |cfg: &crate::config::PackageConfig| cfg.gitee_host.clone(),
    |_s, cfg: &crate::config::PackageConfig| {
        lx_lib::constants::homepage_for_gitee(
            &cfg.github_repo,
            cfg.gitee_host.as_deref().unwrap_or(""),
        )
    },
    parse_gitee_url,
);

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
