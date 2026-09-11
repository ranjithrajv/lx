// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::source::gitea::*;

#[test]
fn parses_gitea_url() {
    assert_eq!(
        parse_gitea_url("https://codeberg.org/owner/repo").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(
        parse_gitea_url("https://codeberg.org/owner/repo.git").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(
        parse_gitea_url("https://codeberg.org/owner/repo/releases/tag/v1").as_deref(),
        Some("owner/repo")
    );
}

#[test]
fn rejects_non_gitea() {
    assert!(parse_gitea_url("https://github.com/owner/repo").is_none());
    assert!(parse_gitea_url("https://gitlab.com/owner/repo").is_none());
    assert!(parse_gitea_url("package.yaml").is_none());
}
