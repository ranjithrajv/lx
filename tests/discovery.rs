// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::discovery::*;

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
fn matches_x86_64_to_amd64() {
    let r = release(vec![
        asset("eza_x86_64-unknown-linux-gnu.tar.gz"),
        asset("eza_aarch64-unknown-linux-gnu.tar.gz"),
    ]);
    let m = match_assets(&r);
    assert!(m.iter().any(|a| a.arch == "amd64"));
    assert!(m.iter().any(|a| a.arch == "arm64"));
}

#[test]
fn gnueabihf_is_armhf_not_armel() {
    let r = release(vec![asset("eza_arm-unknown-linux-gnueabihf.tar.gz")]);
    let m = match_assets(&r);
    assert!(m.iter().any(|a| a.arch == "armhf"));
    assert!(!m.iter().any(|a| a.arch == "armel"));
}

#[test]
fn x86_64_does_not_match_i386() {
    let r = release(vec![asset("eza_x86_64-unknown-linux-gnu.tar.gz")]);
    let m = match_assets(&r);
    assert!(!m.iter().any(|a| a.arch == "i386"));
    assert!(m.iter().any(|a| a.arch == "amd64"));
}

#[test]
fn windows_assets_are_ignored_when_linux_exists() {
    let r = release(vec![
        asset("eza_x86_64-pc-windows-gnu.tar.gz"),
        asset("eza_x86_64-unknown-linux-gnu.tar.gz"),
    ]);
    let m = match_assets(&r);
    assert!(m.iter().any(|a| a.arch == "amd64"));
    // must point at the linux asset
    let amd64 = m.iter().find(|a| a.arch == "amd64").unwrap();
    assert!(amd64.asset.contains("linux"));
}

#[test]
fn x86_64_zip_does_not_become_i386() {
    let r = release(vec![
        asset("eza_x86_64-unknown-linux-gnu.tar.gz"),
        asset("eza_x86_64-unknown-linux-gnu.zip"),
    ]);
    let m = match_assets(&r);
    assert!(m.iter().any(|a| a.arch == "amd64"));
    assert!(!m.iter().any(|a| a.arch == "i386"));
}

#[test]
fn real_i386_asset_still_matches() {
    let r = release(vec![asset("tool_i386-linux.tar.gz")]);
    let m = match_assets(&r);
    assert!(m.iter().any(|a| a.arch == "i386"));
}

#[test]
fn no_match_is_empty() {
    let r = release(vec![asset("eza-windows.zip")]);
    assert!(match_assets(&r).is_empty());
}

#[test]
fn guesses_formats() {
    assert_eq!(guess_format("foo.tar.gz"), "tar.gz");
    assert_eq!(guess_format("foo.tgz"), "tar.gz");
    assert_eq!(guess_format("foo.zip"), "zip");
    assert_eq!(guess_format("foo"), "raw");
}
