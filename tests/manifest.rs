// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::manifest::*;

#[test]
fn round_trips_through_json() {
    let mut m = Manifest::default();
    m.record(
        "eza",
        PackageEntry {
            version: "0.24.0-1+trixie".into(),
            arch: "amd64".into(),
            distribution: "trixie".into(),
            asset: "eza_0.24.0-1+trixie_amd64.deb".into(),
            tag: "v0.24.0".into(),
            installed_at: "2026-08-20T00:00:00Z".into(),
            format: "deb".into(),
        },
    );
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.current("eza").unwrap().version, "0.24.0-1+trixie");
}

#[test]
fn forget_removes_entry() {
    let mut m = Manifest::default();
    m.record(
        "eza",
        PackageEntry {
            version: "1".into(),
            arch: "amd64".into(),
            distribution: "trixie".into(),
            asset: "a".into(),
            tag: "v1".into(),
            installed_at: "t".into(),
            format: "deb".into(),
        },
    );
    assert!(m.forget("eza").is_some());
    assert!(m.packages.is_empty());
    assert!(m.forget("eza").is_none());
}

#[test]
fn generations_accumulate_and_previous_walks_history() {
    let mut m = Manifest::default();
    for v in ["1", "2", "3"] {
        m.record(
            "eza",
            PackageEntry {
                version: v.into(),
                arch: "amd64".into(),
                distribution: "trixie".into(),
                asset: format!("eza_{v}_amd64.deb"),
                tag: format!("v{v}"),
                installed_at: "t".into(),
                format: "deb".into(),
            },
        );
    }
    assert_eq!(m.current("eza").unwrap().version, "3");
    assert_eq!(m.previous("eza", 1).unwrap().version, "2");
    assert_eq!(m.previous("eza", 2).unwrap().version, "1");
    assert!(m.previous("eza", 3).is_none());
    assert_eq!(m.packages["eza"].len(), 3);
}

#[test]
fn manifest_without_format_defaults_to_deb() {
    // Entries written before the `format` field existed must keep loading.
    let json = r#"{"packages":{"eza":[{"version":"1","arch":"amd64",
        "distribution":"trixie","asset":"a","tag":"v1","installed_at":"t"}]}}"#;
    let m: Manifest = serde_json::from_str(json).unwrap();
    assert_eq!(m.current("eza").unwrap().format, "deb");
}
