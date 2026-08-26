use lpt_lib::plugins::source::gerrit::*;

#[test]
fn parses_gerrit_url() {
    assert_eq!(
        parse_gerrit_url("https://review.gerrithub.io/a/platform/build").as_deref(),
        Some("platform/build")
    );
    assert_eq!(
        parse_gerrit_url("https://review.gerrithub.io/platform/build").as_deref(),
        Some("platform/build")
    );
    assert_eq!(
        parse_gerrit_url("https://gerrit.example.com/a/foo/bar/baz").as_deref(),
        Some("foo/bar/baz")
    );
}

#[test]
fn rejects_non_gerrit() {
    assert!(parse_gerrit_url("https://github.com/owner/repo").is_none());
    assert!(parse_gerrit_url("package.yaml").is_none());
}
