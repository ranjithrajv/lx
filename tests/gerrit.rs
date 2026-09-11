// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::gerrit::*;

#[test]
fn maps_tag_to_release() {
    let tag = GerritTagInfo {
        ref_tag: "refs/tags/v1.0.0".into(),
        revision: "abc123".into(),
        object: None,
        tag: Some("v1.0.0".into()),
        created: Some("2025-01-01T00:00:00Z".into()),
        tagger_date: None,
    };
    let r = GerritClient::map_tag_to_release(tag, "a/b", "review.gerrithub.io");
    assert_eq!(r.tag_name, "v1.0.0");
    assert_eq!(r.assets.len(), 1);
    assert!(r.assets[0].name.contains("v1.0.0"));
}

#[test]
fn project_encode_encodes_slash() {
    assert_eq!(GerritClient::project_encode("a/b"), "a%2Fb");
    assert_eq!(GerritClient::project_encode("a/b/c"), "a%2Fb%2Fc");
}
