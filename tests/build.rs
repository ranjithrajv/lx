use lpt_lib::build::*;

#[test]
fn verify_method_as_str_names_every_variant() {
    assert_eq!(VerifyMethod::Pinned.as_str(), "pinned");
    assert_eq!(VerifyMethod::Sidecar.as_str(), "sidecar");
    assert_eq!(
        VerifyMethod::UnverifiedAllowed.as_str(),
        "unverified (--allow-unverified)"
    );
    assert_eq!(
        VerifyMethod::SkippedNoVerify.as_str(),
        "skipped (--no-verify)"
    );
}

#[test]
fn parse_github_url_extracts_owner_repo() {
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza").as_deref(),
        Some("eza-community/eza")
    );
}

#[test]
fn parse_github_url_ignores_trailing_path() {
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza/releases/tag/v0.24.0").as_deref(),
        Some("eza-community/eza")
    );
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza.git").as_deref(),
        Some("eza-community/eza")
    );
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza/").as_deref(),
        Some("eza-community/eza")
    );
}

#[test]
fn parse_github_url_rejects_non_github_or_incomplete() {
    assert!(parse_github_url("package.yaml").is_none());
    assert!(parse_github_url("configs/eza.yaml").is_none());
    assert!(parse_github_url("https://gitlab.com/owner/repo").is_none());
    assert!(parse_github_url("https://github.com/owner-only").is_none());
}

#[test]
fn host_arch_returns_a_known_debian_arch() {
    // Smoke test on this (Linux) dev/CI host: --host relies on
    // host_arch() resolving to one of the architectures the build
    // matrix actually knows about.
    let arch = host_arch().expect("uname -m should resolve on Linux");
    assert!(
        lpt_lib::config::DEFAULT_ARCHITECTURES.contains(&arch.as_str()),
        "unexpected arch: {arch}"
    );
}

#[test]
fn version_placeholder_dedupes_literal_v_prefix() {
    use lpt_lib::build::expand_version_placeholder as x;
    // Legacy pattern with literal v + v-prefixed tag -> single v.
    assert_eq!(
        x("eza_v{version}_x86_64-unknown-linux-gnu.tar.gz", "v0.23.5"),
        "eza_v0.23.5_x86_64-unknown-linux-gnu.tar.gz"
    );
    // Bare-version tag (the action's convention) keeps the literal v.
    assert_eq!(
        x("pkg_v{version}_linux.tar.gz", "1.2.3"),
        "pkg_v1.2.3_linux.tar.gz"
    );
    // Modern patterns without an adjacent v get the tag verbatim.
    assert_eq!(x("tool-{version}.tar.gz", "v9.9.9"), "tool-v9.9.9.tar.gz");
    assert_eq!(x("tool-{version}.tar.gz", "9.9.9"), "tool-9.9.9.tar.gz");
    // Case-insensitive on both sides.
    assert_eq!(x("Pkg_V{version}.zip", "V3.0"), "Pkg_V3.0.zip");
    // Multiple placeholders, each deduped.
    assert_eq!(x("a_v{version}/b-{version}", "v2"), "a_v2/b-v2");
}
