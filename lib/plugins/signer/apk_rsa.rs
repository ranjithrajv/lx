// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;

use super::{SignContext, SignOutcome, Signer};

/// Alpine apk v2 signing: a DER PKCS#1 v1.5 RSA/SHA-1 signature over the
/// package *control* segment, written as a `.SIGN.RSA.<keyname>` tar member.
///
/// The signature is written while the packager assembles the apk (the
/// control segment must be signed as a unit and prepended), and the apk
/// indexer does the equivalent for `APKINDEX.tar.gz`. So this backend
/// declares itself embedded; `build.rs` runs no post-build step. The actual
/// RSA work lives in [`lx_lib::sign::rsa_sha1_sign`].
pub struct ApkRsa;

impl Signer for ApkRsa {
    fn name(&self) -> &'static str {
        "apk-rsa"
    }

    fn description(&self) -> &'static str {
        "Alpine apk v2 RSA/SHA-1 signature (`.SIGN.RSA.<keyname>`, in-process)"
    }

    fn supports(&self, format: &str, _method: &str) -> bool {
        format.eq_ignore_ascii_case("apk")
    }

    fn embedded(&self) -> bool {
        true
    }

    fn sign(&self, _artifact: &Path, _ctx: &SignContext) -> Result<SignOutcome> {
        Ok(SignOutcome::Embedded)
    }
}
