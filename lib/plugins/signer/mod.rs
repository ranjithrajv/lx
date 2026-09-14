// SPDX-License-Identifier: GPL-3.0-or-later

//! Signing-backend plugins.
//!
//! Signing is a lifecycle of its own: it happens *after* the packager has
//! produced an artifact, it has different inputs (a key, an optional
//! passphrase, a role), and it has a different failure contract (a
//! configured-but-failed sign must never yield an unsigned artifact).
//! Modeling it as its own dimension keeps that contract out of the
//! packagers — and, unlike the deb/rpm `if format == "deb"` branches that
//! preceded it, lets any format gain signing by registering a plugin.
//!
//! Two kinds of backend exist:
//!
//! * **detached** ([`SignOutcome::Detached`]): produce a sibling signature
//!   file after the artifact is built (`gpg-detach`).
//! * **embedded** ([`SignOutcome::Embedded`]): the signature is part of the
//!   artifact and is written by the packager while building it (`rpm-pgp`
//!   embeds in the RPM header; `deb-debsign` embeds `_gpg{type}` in the ar).
//!   These are still plugins so `signer_for` is the single place that
//!   decides *how* a format is signed; build.rs simply skips a post-build
//!   step for them.
//!
//! Selection: [`signer_for(format, method)`] → first registered backend
//! whose [`Signer::supports`] returns true.

pub mod deb_debsign;
pub mod gpg_detach;
pub mod rpm_pgp;

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Everything a signer needs, resolved from config + CLI by the caller.
pub struct SignContext<'a> {
    pub key_file: &'a Path,
    pub key_id: &'a str,
    pub passphrase: Option<&'a str>,
    /// debsign role (`origin` / `maint` / `archive`).
    pub sign_type: &'a str,
}

/// What a signer did (or that the packager already did it).
#[derive(Debug)]
pub enum SignOutcome {
    /// A detached signature was written next to the artifact.
    Detached(PathBuf),
    /// The artifact already carries an embedded signature.
    Embedded,
}

/// A signing backend.
pub trait Signer: Send + Sync {
    /// Canonical name (`--signer`), e.g. `gpg-detach`.
    fn name(&self) -> &'static str;

    fn description(&self) -> &'static str;

    /// True when this backend can sign `format` under `method`.
    fn supports(&self, format: &str, method: &str) -> bool;

    /// True when the packager writes the signature into the artifact while
    /// building it (no post-build step). Default: false.
    fn embedded(&self) -> bool {
        false
    }

    /// Sign a built artifact. Embedded backends report [`SignOutcome::Embedded`]
    /// because the work already happened during packaging.
    fn sign(&self, artifact: &Path, ctx: &SignContext) -> Result<SignOutcome>;
}

/// All known signing backends, in selection order (embedded/most-specific
/// first, generic detached last).
pub fn all_signers() -> Vec<Box<dyn Signer>> {
    vec![
        Box::new(rpm_pgp::RpmPgp),
        Box::new(deb_debsign::DebDebsign),
        Box::new(gpg_detach::GpgDetach),
    ]
}

/// Look up a signing backend by name (case-insensitive).
pub fn get_signer(name: &str) -> Option<Box<dyn Signer>> {
    let lower = name.trim().to_ascii_lowercase();
    all_signers().into_iter().find(|s| s.name() == lower)
}

/// Pick the backend for a `(format, method)` pair.
pub fn signer_for(format: &str, method: &str) -> Option<Box<dyn Signer>> {
    all_signers()
        .into_iter()
        .find(|s| s.supports(format, method))
}

pub fn signer_names() -> Vec<&'static str> {
    all_signers().iter().map(|s| s.name()).collect()
}
