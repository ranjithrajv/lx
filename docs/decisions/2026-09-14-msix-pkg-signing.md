# Native MSIX (`.p7x`) and macOS pkg (xar) signing

## Context

Earlier in this cycle `lx` could build `.msix` and `.pkg` artifacts but not
sign them natively, and the generic post-build signer would (wrongly) attach
a detached OpenPGP `.sig` to them. The formats have native signature schemes
that a `.sig` does not satisfy:

- **MSIX** requires an `AppxSignature.p7x` member: `0x504b4358` magic + DER
  `ContentInfo(SignedData)` whose eContent is a Microsoft "SIP indirect
  data" structure carrying five SHA-256 digests (`AXPC` zip local bytes,
  `AXCD` central directory, `AXCT` content types, `AXBM` block map, optional
  `AXCI`), with signed attributes and an `IssuerAndSerialNumber` signer id.
  The signing certificate's Subject must match the manifest `Publisher`.
- **macOS flat package** requires a xar `Signature` member (an RSA signature
  over the archive's SHA-256 checksum) plus the checksum itself. This is the
  scheme `productsign` and `rcodesign` implement.

## Decision

Use established crates rather than hand-rolling ASN.1/CMS and xar signing:

- **MSIX**: the [`msix`](https://crates.io/crates/msix) crate
  (Apache-2.0 OR MIT) for the `p7x`/PKCS#7 layer, plus
  [`xcommon`](https://crates.io/crates/xcommon) for its `Signer`
  (PEM cert + PKCS#8 RSA key).
- **pkg**: [`apple-xar`](https://crates.io/crates/apple-xar) (MPL-2.0) for
  `XarReader`/`XarSigner`, plus
  [`x509-certificate`](https://crates.io/crates/x509-certificate) for
  loading the key/cert. `apple-xar` is the same primitive `rcodesign` uses.

`lx` also now writes the xar archive **checksum** (SHA-256 of the
compressed TOC) it previously omitted, so `apple-xar` can read the package
before signing. Note the xar TOC field semantics: `<length>` is the
*archived* size and `<size>` the *extracted* size (this tripped up the first
implementation).

Configuration: `signature.key_file` (PKCS#8 key, PEM) plus a new
`signature.cert_file` (X.509 certificate, PEM), for both formats.
`msix.signature.pfx_file` is not supported (PEM instead of `.pfx`).

### Deliberately not used: `apple-codesign`

`apple-codesign` exposes the same `UnifiedSigner::sign_xar` entry point but
depends on `walkdir`, which `deny.toml` bans (replaced by `ignore`).
`apple-xar` provides the signing primitive without that dependency.

## Accepted advisory: quick-xml 0.26 (via `msix`)

`msix 0.4.0` depends on `quick-xml 0.26`, which carries
RUSTSEC-2026-0194 / RUSTSEC-2026-0195 (NsReader DoS on untrusted XML).
`lx` uses `msix` **only** to serialize `AppxSignature.p7x` via `rasn`; it
never parses untrusted XML with quick-xml. The vulnerable path is not
reachable from any `lx` code path, so both advisories are accepted in
`deny.toml` alongside the existing `rsa` exception, with a comment to
revisit if `msix` bumps quick-xml or is replaced.

## Verification

- MSIX: round-trip through `msix::p7x::read_p7x` (the crate that produced
  the format decodes our signature as PKCS#7 `SignedData`).
- pkg: after signing, the xar TOC carries a `<signature>` with an embedded
  `X509Certificate`; `apple-xar` reads the signed archive.
- Neither can be validated against Windows `signtool` or macOS `installer`
  from this environment; that remains a follow-up for a platform runner.
