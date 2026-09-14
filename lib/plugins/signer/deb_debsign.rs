// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use super::{SignContext, SignOutcome, Signer};

/// Embedded `.deb` signing (`_gpg{role}` ar member). The signature is
/// written by the deb *packager* while it builds the ar container (the
/// control/data payload must be signed as it is assembled), so this backend
/// exists to make the method visible and selectable, not to do a post-build
/// step.
pub struct DebDebsign;

impl Signer for DebDebsign {
    fn name(&self) -> &'static str {
        "deb-debsign"
    }

    fn description(&self) -> &'static str {
        "Embedded `.deb` signature (`_gpg{origin,maint,archive}` ar member)"
    }

    fn supports(&self, format: &str, method: &str) -> bool {
        format.eq_ignore_ascii_case("deb") && method.eq_ignore_ascii_case("debsign")
    }

    fn embedded(&self) -> bool {
        true
    }

    fn sign(&self, _artifact: &Path, _ctx: &SignContext) -> Result<SignOutcome> {
        Ok(SignOutcome::Embedded)
    }
}
