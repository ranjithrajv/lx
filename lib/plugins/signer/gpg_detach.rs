// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use super::{SignContext, SignOutcome, Signer};
use crate::plugins::plugin::plugin_identity;

/// Detached OpenPGP signature via `gpg --detach-sign`, written as a sibling
/// `<artifact>.sig`. Format-agnostic: works for deb, rpm, arch, apk, and
/// ipk alike.
pub struct GpgDetach;

impl Signer for GpgDetach {
    fn supports(&self, _format: &str, method: &str) -> bool {
        method.eq_ignore_ascii_case("detach")
    }

    fn sign(&self, artifact: &Path, ctx: &SignContext) -> Result<SignOutcome> {
        let req = lx_lib::sign::SignRequest {
            key_file: ctx.key_file,
            key_id: ctx.key_id,
            passphrase: ctx.passphrase,
        };
        Ok(SignOutcome::Detached(lx_lib::sign::gpg_detach_sign(
            artifact, &req,
        )?))
    }
}

plugin_identity!(
    GpgDetach,
    "gpg-detach",
    "Detached OpenPGP signature (gpg --detach-sign → <artifact>.sig)"
);
