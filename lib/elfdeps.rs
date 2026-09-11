//! Shared-library dependency scanning: read an ELF binary's `DT_NEEDED`
//! entries (the same information `ldd`/`readelf -d` report), natively via
//! the `object` crate -- no `ldd`/`objdump`/`readelf` host-tool dependency,
//! consistent with the rest of this project's subprocess-free
//! packaging pipeline.

use anyhow::{Context, Result};
use object::read::Object;

/// The shared libraries a binary needs at runtime (`DT_NEEDED` entries, in
/// the order they appear in the ELF dynamic section). Empty for a
/// statically linked binary.
pub fn needed_libraries(bytes: &[u8]) -> Result<Vec<String>> {
    let file = object::File::parse(bytes).context("failed to parse as an ELF/object file")?;
    let mut names = Vec::new();
    for lib in file
        .import_libraries()
        .context("failed to read dynamic library dependencies")?
    {
        let lib = lib.context("failed to read a dynamic library dependency entry")?;
        names.push(String::from_utf8_lossy(lib.name()).into_owned());
    }
    Ok(names)
}

/// Sonames that come from glibc itself (Debian's `libc6`), which is
/// `Priority: required` -- always present on any Debian system. Debian
/// policy (and every `dpkg-shlibdeps`-generated `Depends:` line) omits
/// these rather than listing them explicitly, so callers should treat them
/// as informational, not as something missing from `depends:`.
const ESSENTIAL_LIBC_SONAMES: &[&str] = &[
    "libc.so.6",
    "libm.so.6",
    "libdl.so.2",
    "libpthread.so.0",
    "librt.so.1",
    "libresolv.so.2",
    "libutil.so.1",
    "libanl.so.1",
    "ld-linux-x86-64.so.2",
    "ld-linux-aarch64.so.1",
    "ld-linux-armhf.so.3",
    "ld-linux.so.2",
];

/// True if `soname` is provided by glibc/`libc6` and can usually be
/// omitted from `depends:`.
pub fn is_essential_libc_soname(soname: &str) -> bool {
    ESSENTIAL_LIBC_SONAMES.contains(&soname)
}
