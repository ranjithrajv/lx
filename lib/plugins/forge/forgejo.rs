// SPDX-License-Identifier: GPL-3.0-or-later

use super::repo_forge_source;

pub struct ForgejoForgeSource;

repo_forge_source!(
    ForgejoForgeSource,
    lx_lib::forgejo::ForgejoClient,
    "forgejo",
    "Forgejo Releases (codeberg.org / self-hosted, Forgejo/Gitea API v1)",
    Some("FORGEJO_TOKEN"),
    Some("forgejo_host"),
    parse_forgejo_url,
);

pub fn parse_forgejo_url(s: &str) -> Option<String> {
    if let Some(repo) = super::parse_host_url(s, lx_lib::constants::DEFAULT_FORGEJO_HOST) {
        return Some(repo);
    }
    // Also accept FORGEJO_HOST env
    if let Ok(host) = std::env::var("FORGEJO_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_FORGEJO_HOST {
            if let Some(repo) = super::parse_host_url(s, host) {
                return Some(repo);
            }
        }
    }
    // Heuristic for forgejo-like hosts
    if s.contains("forgejo") && !s.contains("github.com") && !s.contains("gitlab.com") {
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
    // Also accept codeberg.org for forgejo (since codeberg now runs forgejo)
    if let Some(repo) = super::parse_host_url(s, "codeberg.org") {
        // Only claim codeberg for forgejo if gitea didn't already claim it?
        // For parse_any_forge_url, gitea is tried before forgejo, so codeberg will be gitea first.
        // We still handle it here for direct --source forgejo with codeberg URL.
        return Some(repo);
    }
    None
}
