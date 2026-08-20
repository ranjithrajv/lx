//! Reusable building blocks extracted from the `lpt` binary so they can be
//! shared across other projects:
//!
//! * [`cache`] — flock-guarded persistent download cache with TTL + checksum.
//! * [`checksum`] — SHA-256 helpers and pinned-metadata verification.
//! * [`github`] — octocrab-backed GitHub Releases client with a local JSON
//!   API cache (releases, assets, license/copyright lookups).
//! * [`lintian`] — run `lintian` against any `.deb` inside a Debian
//!   container and parse its output into a structured report.
//! * [`optimize`] — host resource detection and parallel-build tuning.
//! * [`progress`] — cross-process progress tracking with a TTY-friendly bar.
//! * [`telemetry`] — best-effort metrics/stage/failure logging.
//!
//! Each module is standalone (no dependency on the binary crate) so it can be
//! lifted into other tools as-is.

pub mod cache;
pub mod checksum;
pub mod github;
pub mod lintian;
pub mod optimize;
pub mod progress;
pub mod telemetry;
