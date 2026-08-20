//! Shared-library dependency scanning: read an ELF binary's `DT_NEEDED`
//! entries (the same information `ldd`/`readelf -d` report), natively via
//! the `object` crate -- no `ldd`/`objdump`/`readelf` host-tool dependency,
//! consistent with the rest of this project's Docker/host-tool-free
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_elf_input() {
        let err = needed_libraries(b"not an elf file").unwrap_err();
        assert!(err.to_string().contains("ELF/object file"));
    }

    #[test]
    fn essential_libc_sonames_are_recognized() {
        assert!(is_essential_libc_soname("libc.so.6"));
        assert!(is_essential_libc_soname("ld-linux-x86-64.so.2"));
        assert!(!is_essential_libc_soname("libssl.so.3"));
    }

    /// A real, host-provided dynamically-linked binary must report at
    /// least libc among its needed libraries. Skips gracefully on hosts
    /// without a dynamically-linked /bin/ls (e.g. a fully static distro),
    /// mirroring the `real_dpkg_deb_accepts_the_archive` skip pattern in
    /// `debarchive.rs`.
    #[test]
    fn real_dynamically_linked_binary_reports_libc() {
        let Ok(bytes) = std::fs::read("/bin/ls") else {
            eprintln!("skipping: /bin/ls not found");
            return;
        };
        let libs = needed_libraries(&bytes).unwrap();
        if libs.is_empty() {
            eprintln!("skipping: /bin/ls appears to be statically linked");
            return;
        }
        assert!(
            libs.iter().any(|l| l.starts_with("libc.so")),
            "expected libc among {libs:?}"
        );
    }
}
