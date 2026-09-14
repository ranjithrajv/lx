// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::signer::{get_signer, signer_for, signer_names};

#[test]
fn registry_lists_all_signers() {
    let names = signer_names();
    for expected in ["gpg-detach", "rpm-pgp", "deb-debsign"] {
        assert!(names.contains(&expected), "missing signer {expected}");
        assert!(get_signer(expected).is_some());
    }
    assert!(get_signer("GPG-DETACH").is_some(), "case-insensitive");
    assert!(get_signer("bogus").is_none());
}

#[test]
fn selection_prefers_embedded_backends() {
    let deb = signer_for("deb", "debsign").unwrap();
    assert_eq!(deb.name(), "deb-debsign");
    assert!(deb.embedded(), "deb debsign is embedded by the packager");

    let rpm = signer_for("rpm", "detach").unwrap();
    assert_eq!(rpm.name(), "rpm-pgp");
    assert!(rpm.embedded(), "rpm PGP is embedded in the header");
}

#[test]
fn detached_signing_is_available_for_every_other_format() {
    for format in ["deb", "arch", "apk", "ipk"] {
        let signer = signer_for(format, "detach").unwrap();
        assert_eq!(signer.name(), "gpg-detach", "format {format}");
        assert!(!signer.embedded());
    }
}

#[test]
fn debsign_is_not_offered_for_non_deb() {
    assert!(signer_for("apk", "debsign").is_none());
}
