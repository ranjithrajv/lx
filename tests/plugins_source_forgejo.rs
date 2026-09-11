use lx_lib::plugins::source::forgejo::*;

#[test]
fn parses_forgejo_url() {
    assert_eq!(
        parse_forgejo_url("https://codeberg.org/owner/repo").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(
        parse_forgejo_url("https://codeberg.org/owner/repo.git").as_deref(),
        Some("owner/repo")
    );
}

#[test]
fn rejects_non_forgejo() {
    assert!(parse_forgejo_url("https://github.com/owner/repo").is_none());
    assert!(parse_forgejo_url("package.yaml").is_none());
}
