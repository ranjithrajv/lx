// SPDX-License-Identifier: GPL-3.0-or-later

//! Source provider plugins for auto-discovery.
//!
//! Each source (GitHub, GitLab, …) implements `ForgeSource`. The discovery
//! and build pipelines are source-agnostic: they resolve a `Release` (list of
//! assets) via the selected plugin, then reuse the same `match_assets` /
//! checksum / download logic regardless of provider.

pub mod bitbucket;
pub mod custom;
pub mod forgejo;
pub mod gerrit;
pub mod gitea;
pub mod gitee;
pub mod github;
pub mod gitlab;
pub mod sourceforge;

use anyhow::Result;
use std::path::Path;

use crate::plugins::plugin::{Plugin, PluginSet};
use lx_lib::github::{Release, ReleaseMeta, RepoLicense};

/// A source/provider plugin (auto-discovery).
///
/// Implementors are stateless; constructed per-call with the caller's
/// `token` and `cache_dir` so the trait stays `Send+Sync` and testable.
pub trait ForgeSource: Plugin {
    /// Env var holding this provider's auth token (e.g. "GITLAB_TOKEN").
    /// `None` when the provider needs no token.
    /// Used by `resolve_forge_token` so config/build stay OCP — adding a
    /// provider never requires editing them, only this method.
    fn token_env(&self) -> Option<&'static str>;

    /// Config field name for a self-hosted host override (e.g. Some("gitlab_host")).
    /// `None` for single-host providers (github).
    fn host_config_key(&self) -> Option<&'static str> {
        None
    }

    /// Parse a provider URL into `owner/repo` (for zero-config `lx build https://…`).
    /// Returns `None` if the URL does not belong to this provider.
    fn parse_url(&self, url: &str) -> Option<String>;

    /// Fetch latest release for `repo` ("owner/repo").
    fn latest_release(
        &self,
        repo: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release>;

    /// Fetch release by tag.
    fn release_by_tag(
        &self,
        repo: &str,
        tag: &str,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Release>;

    /// List `per_page` recent releases (for `suggest_versions`).
    fn releases(
        &self,
        repo: &str,
        per_page: u8,
        token: Option<&str>,
        cache_dir: Option<&Path>,
    ) -> Result<Vec<ReleaseMeta>>;

    // --- Repo-info capabilities (ISP): defaulted to "unsupported" so
    // providers like gitlab/gitea/bitbucket/gerrit implement only what their
    // API actually offers, instead of being forced to write Ok(None) stubs.
    // Callers (`fetch_upstream_license`) treat these results as best-effort
    // and fall back to `license_spdx` from package.yaml.

    /// Repo license (best-effort). Default: unsupported → `Ok(None)`.
    fn repo_license(
        &self,
        _repo: &str,
        _token: Option<&str>,
        _cache_dir: Option<&Path>,
    ) -> Result<Option<RepoLicense>> {
        Ok(None)
    }

    /// Repo root file list (for dual LICENSE-APACHE/MIT detection).
    /// Default: unsupported → empty list.
    fn repo_root(
        &self,
        _repo: &str,
        _token: Option<&str>,
        _cache_dir: Option<&Path>,
    ) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Raw text of a file in the repo root. Default: unsupported → `Ok(None)`.
    fn repo_file_text(
        &self,
        _repo: &str,
        _path: &str,
        _token: Option<&str>,
        _cache_dir: Option<&Path>,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// Streaming GET for sidecar / asset downloads.
    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn std::io::Read + Send>>;
}

/// All known source plugins.
pub fn all_forge_sources() -> Vec<Box<dyn ForgeSource>> {
    vec![
        Box::new(github::GithubForgeSource),
        Box::new(gitlab::GitlabForgeSource),
        Box::new(gitea::GiteaForgeSource),
        Box::new(forgejo::ForgejoForgeSource),
        Box::new(bitbucket::BitbucketForgeSource),
        Box::new(gitee::GiteeForgeSource),
        Box::new(sourceforge::SourceForgeForgeSource),
        Box::new(custom::CustomForgeSource),
        Box::new(gerrit::GerritForgeSource),
    ]
}

/// Lookup source plugin by name (case-insensitive).
pub fn get_forge_source(name: &str) -> Option<Box<dyn ForgeSource>> {
    PluginSet::new(all_forge_sources()).take(name)
}

/// Available source names for error messages.
pub fn forge_source_names() -> Vec<&'static str> {
    PluginSet::new(all_forge_sources()).names()
}

/// Apply the provider's host override from config to its env var, so
/// `lib::<provider>::Client` (which reads env) picks it up. Data-driven via
/// `host_config_key` — adding a provider requires no edits at call sites.
/// Lives here (not in `build.rs`) because host/token resolution is
/// provider-domain logic (SRP).
pub fn apply_forge_host(source: &dyn ForgeSource, cfg: &crate::config::PackageConfig) {
    let Some(key) = source.host_config_key() else {
        return;
    };
    let Some(host) = cfg_host(cfg, key) else {
        return;
    };
    let env_name = format!("{}_HOST", key.trim_end_matches("_host").to_uppercase());
    std::env::set_var(env_name, host);
}

fn cfg_host<'a>(cfg: &'a crate::config::PackageConfig, key: &str) -> Option<&'a str> {
    match key {
        "gitlab_host" => cfg.gitlab_host.as_deref(),
        "gitea_host" => cfg.gitea_host.as_deref(),
        "forgejo_host" => cfg.forgejo_host.as_deref(),
        "bitbucket_host" => cfg.bitbucket_host.as_deref(),
        "gerrit_host" => cfg.gerrit_host.as_deref(),
        "gitee_host" => cfg.gitee_host.as_deref(),
        _ => None,
    }
}

/// Resolve the auth token for a provider: its `token_env()` first, then the
/// CLI `--token`. Data-driven via the plugin — call sites never enumerate
/// providers.
pub fn resolve_forge_token(source: &dyn ForgeSource, cli_token: Option<&str>) -> Option<String> {
    if let Some(env_name) = source.token_env() {
        if let Ok(t) = std::env::var(env_name) {
            if !t.trim().is_empty() {
                return Some(t);
            }
        }
        return cli_token.map(|s| s.to_string());
    }
    cli_token
        .map(|s| s.to_string())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
}

/// Parse any supported provider URL into `owner/repo`. Tries each plugin's
/// `parse_url` in registry order (github first).
pub fn parse_any_forge_url(url: &str) -> Option<(String, String)> {
    for p in all_forge_sources() {
        if let Some(repo) = p.parse_url(url) {
            return Some((p.name().to_string(), repo));
        }
    }
    None
}

/// Shared URL parser: extract `owner/repo` from a `https://<host>/<owner>/<repo>[.git]` URL.
///
/// Used by every forge plugin's `parse_*_url` instead of each duplicating this logic.
pub fn parse_host_url(s: &str, host: &str) -> Option<String> {
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

/// Implement `Plugin` + `ForgeSource` for an "owner/repo" provider client,
/// delegating the methods every such provider shares. A provider file then
/// carries only its identity and URL parser instead of seven near-identical
/// methods:
///
/// ```ignore
/// repo_forge_source!(
///     GitlabForgeSource, lx_lib::gitlab::GitlabClient,
///     "gitlab", "GitLab Releases …",
///     Some("GITLAB_TOKEN"), Some("gitlab_host"), parse_gitlab_url,
/// );
/// ```
///
/// The repo-info methods (`repo_license`/`repo_root`/`repo_file_text`) are
/// generated too; clients that don't support them return empty results.
macro_rules! repo_forge_source {
    ($ty:ty, $client:ty, $name:literal, $desc:literal, $token_env:expr, $host_key:expr, $parse:path $(,)?) => {
        impl $crate::plugins::plugin::Plugin for $ty {
            fn name(&self) -> &'static str {
                $name
            }

            fn description(&self) -> &'static str {
                $desc
            }
        }

        impl $crate::plugins::forge::ForgeSource for $ty {
            fn token_env(&self) -> Option<&'static str> {
                $token_env
            }

            fn host_config_key(&self) -> Option<&'static str> {
                $host_key
            }

            fn parse_url(&self, url: &str) -> Option<String> {
                $parse(url)
            }

            fn latest_release(
                &self,
                repo: &str,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<$crate::github::Release> {
                let (owner, repo_name) = $crate::discovery::split_repo(repo)?;
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.latest_release(owner, repo_name)
            }

            fn release_by_tag(
                &self,
                repo: &str,
                tag: &str,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<$crate::github::Release> {
                let (owner, repo_name) = $crate::discovery::split_repo(repo)?;
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.release_by_tag(owner, repo_name, tag)
            }

            fn releases(
                &self,
                repo: &str,
                per_page: u8,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<Vec<$crate::github::ReleaseMeta>> {
                let (owner, repo_name) = $crate::discovery::split_repo(repo)?;
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.releases(owner, repo_name, per_page)
            }

            fn raw_get(
                &self,
                url: &str,
                token: Option<&str>,
            ) -> ::anyhow::Result<Box<dyn std::io::Read + Send>> {
                let client = $crate::source_client::new_client_for::<$client>(token, None)?;
                client.raw_get(url)
            }

            fn repo_license(
                &self,
                repo: &str,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<Option<$crate::github::RepoLicense>> {
                let (owner, repo_name) = $crate::discovery::split_repo(repo)?;
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.repo_license(owner, repo_name)
            }

            fn repo_root(
                &self,
                repo: &str,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<Vec<String>> {
                let (owner, repo_name) = $crate::discovery::split_repo(repo)?;
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.repo_root(owner, repo_name)
            }

            fn repo_file_text(
                &self,
                repo: &str,
                path: &str,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<Option<String>> {
                let (owner, repo_name) = $crate::discovery::split_repo(repo)?;
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.repo_file_text(owner, repo_name, path)
            }
        }
    };
}

pub(crate) use repo_forge_source;

/// The single-project counterpart of [`repo_forge_source!`]: Gerrit and
/// SourceForge identify a package by a bare project name (which may itself
/// contain slashes), so no `owner/repo` split is applied.
macro_rules! project_forge_source {
    ($ty:ty, $client:ty, $name:literal, $desc:literal, $token_env:expr, $host_key:expr, $parse:path $(,)?) => {
        impl $crate::plugins::plugin::Plugin for $ty {
            fn name(&self) -> &'static str {
                $name
            }

            fn description(&self) -> &'static str {
                $desc
            }
        }

        impl $crate::plugins::forge::ForgeSource for $ty {
            fn token_env(&self) -> Option<&'static str> {
                $token_env
            }

            fn host_config_key(&self) -> Option<&'static str> {
                $host_key
            }

            fn parse_url(&self, url: &str) -> Option<String> {
                $parse(url)
            }

            fn latest_release(
                &self,
                repo: &str,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<$crate::github::Release> {
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.latest_release(repo)
            }

            fn release_by_tag(
                &self,
                repo: &str,
                tag: &str,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<$crate::github::Release> {
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.release_by_tag(repo, tag)
            }

            fn releases(
                &self,
                repo: &str,
                per_page: u8,
                token: Option<&str>,
                cache_dir: Option<&std::path::Path>,
            ) -> ::anyhow::Result<Vec<$crate::github::ReleaseMeta>> {
                let client = $crate::source_client::new_client_for::<$client>(token, cache_dir)?;
                client.releases(repo, per_page)
            }

            fn raw_get(
                &self,
                url: &str,
                token: Option<&str>,
            ) -> ::anyhow::Result<Box<dyn std::io::Read + Send>> {
                let client = $crate::source_client::new_client_for::<$client>(token, None)?;
                client.raw_get(url)
            }
        }
    };
}

pub(crate) use project_forge_source;
