// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::depmap::apk::deb_to_alpine;
use lx_lib::plugins::depmap::ipk::deb_to_openwrt;
use lx_lib::plugins::depmap::*;

#[test]
fn registry_covers_every_package_format() {
    let names = dependency_mapper_names();
    for n in ["debian", "rpm", "pacman", "alpine", "openwrt"] {
        assert!(names.contains(&n), "missing mapper {n}");
    }
    for f in ["deb", "rpm", "arch", "apk", "ipk"] {
        assert!(get_dependency_mapper(f).is_some(), "missing format {f}");
    }
    assert!(get_dependency_mapper_by_name("debian").is_some());
    assert!(get_dependency_mapper_by_name("nope").is_none());
    assert!(get_dependency_mapper("nope").is_none());
}

#[test]
fn deb_mapper_matches_the_shared_core() {
    assert_eq!(
        map_to_format("npm", "sharp", "deb"),
        Some("libvips".to_string())
    );
    assert_eq!(
        map_to_format("python", "pillow", "deb"),
        Some("libjpeg62-turbo".to_string())
    );
    // Unknown ecosystem dep is unmapped.
    assert_eq!(map_to_format("npm", "lodash", "deb"), None);
}

#[test]
fn constraints_render_per_format() {
    let deb = get_dependency_mapper("deb").unwrap();
    assert_eq!(deb.constraint(">= 1.2.3"), Some(">= 1.2.3".to_string()));
    let arch = get_dependency_mapper("arch").unwrap();
    assert_eq!(arch.constraint(">= 1.2.3"), Some(">=1.2.3".to_string()));
}

#[test]
fn alpine_renders_unparenthesized_and_translates_names() {
    assert_eq!(render_for_format("musl", Some(">=1.2"), "apk"), "musl>=1.2");
    assert_eq!(render_for_format("musl", None, "apk"), "musl");
    assert_eq!(deb_to_alpine("libc6"), "musl");
    assert_eq!(deb_to_alpine("zlib1g"), "zlib");
    assert_eq!(deb_to_alpine("libunknown"), "libunknown");
}

#[test]
fn openwrt_translates_names_but_keeps_debian_syntax() {
    assert_eq!(deb_to_openwrt("libc6"), "libc");
    assert_eq!(deb_to_openwrt("libssl3"), "libopenssl");
    assert_eq!(
        render_for_format("libc", Some(">= 1.0"), "ipk"),
        "libc (>= 1.0)"
    );
}

#[test]
fn unknown_format_falls_back_to_debian_syntax() {
    assert_eq!(render_for_format("foo", Some(">= 1"), "nope"), "foo (>= 1)");
    assert_eq!(render_for_format("foo", None, "nope"), "foo");
}
