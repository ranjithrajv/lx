use lpt_lib::manifest::*;

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
        },
    );
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.packages["eza"].version, "0.24.0-1+trixie");
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
        },
    );
    assert!(m.forget("eza").is_some());
    assert!(m.packages.is_empty());
    assert!(m.forget("eza").is_none());
}
