use lpt_lib::pkgmeta::*;

#[test]
fn with_epoch_prefixes_only_when_set() {
    assert_eq!(with_epoch("", "1.0-1"), "1.0-1");
    assert_eq!(with_epoch("1", "1.0-1"), "1:1.0-1");
    assert_eq!(with_epoch(" 2 ", "1.0-1"), "2:1.0-1");
}

#[test]
fn strip_upstream_prefix_removes_leading_non_digits() {
    assert_eq!(strip_upstream_prefix("v0.23.5"), "0.23.5");
    assert_eq!(strip_upstream_prefix("bun-v1.3.14"), "1.3.14");
    assert_eq!(strip_upstream_prefix("0.23.5"), "0.23.5");
}

#[test]
fn relations_render_only_nonempty_fields_in_order() {
    let r = Relations {
        depends: "libc6".into(),
        breaks: "foo-legacy (<< 2.0)".into(),
        ..Default::default()
    };
    assert_eq!(r.render(), "Depends: libc6\nBreaks: foo-legacy (<< 2.0)\n");
    assert_eq!(Relations::default().render(), "");
}

#[test]
fn relations_render_suggests_and_predepends_in_order() {
    let r = Relations {
        depends: "libc6".into(),
        recommends: "bash-completion".into(),
        suggests: "bar".into(),
        predepends: "libc6 (>= 2.35)".into(),
        ..Default::default()
    };
    let rendered = r.render();
    // Order: Depends, Recommends, Suggests, ..., Pre-Depends
    let deps_idx = rendered.find("Depends:").unwrap();
    let rec_idx = rendered.find("Recommends:").unwrap();
    let sug_idx = rendered.find("Suggests:").unwrap();
    let pre_idx = rendered.find("Pre-Depends:").unwrap();
    assert!(deps_idx < rec_idx && rec_idx < sug_idx && sug_idx < pre_idx);
    assert!(rendered.contains("Suggests: bar\n"));
    assert!(rendered.contains("Pre-Depends: libc6 (>= 2.35)\n"));
}

#[test]
fn changelog_date_is_deterministic_from_published_at() {
    let a = changelog_date(Some(1_735_689_600)); // 2025-01-01T00:00:00Z
    let b = changelog_date(Some(1_735_689_600));
    assert_eq!(a, b);
    assert_eq!(a, "Wed, 01 Jan 2025 00:00:00 +0000");
}

#[test]
fn changelog_date_falls_back_to_epoch_zero_without_published_at() {
    assert_eq!(changelog_date(None), "Thu, 01 Jan 1970 00:00:00 +0000");
}

#[test]
fn copyright_year_matches_changelog_date_year() {
    assert_eq!(copyright_year(Some(1_735_689_600)), "2025");
    assert_eq!(copyright_year(None), "1970");
}

#[test]
fn parse_source_date_epoch_accepts_valid_and_rejects_bad_values() {
    assert_eq!(
        parse_source_date_epoch(Some("1000000000")),
        Some(1_000_000_000)
    );
    assert_eq!(
        parse_source_date_epoch(Some(" 1000000000 ")),
        Some(1_000_000_000)
    );
    assert_eq!(parse_source_date_epoch(Some("not-a-number")), None);
    assert_eq!(parse_source_date_epoch(None), None);
}

#[test]
fn render_changelog_entry_matches_expected_shape() {
    let entry = render_changelog_entry(
        "eza",
        "1:0.23.5-1+bookworm",
        "bookworm",
        "0.23.5",
        "t <t@example.com>",
        Some(1_735_689_600),
    );
    assert_eq!(
        entry,
        "eza (1:0.23.5-1+bookworm) bookworm; urgency=medium\n\n  * New upstream release \
             0.23.5\n\n -- t <t@example.com>  Wed, 01 Jan 2025 00:00:00 +0000\n"
    );
}

#[test]
fn render_copyright_falls_back_to_config_spdx_without_a_detected_license() {
    let text = render_copyright("eza", "eza-community/eza", None, "MIT", Some(1_735_689_600));
    assert!(text.contains("Copyright: 2025 eza-community/eza contributors"));
    assert!(text.contains("License: MIT"));
    assert!(text.contains("No machine-readable license text"));
}

#[test]
fn render_copyright_prefers_detected_license_over_config_fallback() {
    let license = lpt_lib::github::RepoLicense {
        spdx: "Apache-2.0".into(),
        text: Some("full license text".into()),
    };
    let text = render_copyright(
        "eza",
        "eza-community/eza",
        Some(&license),
        "MIT",
        Some(1_735_689_600),
    );
    assert!(text.contains("License: Apache-2.0"));
    assert!(text.contains("full license text"));
    assert!(!text.contains("No machine-readable license text"));
}
