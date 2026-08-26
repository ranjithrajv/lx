use lpt_lib::plugins::source::{get_source_plugin, source_available_names};

#[test]
fn github_sync_is_registered_and_parses_github_urls() {
    assert!(source_available_names().contains(&"github-sync"));
    let plugin = get_source_plugin("github-sync").expect("github-sync plugin registered");
    assert_eq!(plugin.name(), "github-sync");
    assert_eq!(
        plugin.parse_url("https://github.com/owner/repo").as_deref(),
        Some("owner/repo")
    );
    assert_eq!(plugin.token_env(), Some("GITHUB_TOKEN"));
}
