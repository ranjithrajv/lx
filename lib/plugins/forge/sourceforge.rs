// SPDX-License-Identifier: GPL-3.0-or-later

use super::project_forge_source;

pub struct SourceForgeForgeSource;

project_forge_source!(
    SourceForgeForgeSource,
    lx_lib::sourceforge::SourceForgeClient,
    "sourceforge",
    "SourceForge file releases (project RSS feed as pseudo-releases)",
    None,
    None,
    |_| None,
    |_s, cfg: &crate::config::PackageConfig| {
        lx_lib::constants::homepage_for_sourceforge(&cfg.github_repo)
    },
    parse_sourceforge_url,
);

/// Parse a SourceForge project URL/reference into a bare project name.
///
/// Accepts `https://sourceforge.net/projects/{project}[/...]`,
/// `https://sf.net/projects/{project}`, and `{project}.sourceforge.net`.
pub fn parse_sourceforge_url(s: &str) -> Option<String> {
    let trimmed = s.trim();

    // https://{project}.sourceforge.net/...
    if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    {
        let mut parts = rest.splitn(2, '/');
        let host = parts.next().unwrap_or("");
        if let Some(project) = host.strip_suffix(".sourceforge.net") {
            if !project.is_empty() {
                return Some(project.to_string());
            }
        }
    }

    // https://sourceforge.net/projects/{project}/...
    for prefix in [
        "https://sourceforge.net/projects/",
        "http://sourceforge.net/projects/",
        "https://sf.net/projects/",
        "http://sf.net/projects/",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let project = rest.trim_end_matches('/').split('/').next().unwrap_or("");
            if !project.is_empty() {
                return Some(project.to_string());
            }
        }
    }

    None
}
