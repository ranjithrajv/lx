use lpt_lib::gitlab::*;

#[test]
fn maps_gitlab_release_to_common() {
    let raw = GitlabReleaseRaw {
        tag_name: "v1.0.0".into(),
        name: Some("Release v1.0.0".into()),
        description: None,
        created_at: Some("2025-01-01T00:00:00.000Z".into()),
        released_at: Some("2025-01-01T00:00:00.000Z".into()),
        upcoming_release: None,
        assets: GitlabAssets {
            count: 1,
            sources: vec![],
            links: vec![GitlabLink {
                name: "tool_x86_64.tar.gz".into(),
                url: "https://gitlab.com/a/b/-/releases/v1.0.0/downloads/tool.tar.gz".into(),
                direct_asset_url: Some(
                    "https://gitlab.com/a/b/-/releases/v1.0.0/downloads/tool.tar.gz".into(),
                ),
                link_type: None,
            }],
        },
    };
    let r = GitlabClient::map_release(raw, "a/b");
    assert_eq!(r.tag_name, "v1.0.0");
    assert_eq!(r.assets.len(), 1);
    assert_eq!(r.assets[0].name, "tool_x86_64.tar.gz");
    assert_eq!(r.published_at, Some(1_735_689_600));
}

#[test]
fn project_encode_encodes_slash() {
    assert_eq!(GitlabClient::project_encode("a/b"), "a%2Fb");
    assert_eq!(GitlabClient::project_encode("a/b/c"), "a%2Fb%2Fc");
}

#[test]
fn gitlab_time_parsing() {
    assert_eq!(
        parse_gitlab_time("2025-01-01T00:00:00.000Z"),
        Some(1_735_689_600)
    );
    assert_eq!(
        parse_gitlab_time("2025-01-01T00:00:00Z"),
        Some(1_735_689_600)
    );
    assert_eq!(parse_gitlab_time("bad"), None);
}
