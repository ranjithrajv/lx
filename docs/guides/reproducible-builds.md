# Reproducible builds

Part of the [lx docs](../README.md).

Builds are reproducible per
[Debian's definition](https://wiki.debian.org/ReproducibleBuilds): building
the same input twice, at different times, produces byte-identical output —
verified empirically (see
[`docs/decisions/2026-08-20-reproducible-builds.md`](../decisions/2026-08-20-reproducible-builds.md)
for the investigation). Package
metadata timestamps (changelog date, copyright year) come from the GitHub
release's own publish time rather than build time, and the source-package
`.orig.tar.xz`/`.debian.tar.xz` members are built (`lib/debarchive.rs`)
walking their contents in sorted order with normalized mtime/owner/group.
Both respect the standard `SOURCE_DATE_EPOCH` environment variable if you
want to pin an exact value.

