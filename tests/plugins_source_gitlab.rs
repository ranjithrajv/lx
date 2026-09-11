// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::source::gitlab::*;

#[test]
fn parses_gitlab_url() {
    assert_eq!(
        parse_gitlab_url("https://gitlab.com/gitlab-org/gitlab").as_deref(),
        Some("gitlab-org/gitlab")
    );
    assert_eq!(
        parse_gitlab_url("https://gitlab.com/eza-community/eza.git").as_deref(),
        Some("eza-community/eza")
    );
    assert_eq!(
        parse_gitlab_url("https://gitlab.com/owner/repo/").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(
        parse_gitlab_url("https://gitlab.com/owner/repo/releases/tag/v1").as_deref(),
        Some("owner/repo")
    );
}

#[test]
fn rejects_non_gitlab() {
    assert!(parse_gitlab_url("https://github.com/owner/repo").is_none());
    assert!(parse_gitlab_url("package.yaml").is_none());
    assert!(parse_gitlab_url("https://gitlab.com/owner-only").is_none());
}
