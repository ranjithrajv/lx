// SPDX-License-Identifier: GPL-3.0-or-later

//! Reusable building blocks extracted from the `lx` binary so they can be
//! shared across other projects:
//!
//! * [`cache`] — flock-guarded persistent download cache with TTL + checksum.
//! * [`checksum`] — SHA-256 helpers and pinned-metadata verification.
//! * [`debarchive`] — build a `.deb` (ar + control.tar.gz + data.tar.gz)
//!   entirely in-process, no `dpkg-deb` required.
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
//! * [`constants`] — centralized hosts, timeouts, TTLs, file modes.
//! * [`sign`] — post-build artifact signing (gpg detach-sign for `.deb`,
//!   native embedded PGP for `.rpm`).
//! * [`archarchive`] — build a `.pkg.tar.zst` (Arch pacman) entirely
//!   in-process, no `makepkg` required.
//! * [`gitlab`] — GitLab Releases client mirroring `github`'s surface.
//! * [`gitea`] — Gitea Releases client (codeberg.org, self-hosted).
//! * [`forgejo`] — Forgejo Releases client (Codeberg, self-hosted, Gitea-compatible).
//! * [`bitbucket`] — Bitbucket Cloud downloads as pseudo-releases.
//! * [`gerrit`] — Gerrit Code Review tags as pseudo-releases.
//! * [`progress`] — cross-process progress tracking with a TTY-friendly bar.
//! * [`telemetry`] — best-effort metrics/stage/failure logging.
//! * [`lock`] — `package.lock`: pins each arch's resolved (tag, asset,
//!   checksum) so a build fails closed on upstream drift.
//!
//! Everything the `lx` binary does lives here too (`build`, `cli`, `config`,
//! `plugins`, …) — `src/main.rs` is a thin wrapper that just calls
//! [`cli::run`]. Keeping it all in one crate lets `tests/` exercise the CLI's
//! internals as ordinary integration tests.

#![recursion_limit = "256"]

// Lets code written against the crate's own public API (originally as an
// external dependency, back when `src/` was a separate binary crate) keep
// using `lx_lib::` qualified paths unchanged after the merge.
extern crate self as lx_lib;

pub mod archarchive;
pub mod bitbucket;
pub mod build;
pub mod cache;
pub mod checksum;
pub mod cli;
pub mod config;
pub mod constants;
pub mod debarchive;
pub mod debs;
pub mod discovery;
pub mod elfdeps;
pub mod forgejo;
pub mod gerrit;
pub mod gitea;
pub mod github;
pub mod gitlab;
pub mod go_native;
pub mod http;
pub mod index;
pub mod install;
pub mod install_pkg;
pub mod lintian;
pub mod list;
pub mod lock;
pub mod manifest;
pub mod migrate;
pub mod optimize;
pub mod pkgmeta;
pub mod plugins;
pub mod progress;
pub mod reinstall;
pub mod remove;
pub mod repo;
pub mod rollback;
pub mod rpmarchive;
pub mod sbom;
pub mod scandeps;
pub mod schema;
pub mod search;
pub mod show;
pub mod sign;
pub mod source;
pub mod source_client;
pub mod sourcebuild;
pub mod summary;
pub mod telemetry;
pub mod templating;
pub mod update;
pub mod upgrade;
pub mod validate;
pub mod wizard;
