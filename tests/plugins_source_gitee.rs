// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::gitee::{GiteeAssetRaw, GiteeClient, GiteeReleaseRaw};
use lx_lib::plugins::forge::gitee::parse_gitee_url;

#[test]
fn parses_gitee_url() {
    assert_eq!(
        parse_gitee_url("https://gitee.com/owner/repo").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(
        parse_gitee_url("https://gitee.com/owner/repo.git").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(
        parse_gitee_url("https://gitee.com/owner/repo/releases/tag/v1").as_deref(),
        Some("owner/repo")
    );
}

#[test]
fn rejects_non_gitee() {
    assert!(parse_gitee_url("https://github.com/owner/repo").is_none());
    assert!(parse_gitee_url("https://gitlab.com/owner/repo").is_none());
    assert!(parse_gitee_url("package.yaml").is_none());
}

#[test]
fn maps_release_assets_and_timestamp() {
    let raw = GiteeReleaseRaw {
        tag_name: "v1.2.3".into(),
        name: Some("1.2.3".into()),
        prerelease: false,
        created_at: Some("2020-03-27T21:13:11+08:00".into()),
        body: None,
        assets: vec![GiteeAssetRaw {
            name: "app-x86_64.tar.gz".into(),
            browser_download_url:
                "https://gitee.com/owner/repo/releases/download/v1.2.3/app-x86_64.tar.gz".into(),
            size: Some(123),
        }],
        attach_files: vec![],
    };
    let rel = GiteeClient::map_release(raw, "owner", "repo");
    assert_eq!(rel.tag_name, "v1.2.3");
    assert_eq!(rel.assets.len(), 1);
    assert_eq!(rel.assets[0].name, "app-x86_64.tar.gz");
    assert_eq!(rel.assets[0].size, Some(123));
    assert_eq!(
        rel.html_url,
        "https://gitee.com/owner/repo/releases/tag/v1.2.3"
    );
    assert!(rel.published_at.is_some(), "created_at should parse");
}

#[test]
fn falls_back_to_attach_files_and_skips_empty_urls() {
    let raw = GiteeReleaseRaw {
        tag_name: "v2".into(),
        name: None,
        prerelease: true,
        created_at: None,
        body: None,
        assets: vec![],
        attach_files: vec![
            GiteeAssetRaw {
                name: "ok.tar.gz".into(),
                browser_download_url: "https://gitee.com/o/r/download/ok.tar.gz".into(),
                size: None,
            },
            GiteeAssetRaw {
                name: "no-url".into(),
                browser_download_url: String::new(),
                size: None,
            },
        ],
    };
    let rel = GiteeClient::map_release(raw, "o", "r");
    assert_eq!(rel.assets.len(), 1);
    assert_eq!(rel.assets[0].name, "ok.tar.gz");
    assert!(rel.prerelease);
}
