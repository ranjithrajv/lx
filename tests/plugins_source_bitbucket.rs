use lx_lib::plugins::source::bitbucket::*;

#[test]
fn parses_bitbucket_url() {
    assert_eq!(
        parse_bitbucket_url("https://bitbucket.org/atlassian/atlascode").as_deref(),
        Some("atlassian/atlascode")
    );
    assert_eq!(
        parse_bitbucket_url("https://bitbucket.org/team/repo.git").as_deref(),
        Some("team/repo")
    );
    assert_eq!(
        parse_bitbucket_url("https://bitbucket.org/team/repo/downloads").as_deref(),
        Some("team/repo")
    );
    assert_eq!(
        parse_bitbucket_url("https://api.bitbucket.org/2.0/repositories/team/repo").as_deref(),
        Some("team/repo")
    );
}

#[test]
fn rejects_non_bitbucket() {
    assert!(parse_bitbucket_url("https://github.com/owner/repo").is_none());
    assert!(parse_bitbucket_url("https://gitlab.com/owner/repo").is_none());
    assert!(parse_bitbucket_url("package.yaml").is_none());
}
