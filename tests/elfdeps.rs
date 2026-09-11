// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::elfdeps::*;

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
