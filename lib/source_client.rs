// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;

/// Uniform construction contract shared by every source-provider client
/// (`github`/`gitlab`/`gitea`/`forgejo`/`bitbucket`/`gerrit`).
///
/// Exists to deduplicate the `ForgeSource` glue: each plugin method
/// previously repeated `Client::with_cache(token.map(|s| s.to_string()),
/// cache_dir.map(|p| p.to_path_buf()))`. With this trait, plugin impls call
/// [`new_client_for::<C>()`] instead — one line, no per-method boilerplate.
pub trait ClientNew: Sized {
    fn with_cache(token: Option<String>, cache_dir: Option<std::path::PathBuf>) -> Result<Self>;
}

/// Build any provider client from the plugin-call-shaped arguments.
pub fn new_client_for<C: ClientNew>(
    token: Option<&str>,
    cache_dir: Option<&std::path::Path>,
) -> Result<C> {
    C::with_cache(
        token.map(|s| s.to_string()),
        cache_dir.map(|p| p.to_path_buf()),
    )
}

/// Best-effort repo-info methods shared by provider clients. Providers that
/// don't support a capability return empty results rather than erroring,
/// making the "may be unsupported" part of the type-level contract explicit.
pub trait RepoInfo {
    fn repo_license(&self, owner: &str, repo: &str) -> Result<Option<crate::github::RepoLicense>>;
    fn repo_root(&self, owner: &str, repo: &str) -> Result<Vec<String>>;
    fn repo_file_text(&self, owner: &str, repo: &str, path: &str) -> Result<Option<String>>;
}
