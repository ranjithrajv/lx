// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use super::{SignContext, SignOutcome, Signer};
use crate::plugins::plugin::plugin_identity;

/// Detached OpenPGP signature via `gpg --detach-sign`, written as a sibling
/// `<artifact>.sig`. Works for deb, rpm, arch, apk, and ipk alike.
///
/// It is **not** offered for `msix`/`osxpkg`: those formats have native
/// signing schemes (`AppxSignature.p7x`, the xar `Signature` member) that a
/// detached OpenPGP `.sig` does not satisfy. Emitting one would look like a
/// signature while being meaningless to `signtool`/`installer`, so those
/// formats are left to fail closed with "no signer supports format …".
pub struct GpgDetach;

impl Signer for GpgDetach {
    fn supports(&self, format: &str, method: &str) -> bool {
        if !method.eq_ignore_ascii_case("detach") {
            return false;
        }
        !matches!(
            format.to_ascii_lowercase().as_str(),
            "msix" | "osxpkg" | "pkg"
        )
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
