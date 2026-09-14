// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use std::path::Path;

use super::ArtifactFormat;
use crate::plugins::plugin::plugin_identity;

pub struct TarGz;

impl ArtifactFormat for TarGz {
    fn aliases(&self) -> &'static [&'static str] {
        &["tgz", "gz"]
    }

    fn recognizes(&self, file_name: &str) -> bool {
        file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz")
    }

    fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
        let f = std::fs::File::open(archive)?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(dest)
            .with_context(|| format!("failed to extract '{}'", archive.display()))
    }
}

plugin_identity!(TarGz, "tar.gz", "gzip-compressed tarball (.tar.gz / .tgz)");
