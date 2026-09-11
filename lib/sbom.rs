//! Supply-chain attestations: SPDX SBOM + SLSA provenance per build.
//!
//! Nobody in the deb-get/makedeb/multiarch-builder space emits these; lx
//! already records per-asset provenance, so the marginal cost is one JSON
//! write. Gated on `lx build --sbom`: writes `<pkg>_<ver>.spdx.json`
//! (SPDX 2.3, packages = built artifacts + upstream materials) and
//! `<pkg>_<ver>.slsa.json` (SLSA v1-style provenance: builder identity,
//! materials with digests, outputs with digests).

use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

/// One built artifact with its digest.
pub struct Artifact {
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}

/// One upstream input with its digest (empty digest = unverified).
pub struct Material {
    pub uri: String,
    pub digest: Option<String>,
}

/// Collect `.deb`/`.rpm`/`.dsc` artifacts in `out_dir` with SHA-256.
pub fn collect_artifacts(out_dir: &Path) -> Result<Vec<Artifact>> {
    let mut out = Vec::new();
    let entries =
        std::fs::read_dir(out_dir).with_context(|| format!("reading '{}'", out_dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let is_artifact = path
            .extension()
            .map(|e| matches!(e.to_string_lossy().as_ref(), "deb" | "rpm" | "dsc"))
            .unwrap_or(false);
        if !is_artifact || !entry.file_type()?.is_file() {
            continue;
        }
        let sha256 = lx_lib::checksum::sha256_file(&path)?;
        let size = std::fs::metadata(&path)?.len();
        out.push(Artifact {
            filename: path.file_name().unwrap().to_string_lossy().to_string(),
            sha256,
            size,
        });
    }
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(out)
}

/// Write `<pkg>_<ver>.spdx.json` + `<pkg>_<ver>.slsa.json` into `out_dir`.
pub fn emit(
    out_dir: &Path,
    package: &str,
    version: &str,
    build_version: &str,
    artifacts: &[Artifact],
    materials: &[Material],
) -> Result<Vec<PathBuf>> {
    let stem = format!("{package}_{version}-{build_version}");
    let now = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let mut spdx_packages = vec![json!({
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": stem,
        "supplier": "Organization: latest-debs",
        "downloadLocation": "NOASSERTION",
        "filesAnalyzed": false,
        "verificationCode": {"packageVerificationCodeValue": artifacts_digest(artifacts)},
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": "NOASSERTION",
        "copyrightText": "NOASSERTION",
    })];
    for a in artifacts {
        spdx_packages.push(json!({
            "SPDXID": format!("SPDXRef-Artifact-{}", sanitize(&a.filename)),
            "name": a.filename,
            "supplier": "Organization: latest-debs",
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": false,
            "checksums": [{"algorithm": "SHA256", "checksumValue": a.sha256}],
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION",
            "copyrightText": "NOASSERTION",
        }));
    }
    for m in materials {
        spdx_packages.push(json!({
            "SPDXID": format!("SPDXRef-Material-{}", sanitize(&m.uri)),
            "name": m.uri,
            "supplier": "NOASSERTION",
            "downloadLocation": m.uri,
            "filesAnalyzed": false,
            "checksums": m.digest.as_ref().map(|d| vec![json!({"algorithm": "SHA256", "checksumValue": d})]).unwrap_or_default(),
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION",
            "copyrightText": "NOASSERTION",
        }));
    }
    let spdx = json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": stem,
        "documentNamespace": format!("https://latest-debs.org/sbom/{stem}"),
        "creationInfo": {"created": now, "creators": ["Tool: lx"]},
        "packages": spdx_packages,
    });
    let spdx_path = out_dir.join(format!("{stem}.spdx.json"));
    std::fs::write(&spdx_path, serde_json::to_string_pretty(&spdx)?)?;

    let slsa = json!({
        "_type": "https://in-toto.io/Statement/v1",
        "predicateType": "https://slsa.dev/provenance/v1",
        "subject": artifacts.iter().map(|a| json!({
            "name": a.filename,
            "digest": {"sha256": a.sha256},
        })).collect::<Vec<_>>(),
        "predicate": {
            "buildDefinition": {
                "buildType": "https://latest-debs.org/lx/build/v1",
                "resolvedDependencies": materials.iter().map(|m| json!({
                    "uri": m.uri,
                    "digest": m.digest.as_ref().map(|d| json!({"sha256": d})).unwrap_or(json!({})),
                })).collect::<Vec<_>>(),
            },
            "runDetails": {
                "builder": {"id": "https://latest-debs.org/lx/v1"},
                "metadata": {"finishedOn": now},
            },
        },
    });
    let slsa_path = out_dir.join(format!("{stem}.slsa.json"));
    std::fs::write(&slsa_path, serde_json::to_string_pretty(&slsa)?)?;

    println!("  sbom: {} + {}", spdx_path.display(), slsa_path.display());
    Ok(vec![spdx_path, slsa_path])
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Document-level verification code: sha256 over the sorted artifact digests.
fn artifacts_digest(artifacts: &[Artifact]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    let mut digests: Vec<&str> = artifacts.iter().map(|a| a.sha256.as_str()).collect();
    digests.sort_unstable();
    for d in digests {
        h.update(d.as_bytes());
    }
    hex::encode(h.finalize())
}
