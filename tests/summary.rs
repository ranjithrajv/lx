use lx_lib::summary::*;
use std::time::Instant;

#[test]
fn badge_encode_percent_encodes_spaces_and_pipes() {
    assert_eq!(
        badge_encode(&["bookworm".into(), "trixie".into(), "sid".into()]),
        "bookworm%20%7C%20trixie%20%7C%20sid"
    );
    assert_eq!(badge_encode(&["amd64".into()]), "amd64");
}

#[test]
fn glob_match_basic() {
    let pattern = glob::Pattern::new("eza_*.deb").unwrap();
    assert!(pattern.matches("eza_0.23.5-1+bookworm_amd64.deb"));
    assert!(!pattern.matches("eza_0.23.5-1+bookworm_amd64.dsc"));
    assert!(!pattern.matches("ezaa_0.1.deb"));
}

#[test]
fn summary_lists_built_debs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::write(path.join("eza_1.0-1+bookworm_amd64.deb"), vec![0u8; 2048]).unwrap();
    std::fs::write(path.join("eza_1.0-1+bookworm_arm64.deb"), vec![0u8; 4096]).unwrap();
    std::fs::write(path.join("eza_1.0-1+bookworm.dsc"), "x").unwrap();

    let inputs = SummaryInputs {
        package: "eza".into(),
        version: "v1.0".into(),
        build_version: "1".into(),
        github_repo: "eza-community/eza".into(),
        architectures: vec!["amd64".into(), "arm64".into()],
        distributions: vec!["bookworm".into()],
        max_parallel: 2,
        start: Instant::now(),
        telemetry: serde_json::json!({ "build_duration_seconds": 7 }),
        provenance: vec![],
        package_format: "deb".into(),
        source: "gitlab".into(),
    };
    write(path, 2, &inputs).unwrap();

    let text = std::fs::read_to_string(path.join(lx_lib::constants::SUMMARY_FILENAME)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["total_packages"], 2);
    assert_eq!(v["total_size_bytes"], 6144);
    assert_eq!(v["total_size_human"], "6 KB");
    assert_eq!(v["success_rate"], 100);
    assert_eq!(v["package"], "eza");
    assert_eq!(v["full_version"], "v1.0-1");
    assert_eq!(v["packages"].as_array().unwrap().len(), 2);
    assert_eq!(v["telemetry"]["build_duration_seconds"], 7);
    assert_eq!(v["source"], "gitlab");
}

#[test]
fn provenance_is_embedded_and_unverified_assets_are_counted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::write(path.join("eza_1.0-1+bookworm_amd64.deb"), b"x").unwrap();

    let inputs = SummaryInputs {
        package: "eza".into(),
        version: "v1.0".into(),
        build_version: "1".into(),
        github_repo: "eza-community/eza".into(),
        architectures: vec!["amd64".into()],
        distributions: vec!["bookworm".into()],
        max_parallel: 1,
        start: Instant::now(),
        telemetry: serde_json::json!({}),
        provenance: vec![
            serde_json::json!({"asset": "a", "method": "pinned", "sha256": "aa"}),
            serde_json::json!({"asset": "b", "method": "sidecar", "sha256": "bb"}),
            serde_json::json!({
                "asset": "c",
                "method": "unverified (--allow-unverified)",
                "sha256": "cc"
            }),
        ],
        package_format: "deb".into(),
        source: "github".into(),
    };
    write(path, 1, &inputs).unwrap();

    let text = std::fs::read_to_string(path.join(lx_lib::constants::SUMMARY_FILENAME)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["provenance"].as_array().unwrap().len(), 3);
    assert_eq!(v["provenance"][0]["asset"], "a");
    // Only the pinned and sidecar entries count as verified.
    assert_eq!(v["unverified_assets"], 1);
}
