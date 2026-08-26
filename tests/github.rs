use lpt_lib::github::*;

#[test]
fn converts_octocrab_release() {
    let json = r#"{
            "url": "https://api.github.com/repos/eza-community/eza/releases/1",
            "html_url": "https://github.com/eza-community/eza/releases/tag/v0.24.0",
            "assets_url": "https://api.github.com/repos/eza-community/eza/releases/1/assets",
            "upload_url": "https://uploads.github.com/...",
            "tarball_url": null,
            "zipball_url": null,
            "id": 1,
            "node_id": "RE_1",
            "tag_name": "v0.24.0",
            "target_commitish": "main",
            "name": "eza v0.24.0",
            "body": null,
            "draft": false,
            "prerelease": false,
            "created_at": "2025-01-01T00:00:00Z",
            "published_at": "2025-01-01T00:00:00Z",
            "assets": [
                {
                    "url": "https://api.github.com/.../assets/10",
                    "browser_download_url": "https://github.com/.../eza_x86_64-unknown-linux-gnu.tar.gz",
                    "id": 10,
                    "node_id": "RA_10",
                    "name": "eza_x86_64-unknown-linux-gnu.tar.gz",
                    "label": null,
                    "state": "uploaded",
                    "content_type": "application/gzip",
                    "size": 123,
                    "download_count": 5,
                    "created_at": "2025-01-01T00:00:00Z",
                    "updated_at": "2025-01-01T00:00:00Z"
                }
            ]
        }"#;
    let octo: octocrab::models::repos::Release = serde_json::from_str(json).unwrap();
    let r: Release = octo.into();
    assert_eq!(r.tag_name, "v0.24.0");
    assert_eq!(r.assets.len(), 1);
    assert_eq!(r.assets[0].name, "eza_x86_64-unknown-linux-gnu.tar.gz");
    assert_eq!(r.assets[0].size, Some(123));
    assert_eq!(r.published_at, Some(1_735_689_600)); // 2025-01-01T00:00:00Z
    assert!(!r.prerelease);
    assert!(!r.draft);
}

#[test]
fn release_from_octocrab_maps_prerelease_and_draft() {
    let json = r#"{
            "url": "https://api.github.com/repos/eza-community/eza/releases/2",
            "html_url": "https://github.com/eza-community/eza/releases/tag/v0.25.0-rc1",
            "assets_url": "https://api.github.com/repos/eza-community/eza/releases/2/assets",
            "upload_url": "https://uploads.github.com/...",
            "tarball_url": null,
            "zipball_url": null,
            "id": 2,
            "node_id": "RE_2",
            "tag_name": "v0.25.0-rc1",
            "target_commitish": "main",
            "name": "eza v0.25.0-rc1",
            "body": null,
            "draft": true,
            "prerelease": true,
            "created_at": "2025-01-01T00:00:00Z",
            "published_at": null,
            "assets": []
        }"#;
    let octo: octocrab::models::repos::Release = serde_json::from_str(json).unwrap();
    let r: Release = octo.into();
    assert!(r.prerelease);
    assert!(r.draft);
}
