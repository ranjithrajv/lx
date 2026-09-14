// SPDX-License-Identifier: GPL-3.0-or-later

use super::project_forge_source;

pub struct GerritForgeSource;

project_forge_source!(
    GerritForgeSource,
    lx_lib::gerrit::GerritClient,
    "gerrit",
    "Gerrit Code Review (review.gerrithub.io / self-hosted, Gerrit API)",
    Some("GERRIT_TOKEN"),
    Some("GERRIT_HOST"),
    |cfg: &crate::config::PackageConfig| cfg.gerrit_host.clone(),
    |_s, cfg: &crate::config::PackageConfig| {
        lx_lib::constants::homepage_for_gerrit(
            &cfg.github_repo,
            cfg.gerrit_host.as_deref().unwrap_or(""),
        )
    },
    parse_gerrit_url,
);

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
