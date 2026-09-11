// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx go-native` parser unit tests: snap/flatpak/nix listings, shell
//! history URL extraction, and mapping-table spot checks. All pure
//! functions — no live snap/flatpak/nix required.

use lx_lib::go_native::{
    go_native_mapping_probe, parse_flatpak_list, parse_nix_profile, parse_sh_urls, parse_snap_list,
};

#[test]
fn parses_snap_list_columns() {
    let text = "Name    Version  Rev  Tracking\nfirefox 1.2.3   99   latest/stable\ncore22  2024     11   latest/stable\n";
    assert_eq!(
        parse_snap_list(text),
        vec![
            ("firefox".to_string(), "1.2.3".to_string()),
            ("core22".to_string(), "2024".to_string()),
        ]
    );
}

#[test]
fn parses_flatpak_app_rows_and_skips_runtimes() {
    let text = "org.mozilla.firefox\t1.2.3\ncom.spotify.Client\t\n";
    assert_eq!(
        parse_flatpak_list(text),
        vec![
            ("org.mozilla.firefox".to_string(), "1.2.3".to_string()),
            ("com.spotify.Client".to_string(), String::new()),
        ]
    );
}

#[test]
fn parses_nix_profile_attr_paths() {
    let text = "0 flake:nixpkgs#hello  attr-path: legacyPackages.x86_64-linux.hello\n";
    assert_eq!(
        parse_nix_profile(text),
        vec![("hello".to_string(), String::new())]
    );
}

#[test]
fn extracts_curl_sh_urls_from_history() {
    let text = "ls\ncurl -fsSL https://astral.sh/uv/install.sh | sh\nsudo apt install foo\n";
    assert_eq!(
        parse_sh_urls(text),
        vec!["https://astral.sh/uv/install.sh".to_string()]
    );
}

#[test]
fn mapping_table_covers_common_managed_apps() {
    for (source, id, native) in [
        ("snap", "firefox", "firefox"),
        ("snap", "spotify", "spotify-client"),
        ("flatpak", "org.mozilla.firefox", "firefox"),
        ("flatpak", "com.spotify.Client", "spotify-client"),
        ("nix", "firefox", "firefox"),
    ] {
        assert_eq!(
            go_native_mapping_probe(source, id, &[]),
            Some(native.to_string()),
            "source={source} id={id}"
        );
    }
}

#[test]
fn mapping_table_covers_curl_sh_tools_by_binary() {
    for (binary, native) in [
        ("/home/u/.local/bin/uv", "uv"),
        ("/usr/local/bin/starship", "starship"),
        ("/home/u/.local/bin/bun", "bun"),
    ] {
        assert_eq!(
            go_native_mapping_probe("sh", binary, &[]),
            Some(native.to_string()),
            "binary={binary}"
        );
    }
}

#[test]
fn sh_url_history_attributes_installer() {
    let urls = ["https://astral.sh/uv/install.sh".to_string()];
    assert_eq!(
        go_native_mapping_probe("sh", "/home/u/.local/bin/uv", &urls),
        Some("uv".to_string())
    );
}

#[test]
fn unknown_packages_map_to_nothing() {
    assert_eq!(go_native_mapping_probe("snap", "core22", &[]), None);
    assert_eq!(
        go_native_mapping_probe("flatpak", "com.example.Nope", &[]),
        None
    );
    assert_eq!(
        go_native_mapping_probe("sh", "/usr/local/bin/my-own-tool", &[]),
        None
    );
}
