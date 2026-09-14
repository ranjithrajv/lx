// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use super::{SignContext, SignOutcome, Signer};

/// Embedded RPM PGP signature. The `rpm` crate writes the signature into the
/// package header while `rpmarchive::build` assembles it, so this backend is
/// a declaration of how RPM signing works rather than a post-build step.
pub struct RpmPgp;

impl Signer for RpmPgp {
    fn name(&self) -> &'static str {
        "rpm-pgp"
    }

    fn description(&self) -> &'static str {
        "Embedded RPM PGP signature (written into the package header)"
    }

    fn supports(&self, format: &str, _method: &str) -> bool {
        format.eq_ignore_ascii_case("rpm")
    }

    fn embedded(&self) -> bool {
        true
    }

    fn sign(&self, _artifact: &Path, _ctx: &SignContext) -> Result<SignOutcome> {
        Ok(SignOutcome::Embedded)
    }
}
