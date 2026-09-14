// SPDX-License-Identifier: GPL-3.0-or-later

//! Native macOS flat-package (`.pkg`) signing.
//!
//! A flat package is a xar archive; native signing adds a `Signature` member
//! over the archive checksum, embedding the X.509 certificate chain. This
//! backend rewrites the built package in place using `apple-xar`'s
//! `XarSigner` (the same primitive `rcodesign`/`productsign` implement).
//!
//! Inputs: `signature.key_file` (PKCS#8 RSA/EC private key, PEM) and
//! `signature.cert_file` (X.509 certificate, PEM).

use anyhow::{bail, Context, Result};
use std::path::Path;

use apple_xar::reader::XarReader;
use apple_xar::signing::XarSigner;
use x509_certificate::{CapturedX509Certificate, InMemorySigningKeyPair};

use super::{SignContext, SignOutcome, Signer};
use crate::plugins::plugin::plugin_identity;

pub struct PkgXar;

impl Signer for PkgXar {
    fn supports(&self, format: &str, _method: &str) -> bool {
        crate::config::canonical_format(format) == "osxpkg"
    }

    fn sign(&self, artifact: &Path, ctx: &SignContext) -> Result<SignOutcome> {
        let cert_path = ctx.cert_file.trim();
        if cert_path.is_empty() {
            bail!("osxpkg signing requires signature.cert_file (X.509 certificate, PEM)");
        }

        let key_pem = std::fs::read(ctx.key_file)
            .with_context(|| format!("reading signing key '{}'", ctx.key_file.display()))?;
        let key = InMemorySigningKeyPair::from_pkcs8_pem(key_pem)
            .context("parsing PKCS#8 signing key (need an RSA/EC PKCS#8 PEM)")?;
        let cert_pem = std::fs::read(cert_path)
            .with_context(|| format!("reading signing cert '{cert_path}'"))?;
        let cert =
            CapturedX509Certificate::from_pem(cert_pem).context("parsing X.509 certificate")?;

        // Sign to a sibling temp file (signing in place can corrupt the xar),
        // then replace the artifact.
        let tmp = artifact.with_extension("pkg.tmp");
        {
            let reader = XarReader::new(std::fs::File::open(artifact)?)
                .with_context(|| format!("reading xar '{}'", artifact.display()))?;
            let mut signer = XarSigner::new(reader);
            let mut out = std::fs::File::create(&tmp)
                .with_context(|| format!("creating '{}'", tmp.display()))?;
            signer
                .sign(
                    &mut out,
                    &key,
                    &cert,
                    None,
                    std::iter::empty::<CapturedX509Certificate>(),
                )
                .context("signing xar (pkg)")?;
        }
        std::fs::rename(&tmp, artifact).with_context(|| {
            format!("replacing '{}' with the signed package", artifact.display())
        })?;
        Ok(SignOutcome::Embedded)
    }
}

plugin_identity!(
    PkgXar,
    "pkg-xar",
    "Native macOS flat-package (xar) signature via apple-xar"
);
