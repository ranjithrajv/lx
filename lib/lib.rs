//! Reusable building blocks extracted from the `lpt` binary so they can be
//! shared across other projects:
//!
//! * [`cache`] — flock-guarded persistent download cache with TTL + checksum.
//! * [`checksum`] — SHA-256 helpers and pinned-metadata verification.
//! * [`debarchive`] — build a `.deb` (ar + control.tar.gz + data.tar.gz)
//!   entirely in-process, no `dpkg-deb` or Docker required.
//! * [`elfdeps`] — read an ELF binary's `DT_NEEDED` shared-library
//!   dependencies natively, no `ldd`/`objdump`/`readelf` required.
//! * [`github`] — octocrab-backed GitHub Releases client with a local JSON
//!   API cache (releases, assets, license/copyright lookups).
//! * [`lintian`] — run the host `lintian` binary against a `.deb` and parse
//!   its output into a structured report.
//! * [`optimize`] — host resource detection and parallel-build tuning.
//! * [`pkgmeta`] — Debian package metadata rendering (epoch/version,
//!   reproducible-builds-aware timestamps, changelog/copyright bodies)
//!   shared by both the binary `.deb` and its source package.
//! * [`progress`] — cross-process progress tracking with a TTY-friendly bar.
//! * [`telemetry`] — best-effort metrics/stage/failure logging.
//!
//! Each module is standalone (no dependency on the binary crate) so it can be
//! lifted into other tools as-is.

pub mod cache;
pub mod checksum;
pub mod debarchive;
pub mod elfdeps;
pub mod github;
pub mod lintian;
pub mod optimize;
pub mod pkgmeta;
pub mod progress;
pub mod telemetry;
