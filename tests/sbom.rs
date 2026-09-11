// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::sbom::{collect_artifacts, emit, Artifact, Material};

fn unverified_material() -> Material {
    Material {
        uri: "https://example.invalid/src.tar.gz".to_string(),
        digest: None,
    }
}

#[test]
fn emit_writes_spdx_and_slsa() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pkg_1.0-1+bookworm_amd64.deb"), b"fake-deb").unwrap();
    let artifacts = collect_artifacts(dir.path()).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].filename, "pkg_1.0-1+bookworm_amd64.deb");
    assert!(!artifacts[0].sha256.is_empty());

    let materials = vec![Material {
        uri: "https://github.com/o/r/archive/refs/tags/v1.0.tar.gz".to_string(),
        digest: Some("abc123".to_string()),
    }];
    let paths = emit(dir.path(), "pkg", "1.0", "1", &artifacts, &materials).unwrap();
    assert_eq!(paths.len(), 2);

    let spdx: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&paths[0]).unwrap()).unwrap();
    assert_eq!(spdx["spdxVersion"], "SPDX-2.3");
    assert_eq!(spdx["packages"].as_array().unwrap().len(), 3); // doc + artifact + material

    let slsa: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&paths[1]).unwrap()).unwrap();
    assert_eq!(slsa["predicateType"], "https://slsa.dev/provenance/v1");
    assert_eq!(
        slsa["subject"][0]["digest"]["sha256"],
        serde_json::Value::from(artifacts[0].sha256.clone())
    );
}

#[test]
fn collect_ignores_non_artifacts_and_unverified_material_ok() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("build-summary.json"), b"{}").unwrap();
    std::fs::write(dir.path().join("pkg_1.0-1+trixie.dsc"), b"dsc").unwrap();
    let artifacts = collect_artifacts(dir.path()).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].filename.ends_with(".dsc"));

    // Unverified material serializes without a digest, still valid JSON.
    let single = Artifact {
        filename: "x.deb".to_string(),
        sha256: "00".to_string(),
        size: 1,
    };
    let paths = emit(
        dir.path(),
        "pkg",
        "1.0",
        "1",
        std::slice::from_ref(&single),
        std::slice::from_ref(&unverified_material()),
    )
    .unwrap();
    let slsa: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&paths[1]).unwrap()).unwrap();
    assert_eq!(
        slsa["predicate"]["buildDefinition"]["resolvedDependencies"][0]["uri"],
        "https://example.invalid/src.tar.gz"
    );
}
