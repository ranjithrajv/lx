// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use super::ArtifactFormat;

/// A single-file payload: copy the asset into the payload directory under
/// its own name. This is the catch-all format and must be registered last.
pub struct Raw;

impl ArtifactFormat for Raw {
    fn name(&self) -> &'static str {
        "raw"
    }

    fn description(&self) -> &'static str {
        "single binary/asset copied as-is (no archive)"
    }

    fn recognizes(&self, _file_name: &str) -> bool {
        true
    }

    fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
        let name = archive
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("binary");
        std::fs::copy(archive, dest.join(name))?;
        Ok(())
    }
}
