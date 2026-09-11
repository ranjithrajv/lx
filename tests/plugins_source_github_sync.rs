// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::forge::{get_forge_source, forge_source_names};

#[test]
fn github_sync_is_registered_and_parses_github_urls() {
    assert!(forge_source_names().contains(&"github-sync"));
    let plugin = get_forge_source("github-sync").expect("github-sync plugin registered");
    assert_eq!(plugin.name(), "github-sync");
    assert_eq!(
        plugin.parse_url("https://github.com/owner/repo").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(plugin.token_env(), Some("GITHUB_TOKEN"));
}
