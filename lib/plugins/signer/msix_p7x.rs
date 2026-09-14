// SPDX-License-Identifier: GPL-3.0-or-later

//! Native MSIX signing backend.
//!
//! The signature (`AppxSignature.p7x`) is a member of the package, written by
//! the MSIX packager itself ([`crate::plugins::msix`] +
//! [`lx_lib::msixarchive`]). This backend only tells the post-build step that
//! the artifact is already signed, so it isn't reported as unsigned.

use anyhow::Result;
use std::path::Path;

use super::{SignContext, SignOutcome, Signer};
use crate::plugins::plugin::plugin_identity;

pub struct MsixP7x;

impl Signer for MsixP7x {
    fn supports(&self, format: &str, _method: &str) -> bool {
        format.eq_ignore_ascii_case("msix")
    }

    fn embedded(&self) -> bool {
        true
    }

    fn sign(&self, _artifact: &Path, _ctx: &SignContext) -> Result<SignOutcome> {
        Ok(SignOutcome::Embedded)
    }
}

plugin_identity!(
    MsixP7x,
    "msix-p7x",
    "Native MSIX AppxSignature.p7x (embedded by the packager)"
);
