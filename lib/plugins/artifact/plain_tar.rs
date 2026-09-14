// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use std::path::Path;

use super::ArtifactFormat;
use crate::plugins::plugin::plugin_identity;

pub struct PlainTar;

impl ArtifactFormat for PlainTar {
    fn recognizes(&self, file_name: &str) -> bool {
        file_name.ends_with(".tar")
    }

    fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
        let f = std::fs::File::open(archive)?;
        let mut tar = tar::Archive::new(f);
        tar.unpack(dest)
            .with_context(|| format!("failed to extract '{}'", archive.display()))
    }
}

plugin_identity!(PlainTar, "tar", "uncompressed tarball (.tar)");
