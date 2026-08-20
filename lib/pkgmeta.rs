//! Shared Debian package metadata rendering: epoch/version handling,
//! reproducible-builds-aware timestamps, and the changelog/copyright bodies
//! used by both a binary `.deb` (`src/build.rs`) and its companion source
//! package (`src/source.rs`).
//!
//! These two were previously independent, hand-rolled implementations that
//! had already drifted apart in practice -- the source package's changelog
//! date and copyright year were hardcoded literals instead of using the
//! same reproducible-builds-aware timestamp the binary package computes,
//! and its `debian/control` was missing the dependency-relation fields the
//! binary control file has. Extracted here so both consumers share one
//! implementation instead of two that can silently diverge again.

/// `<epoch>:<version>` when `epoch` is set, else `version` unchanged.
/// Epoch belongs in `Version:` control/`.dsc` fields and changelog entries
/// but never in filenames -- Debian policy excludes it there since `:`
/// isn't filename-safe.
pub fn with_epoch(epoch: &str, version: &str) -> String {
    if epoch.trim().is_empty() {
        version.to_string()
    } else {
        format!("{}:{version}", epoch.trim())
    }
}

/// Debian policy requires the `Version` field to start with a digit, but
/// upstream tags commonly carry a non-digit prefix (e.g. `v0.23.5`, bun's
/// `bun-v1.3.14`). Strips any leading run of non-digit characters,
/// mirroring the original action's `sed -E 's/^[^0-9]*//'`.
pub fn strip_upstream_prefix(version: &str) -> String {
    version
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .to_string()
}

/// Render a `Name: value\n` control-file line for a comma-separated
/// relation field (Depends/Recommends/Conflicts/Replaces/Provides/Breaks),
/// or an empty string when unset.
fn relation_field(name: &str, value: &str) -> String {
    if value.trim().is_empty() {
        String::new()
    } else {
        format!("{name}: {}\n", value.trim())
    }
}

/// A package's dependency-relation fields, rendered together in the
/// conventional order (Depends, Recommends, Conflicts, Replaces, Provides,
/// Breaks) for appending after a control stanza's `Description`. Shared by
/// the binary `.deb`'s `DEBIAN/control` and the source package's
/// `debian/control` binary-package stanza, so the two can't independently
/// forget a field the way the source package's `Depends:` (and friends)
/// previously did entirely.
#[derive(Debug, Clone, Default)]
pub struct Relations {
    pub depends: String,
    pub recommends: String,
    pub conflicts: String,
    pub replaces: String,
    pub provides: String,
    pub breaks: String,
}

impl Relations {
    pub fn render(&self) -> String {
        [
            relation_field("Depends", &self.depends),
            relation_field("Recommends", &self.recommends),
            relation_field("Conflicts", &self.conflicts),
            relation_field("Replaces", &self.replaces),
            relation_field("Provides", &self.provides),
            relation_field("Breaks", &self.breaks),
        ]
        .concat()
    }
}

/// Timestamp source for reproducible package metadata (changelog date,
/// copyright year, source-tarball mtimes): the `SOURCE_DATE_EPOCH` env var
/// if set (the reproducible-builds.org standard, letting operators pin an
/// exact value), else the release's own publish time, else a fixed epoch.
/// Deliberately never wall-clock "now" -- building from build time would
/// make the same release produce different package metadata depending on
/// when it's built, which is exactly what reproducible builds rule out.
pub fn reproducible_epoch(published_at: Option<i64>) -> i64 {
    let env_override = std::env::var("SOURCE_DATE_EPOCH").ok();
    parse_source_date_epoch(env_override.as_deref())
        .or(published_at)
        .unwrap_or(0)
}

/// Parse a `SOURCE_DATE_EPOCH` value, split out from `reproducible_epoch`
/// so its precedence logic is testable without mutating the real process
/// environment (a global shared with every other test in this binary,
/// which run in parallel by default -- a prior version of this test set
/// and unset the env var directly and intermittently leaked into unrelated
/// tests reading `reproducible_epoch(None)` concurrently).
fn parse_source_date_epoch(raw: Option<&str>) -> Option<i64> {
    raw?.trim().parse::<i64>().ok()
}

/// RFC 2822 date (e.g. `Thu, 14 Aug 2026 09:30:00 +0000`) for a changelog
/// trailer, derived from `reproducible_epoch`.
pub fn changelog_date(published_at: Option<i64>) -> String {
    let secs = reproducible_epoch(published_at);
    jiff::Timestamp::from_second(secs)
        .map(|t| t.strftime("%a, %d %b %Y %H:%M:%S %z").to_string())
        .unwrap_or_default()
}

/// The 4-digit year for a copyright notice, derived the same way as
/// `changelog_date` so both stay consistent with each other.
pub fn copyright_year(published_at: Option<i64>) -> String {
    let secs = reproducible_epoch(published_at);
    jiff::Timestamp::from_second(secs)
        .map(|t| t.strftime("%Y").to_string())
        .unwrap_or_else(|_| "1970".to_string())
}

/// Render a `debian/changelog` entry:
/// `<pkg> (<full_version>) <dist>; urgency=medium\n\n  * New upstream
/// release <upstream_version>\n\n -- <maintainer>  <date>\n`.
/// Shared by the binary `.deb`'s changelog and the source package's, so a
/// hardcoded/stale date can't drift between them again.
pub fn render_changelog_entry(
    pkg_name: &str,
    full_version: &str,
    dist: &str,
    upstream_version: &str,
    maintainer: &str,
    published_at: Option<i64>,
) -> String {
    format!(
        "{pkg_name} ({full_version}) {dist}; urgency=medium\n\n  * New upstream release \
         {upstream_version}\n\n -- {maintainer}  {date}\n",
        date = changelog_date(published_at),
    )
}

/// Render `debian/copyright`, shared by the binary `.deb` and its source
/// package.
///
/// `license` is the upstream license fetched from GitHub (SPDX id +
/// optional full text); `license_spdx_fallback` is used when GitHub
/// couldn't detect one (or when running without a `license` lookup at
/// all).
pub fn render_copyright(
    pkg_name: &str,
    github_repo: &str,
    license: Option<&crate::github::RepoLicense>,
    license_spdx_fallback: &str,
    published_at: Option<i64>,
) -> String {
    let year = copyright_year(published_at);
    let fallback_spdx = if license_spdx_fallback.is_empty() {
        "NOASSERTION"
    } else {
        license_spdx_fallback
    };
    let spdx = license
        .map(|l| l.spdx.as_str())
        .filter(|s| !s.is_empty() && *s != "NOASSERTION")
        .unwrap_or(fallback_spdx);
    let body = license.and_then(|l| l.text.clone()).unwrap_or_else(|| {
        " No machine-readable license text could be detected upstream;\n see the project's \
          repository for licensing terms."
            .to_string()
    });
    format!(
        "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n\
         Upstream-Name: {pkg_name}\n\
         Upstream-Contact: https://github.com/{github_repo}/issues\n\
         Source: https://github.com/{github_repo}\n\n\
         Files: *\n\
         Copyright: {year} {github_repo} contributors\n\
         License: {spdx}\n\n\
         Files: debian/*\n\
         Copyright: {year} latest-debs\n\
         License: {spdx}\n\n\
         License: {spdx}\n{body}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let license = crate::github::RepoLicense {
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
}
