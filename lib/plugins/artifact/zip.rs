// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Result};
use std::path::Path;

use super::ArtifactFormat;

/// Zip archives are recognized (so `zip` is a real, selectable format and
/// `guess_format` routes them here) but extraction is not implemented: lx
/// deliberately carries no zip decompressor yet. Fails with an actionable
/// error instead of the old generic "unsupported artifact_format".
pub struct Zip;

impl ArtifactFormat for Zip {
    fn name(&self) -> &'static str {
        "zip"
    }

    fn description(&self) -> &'static str {
        "zip archive (.zip) — recognized, extraction not yet implemented"
    }

    fn recognizes(&self, file_name: &str) -> bool {
        file_name.ends_with(".zip")
    }

    fn extract(&self, archive: &Path, _dest: &Path) -> Result<()> {
        bail!(
            "zip extraction is not implemented (asset '{}'); \
             use a tar.gz/tar.xz/tar.zst asset, or `--from-dir` after unpacking manually",
            archive.display()
        )
    }
}
