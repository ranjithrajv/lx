use lpt_lib::plugins::source::*;

#[test]
fn registry_contains_github_and_gitlab() {
    let names = source_available_names();
    assert!(names.contains(&"github"));
    assert!(names.contains(&"gitlab"));
    assert!(names.contains(&"gitea"));
    assert!(names.contains(&"forgejo"));
    assert!(names.contains(&"bitbucket"));
    assert!(names.contains(&"gerrit"));
    assert!(get_source_plugin("github").is_some());
    assert!(get_source_plugin("gitlab").is_some());
    assert!(get_source_plugin("gerrit").is_some());
    assert!(get_source_plugin("unknown").is_none());
    assert!(get_source_plugin("GitHub").is_some());
    assert!(get_source_plugin("GERRIT").is_some());
}

#[test]
fn parse_any_url_dispatches_by_host() {
    assert_eq!(
        parse_any_url("https://github.com/eza-community/eza").unwrap(),
        ("github".to_string(), "eza-community/eza".to_string())
    );
    assert_eq!(
        parse_any_url("https://gitlab.com/gitlab-org/gitlab").unwrap(),
        ("gitlab".to_string(), "gitlab-org/gitlab".to_string())
    );
    assert!(parse_any_url("package.yaml").is_none());
}
