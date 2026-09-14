// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use std::path::Path;

use super::ArtifactFormat;
use crate::plugins::plugin::plugin_identity;

/// Zip archives (`.zip`), extracted with the pure-Rust `zip` crate
/// (deflate only). Preserves unix modes when the archive records them and
/// refuses entries that would escape the destination directory.
pub struct Zip;

impl ArtifactFormat for Zip {
    fn recognizes(&self, file_name: &str) -> bool {
        file_name.ends_with(".zip")
    }

    fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
        let file = std::fs::File::open(archive)
            .with_context(|| format!("opening '{}'", archive.display()))?;
        let mut zip = zip::ZipArchive::new(file)
            .with_context(|| format!("reading zip '{}'", archive.display()))?;

        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .with_context(|| format!("reading zip entry {i} of '{}'", archive.display()))?;

            // `enclosed_name` rejects absolute paths and `..` traversal.
            let Some(rel) = entry.enclosed_name() else {
                bail!(
                    "zip entry '{}' escapes the destination directory",
                    entry.name()
                );
            };
            let out = dest.join(rel);

            if entry.is_dir() {
                std::fs::create_dir_all(&out)?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)
                .with_context(|| format!("creating '{}'", out.display()))?;
            std::io::copy(&mut entry, &mut f)
                .with_context(|| format!("extracting '{}'", out.display()))?;

            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
            }
        }
        Ok(())
    }
}

plugin_identity!(Zip, "zip", "zip archive (.zip)");
