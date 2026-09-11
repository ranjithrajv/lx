// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::forge::*;

#[test]
fn registry_contains_github_and_gitlab() {
    let names = forge_source_names();
    assert!(names.contains(&"github"));
    assert!(names.contains(&"gitlab"));
    assert!(names.contains(&"gitea"));
    assert!(names.contains(&"forgejo"));
    assert!(names.contains(&"bitbucket"));
    assert!(names.contains(&"gerrit"));
    assert!(get_forge_source("github").is_some());
    assert!(get_forge_source("gitlab").is_some());
    assert!(get_forge_source("gerrit").is_some());
    assert!(get_forge_source("unknown").is_none());
    assert!(get_forge_source("GitHub").is_some());
    assert!(get_forge_source("GERRIT").is_some());
}

#[test]
fn parse_any_forge_url_dispatches_by_host() {
    assert_eq!(
        parse_any_forge_url("https://github.com/eza-community/eza").unwrap(),
        ("github".to_string(), "eza-community/eza".to_string())
    );
    assert_eq!(
        parse_any_forge_url("https://gitlab.com/gitlab-org/gitlab").unwrap(),
        ("gitlab".to_string(), "gitlab-org/gitlab".to_string())
    );
    assert!(parse_any_forge_url("package.yaml").is_none());
}
