// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::github::GitHubClient;

use super::repo_forge_source;

pub struct GithubForgeSource;

repo_forge_source!(
    GithubForgeSource,
    GitHubClient,
    "github",
    "GitHub Releases (api.github.com, blocking reqwest) — github_repo: owner/repo",
    Some("GITHUB_TOKEN"),
    None,
    parse_github_url,
);

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
