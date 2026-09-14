// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use std::path::Path;

use super::ArtifactFormat;
use crate::plugins::plugin::plugin_identity;

pub struct TarXz;

impl ArtifactFormat for TarXz {
    fn aliases(&self) -> &'static [&'static str] {
        &["txz", "xz"]
    }

    fn recognizes(&self, file_name: &str) -> bool {
        file_name.ends_with(".tar.xz") || file_name.ends_with(".txz")
    }

    fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
        let data = std::fs::read(archive)
            .with_context(|| format!("failed to read '{}'", archive.display()))?;
        let decoder = lzma_rust2::XzReader::new(data.as_slice(), true);
        let mut tar = tar::Archive::new(decoder);
        tar.unpack(dest)
            .with_context(|| format!("failed to extract '{}'", archive.display()))
    }
}

plugin_identity!(TarXz, "tar.xz", "xz-compressed tarball (.tar.xz / .txz)");
