// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::{Path, PathBuf};

use super::{Capabilities, IndexOptions, PackageIndex};
use crate::plugins::plugin::plugin_identity;

/// Debian/Ubuntu apt repository: `Packages`, `Packages.gz`, `Release`, and
/// a clearsigned `InRelease` when a key is given. Delegates to the original
/// implementation in `lib/repo.rs` so behavior is unchanged.
pub struct AptIndexer;

plugin_identity!(
    AptIndexer,
    "apt",
    "apt repository (Packages, Packages.gz, Release, InRelease)"
);

impl PackageIndex for AptIndexer {
    fn capabilities(&self) -> Capabilities {
        Capabilities::WRITE
    }

    fn file_extension(&self) -> Option<&'static str> {
        Some("deb")
    }

    fn build_index(&self, dir: &Path, artifacts: &[PathBuf], opts: &IndexOptions) -> Result<()> {
        crate::repo::build_apt_index(dir, artifacts, opts.suite, opts.origin)
    }

    fn sign_index(&self, dir: &Path, opts: &IndexOptions) -> Result<()> {
        let Some(key) = opts.sign_key else {
            return Ok(());
        };
        let release = std::fs::read(dir.join("Release"))?;
        let req = lx_lib::sign::SignRequest {
            key_file: key,
            key_id: opts.sign_key_id,
            passphrase: None,
        };
        // `InRelease` is an inline-clearsigned Release (apt's primary
        // authenticated index); `Release.gpg` is the armored detached form
        // older clients fetch.
        let inline = lx_lib::sign::clearsign_inline(&release, &req)?;
        std::fs::write(dir.join("InRelease"), inline)?;
        let detached = lx_lib::sign::clearsign(&release, &req)?;
        std::fs::write(dir.join("Release.gpg"), detached)?;
        println!("wrote InRelease, Release.gpg (clearsigned)");
        Ok(())
    }
}
