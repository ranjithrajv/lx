// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;

/// Uniform construction contract shared by every source-provider client
/// (`github`/`gitlab`/`gitea`/`forgejo`/`bitbucket`/`gerrit`).
///
/// Exists to deduplicate the `ForgeSource` glue: each plugin method once
/// repeated `Client::with_cache(token.map(|s| s.to_string()),
/// cache_dir.map(|p| p.to_path_buf()))`. The `forge` layer's
/// `repo_forge_source!`/`project_forge_source!` macros call
/// [`new_client_for::<C>()`] instead — no per-provider boilerplate.
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
