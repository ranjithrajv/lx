// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use std::path::Path;

use super::ArtifactFormat;

pub struct PlainTar;

impl ArtifactFormat for PlainTar {
    fn name(&self) -> &'static str {
        "tar"
    }

    fn description(&self) -> &'static str {
        "uncompressed tarball (.tar)"
    }

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
