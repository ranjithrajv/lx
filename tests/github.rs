// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::github::*;

#[test]
fn maps_raw_release_to_common() {
    let raw = GitHubReleaseRaw {
        tag_name: "v0.24.0".into(),
        prerelease: false,
        draft: false,
        html_url: "https://github.com/eza-community/eza/releases/tag/v0.24.0".into(),
        published_at: Some("2025-01-01T00:00:00Z".into()),
        assets: vec![GitHubAssetRaw {
            name: "eza_x86_64-unknown-linux-gnu.tar.gz".into(),
            size: Some(123),
            browser_download_url: "https://github.com/.../eza_x86_64-unknown-linux-gnu.tar.gz"
                .into(),
            digest: Some("sha256:deadbeef".into()),
        }],
        body: None,
    };
    let r: Release = raw.into();
    assert_eq!(r.tag_name, "v0.24.0");
    assert!(!r.prerelease && !r.draft);
    assert_eq!(r.assets.len(), 1);
    assert_eq!(r.assets[0].name, "eza_x86_64-unknown-linux-gnu.tar.gz");
    assert_eq!(r.assets[0].size, Some(123));
    assert_eq!(r.published_at, Some(1_735_689_600));
    assert_eq!(
        r.assets[0].checksums.get("sha256").map(String::as_str),
        Some("deadbeef")
    );
}

#[test]
fn decodes_base64_content() {
    // "MIT License" base64-encoded, as GitHub's contents API returns it.
    assert_eq!(
        decode_content(Some("TUlUIExpY2Vuc2U="), Some("base64")),
        Some("MIT License".to_string())
    );
    assert_eq!(
        decode_content(Some("TUlUIExpY2Vuc2U="), Some("other")),
        None
    );
    assert_eq!(decode_content(None, Some("base64")), None);
}
