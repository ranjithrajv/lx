// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use std::path::Path;

use super::ArtifactFormat;

pub struct TarZst;

impl ArtifactFormat for TarZst {
    fn name(&self) -> &'static str {
        "tar.zst"
    }

    fn description(&self) -> &'static str {
        "zstd-compressed tarball (.tar.zst / .tzst)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["tzst", "zst", "tar.zstd"]
    }

    fn recognizes(&self, file_name: &str) -> bool {
        file_name.ends_with(".tar.zst")
            || file_name.ends_with(".tzst")
            || file_name.ends_with(".tar.zstd")
    }

    fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
        let f = std::fs::File::open(archive)?;
        let decoder = zstd::stream::read::Decoder::new(f)
            .with_context(|| format!("failed to zstd-decode '{}'", archive.display()))?;
        let mut tar = tar::Archive::new(decoder);
        tar.unpack(dest)
            .with_context(|| format!("failed to extract '{}'", archive.display()))
    }
}
