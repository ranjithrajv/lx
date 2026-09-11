// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::debs::*;
use lx_lib::github::{Asset, Release};

fn asset(name: &str) -> Asset {
    Asset {
        name: name.to_string(),
        size: None,
        browser_download_url: format!("https://github.com/x/y/releases/download/v1/{name}"),
    }
}

fn release(assets: Vec<Asset>) -> Release {
    Release {
        tag_name: "v1.0.0".into(),
        prerelease: false,
        draft: false,
        html_url: "https://github.com/x/y/releases".into(),
        assets,
        published_at: None,
        body: None,
    }
}

#[test]
fn repo_name_appends_debian_suffix() {
    assert_eq!(repo_name("eza"), "eza-debian");
    assert_eq!(repo_name("git-delta"), "git-delta-debian");
}

#[test]
fn finds_matching_asset() {
    let r = release(vec![
        asset("eza_0.24.0-1+trixie_amd64.deb"),
        asset("eza_0.24.0-1+trixie_arm64.deb"),
        asset("eza_0.24.0-1+bookworm_amd64.deb"),
    ]);
    let hit = find_asset(&r, "eza", "amd64", "trixie").unwrap();
    assert_eq!(hit.name, "eza_0.24.0-1+trixie_amd64.deb");
}

#[test]
fn no_match_for_wrong_arch() {
    let r = release(vec![asset("eza_0.24.0-1+trixie_arm64.deb")]);
    assert!(find_asset(&r, "eza", "amd64", "trixie").is_none());
}

#[test]
fn no_cross_package_match() {
    let r = release(vec![asset("eza-extras_0.24.0-1+trixie_amd64.deb")]);
    assert!(find_asset(&r, "eza", "amd64", "trixie").is_none());
}

#[test]
fn extracts_control_version() {
    let v = control_version("eza_0.24.0-1+trixie_amd64.deb", "eza", "amd64").unwrap();
    assert_eq!(v, "0.24.0-1+trixie");
}

#[test]
fn control_version_none_on_mismatch() {
    assert!(control_version("eza_0.24.0-1+trixie_amd64.deb", "eza", "arm64").is_none());
}

#[test]
fn parse_depends_strips_versions_and_takes_first_alternative() {
    let deps = parse_depends_field("libc6 (>= 2.34), libssl3 | libssl1.1, adduser");
    assert_eq!(deps, vec!["libc6", "libssl3", "adduser"]);
}

#[test]
fn parse_depends_empty_field_is_empty() {
    assert!(parse_depends_field("").is_empty());
}
