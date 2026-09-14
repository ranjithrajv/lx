// SPDX-License-Identifier: GPL-3.0-or-later

//! Package-index plugin dimension: one backend per package index, covering
//! both sides of the index lifecycle.
//!
//! This dimension merges what were two complementary plugin types:
//!
//! * the **write** side (formerly `RepoIndexer`): turn built artifacts into a
//!   servable repository index (`lx repo`) — `apt`, `opkg`, `pacman`, `apk`,
//!   `rpm`.
//! * the **read** side (formerly `IndexSource`): fan out across upstream
//!   package indexes (`lx index`) — `lx-community`, `aur`, `repology`.
//!
//! Both are a [`PackageIndex`]. Every role method has a default, so a backend
//! overrides only the half it implements; the other half fails with an
//! actionable error keyed off [`PackageIndex::id`]/[`PackageIndex::capabilities`].
//! This is the read/write counterpart of the `RepoIndexer`/`IndexSource`
//! split it replaces: a future backend that both publishes *and* consumes an
//! index (e.g. reading back a locally generated repository) implements both
//! halves; today the two sets remain disjoint.
//!
//! ## Canonical ids vs. formats
//!
//! Selection is by canonical [`id`](PackageIndex::id)
//! ([`BACKEND_IDS`]): `apt`, `opkg`, `pacman`, `apk`, `rpm`,
//! `lx-community`, `aur`, `repology`. The user-facing `lx repo --format`
//! vocabulary (`deb`/`ipk`/`arch`/…) is an alias mapped through
//! [`FORMAT_ALIASES`]/[`resolve_format`]; [`get_index_backend`] accepts either.
//!
//! ## Roles
//!
//! * Write backends are stateless (`AptIndexer`, …) and expose
//!   `file_extension`/`build_index`/`sign_index`.
//! * Read backends carry a configured instance name (`indexes.yaml` can
//!   rename/duplicate a source) returned by
//!   [`instance_name`](PackageIndex::instance_name); that name is what lands
//!   in `IndexHit::source` and what `--repo` filters on. [`id`](PackageIndex::id)
//!   stays the canonical backend id.

pub mod apk;
pub mod apt;
pub mod aur;
pub mod lx_community;
pub mod opkg;
pub mod pacman;
pub mod repology;
pub mod rpm;

use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::index::{IndexHit, InstallOpts};
use crate::plugins::plugin::Plugin;

/// Which roles a [`PackageIndex`] backend implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities(u8);

impl Capabilities {
    /// No role implemented.
    pub const NONE: Self = Self(0);
    /// Can consume an upstream index (`search`/`info`/`update`/`install`).
    pub const READ: Self = Self(0b01);
    /// Can publish a local index (`build_index`/`sign_index`).
    pub const WRITE: Self = Self(0b10);
    /// Both roles.
    pub const READ_WRITE: Self = Self(0b11);

    /// True when the read role is implemented.
    pub fn can_read(self) -> bool {
        self.0 & Self::READ.0 != 0
    }

    /// True when the write role is implemented.
    pub fn can_write(self) -> bool {
        self.0 & Self::WRITE.0 != 0
    }

    /// True when no role is implemented.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::fmt::Display for Capabilities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.can_read(), self.can_write()) {
            (true, true) => f.write_str("read+write"),
            (true, false) => f.write_str("read"),
            (false, true) => f.write_str("write"),
            (false, false) => f.write_str("none"),
        }
    }
}

/// Repository-level metadata passed from `lx repo` to a write backend.
pub struct IndexOptions<'a> {
    /// Suite/repository name (`stable`, `openwrt`, `core`, …).
    pub suite: &'a str,
    /// Origin/label recorded in the index (when the format has one).
    pub origin: &'a str,
    /// Components (apt `--components`).
    pub components: &'a str,
    /// Optional signing key for the index.
    pub sign_key: Option<&'a Path>,
    pub sign_key_id: &'a str,
}

/// One package-index backend: publishes a local index (write) and/or
/// consumes an upstream one (read).
///
/// Implementors are the merge of the former `RepoIndexer` and `IndexSource`:
/// `apt`, `opkg`, `pacman`, `apk`, `rpm` are write-capable; `lx-community`,
/// `aur` and `repology` are read-capable. A backend that does both is
/// possible (the trait does not restrict it).
///
/// Every role method has a default so a backend only writes the half it
/// supports; the defaults fail with an actionable message instead of
/// panicking. `id`, `description` and `capabilities` are required.
pub trait PackageIndex: Plugin {
    /// Canonical id (`apt`, `opkg`, `pacman`, `apk`, `rpm`, `lx-community`,
    /// `aur`, `repology`). See [`BACKEND_IDS`]. Defaults to the plugin's
    /// [`name`](Plugin::name).
    fn id(&self) -> &'static str {
        self.name()
    }

    /// Roles this backend implements.
    fn capabilities(&self) -> Capabilities;

    /// Configured instance label. Read backends override this with the
    /// `indexes.yaml` source name (used as `IndexHit::source` and the
    /// `--repo` filter); stateless write backends keep the [`id`](Self::id)
    /// default.
    fn instance_name(&self) -> &str {
        self.id()
    }

    // -----------------------------------------------------------------
    // Write role (was `RepoIndexer`)
    // -----------------------------------------------------------------

    /// Artifact file extension this backend consumes (without dot), if it
    /// can publish an index.
    fn file_extension(&self) -> Option<&'static str> {
        None
    }

    /// Write the repository index for `artifacts` into `dir`.
    fn build_index(&self, _dir: &Path, _artifacts: &[PathBuf], _opts: &IndexOptions) -> Result<()> {
        anyhow::bail!(
            "'{}' cannot publish a package index (read-only source)",
            self.id()
        )
    }

    /// Sign the index this backend just wrote.
    ///
    /// The default warns when a key was supplied but the backend has no
    /// supported scheme (matching the old `RepoIndexer` default).
    fn sign_index(&self, _dir: &Path, opts: &IndexOptions) -> Result<()> {
        if opts.sign_key.is_some() {
            eprintln!(
                "    ⚠ index signing is not supported for '{}' repositories; index left unsigned",
                self.id()
            );
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Read role (was `IndexSource`)
    // -----------------------------------------------------------------

    /// Search this index for projects matching `pattern`.
    fn search(&self, _pattern: Option<&str>) -> Result<Vec<IndexHit>> {
        anyhow::bail!("'{}' cannot search (write-only indexer)", self.id())
    }

    /// Look up one package in this index.
    fn info(&self, _package: &str) -> Result<Option<IndexHit>> {
        Ok(None)
    }

    /// Refresh this index's local cache; `Ok(true)` when it changed.
    fn update(&self) -> Result<bool> {
        Ok(false)
    }

    /// Install a package from this index.
    fn install(&self, _package: &str, _opts: InstallOpts) -> Result<()> {
        anyhow::bail!("'{}' is not installable", self.id())
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// A registered [`PackageIndex`] backend: its canonical id, capabilities and
/// a factory. `make` takes the configured instance name (write backends
/// ignore it; read backends label their cache with it).
#[derive(Clone, Copy, Debug)]
pub struct IndexBackend {
    /// Canonical id, see [`BACKEND_IDS`].
    pub id: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Roles this backend implements.
    pub capabilities: Capabilities,
    make: fn(name: &str) -> Box<dyn PackageIndex>,
}

impl IndexBackend {
    /// Instantiate this backend, labelling the instance with `name`.
    pub fn make(&self, name: &str) -> Box<dyn PackageIndex> {
        (self.make)(name)
    }
}

// Write-side factories. Repo indexers are stateless, so `name` is ignored.
fn make_apt(_name: &str) -> Box<dyn PackageIndex> {
    Box::new(apt::AptIndexer)
}

fn make_opkg(_name: &str) -> Box<dyn PackageIndex> {
    Box::new(opkg::OpkgIndexer)
}

fn make_pacman(_name: &str) -> Box<dyn PackageIndex> {
    Box::new(pacman::PacmanIndexer)
}

fn make_apk(_name: &str) -> Box<dyn PackageIndex> {
    Box::new(apk::ApkIndexer)
}

fn make_rpm(_name: &str) -> Box<dyn PackageIndex> {
    Box::new(rpm::RpmIndexer)
}

// Read-side factories. `name` is the configured `indexes.yaml` source name.
fn make_lx_community(name: &str) -> Box<dyn PackageIndex> {
    Box::new(lx_community::LxCommunitySource::new(name))
}

fn make_aur(name: &str) -> Box<dyn PackageIndex> {
    Box::new(aur::AurSource::new(name))
}

fn make_repology(name: &str) -> Box<dyn PackageIndex> {
    Box::new(repology::RepologySource::new(name))
}

/// All known backends, in [`BACKEND_IDS`] order.
pub fn all_index_backends() -> Vec<IndexBackend> {
    vec![
        IndexBackend {
            id: "apt",
            description: "apt repository (Packages, Packages.gz, Release, InRelease)",
            capabilities: Capabilities::WRITE,
            make: make_apt,
        },
        IndexBackend {
            id: "opkg",
            description: "opkg repository (Packages, Packages.gz)",
            capabilities: Capabilities::WRITE,
            make: make_opkg,
        },
        IndexBackend {
            id: "pacman",
            description: "pacman repository (<repo>.db.tar.gz from .PKGINFO)",
            capabilities: Capabilities::WRITE,
            make: make_pacman,
        },
        IndexBackend {
            id: "apk",
            description: "Alpine repository (APKINDEX.tar.gz from .PKGINFO)",
            capabilities: Capabilities::WRITE,
            make: make_apk,
        },
        IndexBackend {
            id: "rpm",
            description: "RPM repository (repodata/repomd.xml + primary.xml.gz)",
            capabilities: Capabilities::WRITE,
            make: make_rpm,
        },
        IndexBackend {
            id: "lx-community",
            description: "LX community index (recipes + per-release prebuilts)",
            capabilities: Capabilities::READ,
            make: make_lx_community,
        },
        IndexBackend {
            id: "aur",
            description: "Arch User Repository (PKGBUILD → native build)",
            capabilities: Capabilities::READ,
            make: make_aur,
        },
        IndexBackend {
            id: "repology",
            description: "Repology cross-distro metadata (read-only)",
            capabilities: Capabilities::READ,
            make: make_repology,
        },
    ]
}

/// Look up a backend by canonical id **or** `package_format` alias
/// (`deb`→`apt`, `arch`→`pacman`, `ipk`→`opkg`), case-insensitively.
pub fn get_index_backend(name_or_format: &str) -> Option<IndexBackend> {
    let id = canonical_id(name_or_format)?;
    all_index_backends().into_iter().find(|b| b.id == id)
}

// ---------------------------------------------------------------------------
// Canonical ids & format aliases
// ---------------------------------------------------------------------------

/// Canonical backend ids, in registration order: write-side repo indexers
/// followed by read-side index sources. Both share one namespace.
pub const BACKEND_IDS: &[&str] = &[
    // write (was RepoIndexer)
    "apt",
    "opkg",
    "pacman",
    "apk",
    "rpm",
    // read (was IndexSource)
    "lx-community",
    "aur",
    "repology",
];

/// User-facing `package_format` aliases (`lx repo --format`,
/// `package.yaml package_format:`) mapped to canonical ids.
pub const FORMAT_ALIASES: &[(&str, &str)] = &[
    ("deb", "apt"),
    ("ipk", "opkg"),
    ("arch", "pacman"),
    ("apk", "apk"),
    ("rpm", "rpm"),
];

/// Map a `package_format` to its canonical backend id.
pub fn resolve_format(format: &str) -> Option<&'static str> {
    let lower = format.trim().to_ascii_lowercase();
    FORMAT_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map(|(_, id)| *id)
}

/// Resolve either a canonical id or a `package_format` alias to the id.
/// Case-insensitive.
pub fn canonical_id(name: &str) -> Option<&'static str> {
    let name = name.trim().to_ascii_lowercase();
    if let Some(id) = BACKEND_IDS.iter().find(|id| **id == name) {
        return Some(id);
    }
    resolve_format(&name)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Artifacts in `dir` whose extension matches `ext`, sorted.
pub fn artifacts_with_ext(dir: &Path, ext: &str) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().map(|e| e == ext).unwrap_or(false))
        .collect();
    out.sort();
    Ok(out)
}

/// Parse `key = value` lines (used for `.PKGINFO` and apk control metadata).
pub fn parse_key_value(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_aliases_resolve_to_canonical_ids() {
        assert_eq!(resolve_format("deb"), Some("apt"));
        assert_eq!(resolve_format("ipk"), Some("opkg"));
        assert_eq!(resolve_format("arch"), Some("pacman"));
        assert_eq!(resolve_format("apk"), Some("apk"));
        assert_eq!(resolve_format("rpm"), Some("rpm"));
        // Case/whitespace insensitive; unknown formats and sources are None.
        assert_eq!(resolve_format("  DEB "), Some("apt"));
        assert_eq!(resolve_format("lx-community"), None);
        assert_eq!(resolve_format("nope"), None);
    }

    #[test]
    fn canonical_id_accepts_ids_and_format_aliases() {
        for id in BACKEND_IDS {
            assert_eq!(canonical_id(id), Some(*id), "id {id} did not round-trip");
        }
        assert_eq!(canonical_id("arch"), Some("pacman"));
        assert_eq!(canonical_id("PACMAN"), Some("pacman"));
        assert_eq!(canonical_id(" repology "), Some("repology"));
        assert_eq!(canonical_id("not-a-backend"), None);
    }

    #[test]
    fn capabilities_report_roles() {
        assert!(Capabilities::READ.can_read());
        assert!(!Capabilities::READ.can_write());
        assert!(Capabilities::WRITE.can_write());
        assert!(!Capabilities::WRITE.can_read());
        assert!(Capabilities::READ_WRITE.can_read());
        assert!(Capabilities::READ_WRITE.can_write());
        assert!(Capabilities::NONE.is_empty());
        assert_eq!(
            Capabilities::READ | Capabilities::WRITE,
            Capabilities::READ_WRITE
        );
        assert_eq!(Capabilities::READ_WRITE.to_string(), "read+write");
        assert_eq!(Capabilities::WRITE.to_string(), "write");
    }

    #[test]
    fn registry_covers_backend_ids() {
        let ids: Vec<&str> = all_index_backends().iter().map(|b| b.id).collect();
        assert_eq!(ids, BACKEND_IDS);
        for id in BACKEND_IDS {
            let backend = get_index_backend(id).unwrap_or_else(|| panic!("missing backend {id}"));
            assert_eq!(backend.id, *id);
            assert!(!backend.capabilities.is_empty());
        }
    }
}
