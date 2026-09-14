// SPDX-License-Identifier: GPL-3.0-or-later

use super::repo_forge_source;

pub struct BitbucketForgeSource;

repo_forge_source!(
    BitbucketForgeSource,
    lx_lib::bitbucket::BitbucketClient,
    "bitbucket",
    "Bitbucket Cloud downloads (api.bitbucket.org, downloads as pseudo-releases)",
    Some("BITBUCKET_TOKEN"),
    Some("BITBUCKET_HOST"),
    |cfg: &crate::config::PackageConfig| cfg.bitbucket_host.clone(),
    parse_bitbucket_url,
);

pub fn parse_bitbucket_url(s: &str) -> Option<String> {
    // Bitbucket Cloud: https://bitbucket.org/{workspace}/{repo_slug}
    // Also support https://api.bitbucket.org/2.0/repositories/{workspace}/{repo_slug}
    if let Some(repo) = super::parse_host_url(s, lx_lib::constants::DEFAULT_BITBUCKET_HOST) {
        return Some(repo);
    }
    if let Some(repo) = super::parse_host_url(s, "api.bitbucket.org") {
        // Handle API URL: https://api.bitbucket.org/2.0/repositories/{workspace}/{repo}
        // Extract workspace/repo from path after /2.0/repositories/
        if let Some(rest) = s.split("/2.0/repositories/").nth(1) {
            let mut parts = rest.trim_end_matches('/').splitn(3, '/');
            let owner = parts.next()?;
            let repo = parts.next()?.trim_end_matches(".git");
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return None;
            }
            return Some(format!("{owner}/{repo}"));
        }
        return Some(repo);
    }
    // Custom host via BITBUCKET_HOST? Check env
    if let Ok(host) = std::env::var("BITBUCKET_HOST") {
        let host = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        if !host.is_empty() && host != lx_lib::constants::DEFAULT_BITBUCKET_HOST {
            if let Some(repo) = super::parse_host_url(s, host) {
                return Some(repo);
            }
        }
    }
    None
}
