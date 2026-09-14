// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::{Path, PathBuf};

use super::{IndexOptions, RepoIndexer};

/// Debian/Ubuntu apt repository: `Packages`, `Packages.gz`, `Release`, and
/// a clearsigned `InRelease` when a key is given. Delegates to the original
/// implementation in `lib/repo.rs` so behavior is unchanged.
pub struct AptIndexer;

impl RepoIndexer for AptIndexer {
    fn name(&self) -> &'static str {
        "apt"
    }

    fn format(&self) -> &'static str {
        "deb"
    }

    fn description(&self) -> &'static str {
        "apt repository (Packages, Packages.gz, Release, InRelease)"
    }

    fn file_extension(&self) -> &'static str {
        "deb"
    }

    fn build_index(&self, dir: &Path, artifacts: &[PathBuf], opts: &IndexOptions) -> Result<()> {
        crate::repo::build_apt_index(
            dir,
            artifacts,
            opts.suite,
            opts.origin,
            opts.sign_key,
            opts.sign_key_id,
        )
    }
}
