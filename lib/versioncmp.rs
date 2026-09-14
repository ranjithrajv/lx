// SPDX-License-Identifier: GPL-3.0-or-later

//! Cross-format package version comparison for the consumer layer.
//!
//! Debian versions are compared with `dpkg --compare-versions`, i.e. the
//! host's own rules (see [`crate::debs::is_newer`]). RPM and Arch versions
//! are compared with RPM's `EVR` ordering, which is the algorithm both
//! `rpm` and `pacman`'s `vercmp` derive from and which both encode as
//! `[epoch:]version-release`. Using one implementation keeps the consumer
//! layer free of host tools on non-Debian systems.

use std::cmp::Ordering;

/// Compare two `[epoch:]version[-release]` strings with RPM's ordering.
/// Valid for RPM and pacman versions alike.
pub fn rpm_evr_cmp(a: &str, b: &str) -> Ordering {
    rpm::Evr::parse(a).cmp(&rpm::Evr::parse(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn identical_versions_are_equal() {
        assert_eq!(rpm_evr_cmp("1.2.3-1", "1.2.3-1"), Ordering::Equal);
        // An absent epoch is equivalent to epoch 0.
        assert_eq!(rpm_evr_cmp("1.2.3-1", "0:1.2.3-1"), Ordering::Equal);
    }

    #[test]
    fn numeric_segments_order_numerically_not_lexically() {
        assert_eq!(rpm_evr_cmp("1.9", "1.10"), Ordering::Less);
        assert_eq!(rpm_evr_cmp("1.10", "1.9"), Ordering::Greater);
    }

    #[test]
    fn release_breaks_ties_on_version() {
        assert_eq!(rpm_evr_cmp("1.0-2", "1.0-1"), Ordering::Greater);
        assert_eq!(rpm_evr_cmp("1.0-1.fc40", "1.0-1.fc39"), Ordering::Greater);
    }

    #[test]
    fn epoch_overrides_version_and_release() {
        assert_eq!(rpm_evr_cmp("1:0.1-1", "9.9-9"), Ordering::Greater);
        assert_eq!(rpm_evr_cmp("0.1-1", "1:0.0-1"), Ordering::Less);
    }

    #[test]
    fn tilde_prerelease_sorts_below_the_release() {
        assert_eq!(rpm_evr_cmp("1.0~rc1-1", "1.0-1"), Ordering::Less);
    }
}
