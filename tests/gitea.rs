use lx_lib::gitea::*;

#[test]
fn maps_gitea_release() {
    let raw = GiteaReleaseRaw {
        tag_name: "v1.0.0".into(),
        name: Some("v1.0.0".into()),
        prerelease: false,
        draft: false,
        html_url: Some("https://codeberg.org/a/b/releases/tag/v1.0.0".into()),
        created_at: Some("2025-01-01T00:00:00Z".into()),
        published_at: Some("2025-01-01T00:00:00Z".into()),
        assets: vec![GiteaAssetRaw {
            name: "tool.tar.gz".into(),
            browser_download_url: "https://codeberg.org/a/b/releases/download/v1.0.0/tool.tar.gz"
                .into(),
            size: Some(123),
        }],
        body: None,
    };
    let r = GiteaClient::map_release(raw, "a/b");
    assert_eq!(r.tag_name, "v1.0.0");
    assert_eq!(r.assets[0].name, "tool.tar.gz");
}
