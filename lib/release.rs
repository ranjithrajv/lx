// SPDX-License-Identifier: GPL-3.0-or-later

//! Provider-agnostic release domain model.
//!
//! [`Release`]/[`Asset`]/[`ReleaseMeta`]/[`RepoLicense`] are shared by every
//! forge client (github, gitlab, gitea, …). They used to live in `github.rs`,
//! so the module named after one provider owned the types every provider uses
//! and `github` had to reach into `gitlab` for timestamp parsing — a
//! provider-to-provider dependency. They live here so no provider owns them
//! and the parser is shared.

use serde::{Deserialize, Serialize};

/// Parse an ISO-8601 / RFC3339 timestamp (`2025-01-01T00:00:00[.fff]Z`) into
/// Unix epoch seconds. Shared by every forge client's release mapper.
pub fn parse_timestamp(s: &str) -> Option<i64> {
    // jiff's RFC3339 parser handles both forms; strptime is the fallback.
    if let Ok(ts) = s.parse::<jiff::Timestamp>() {
        return Some(ts.as_second());
    }
    jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%S%.fZ", s)
        .or_else(|_| jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%SZ", s))
        .ok()
        .map(|t| t.as_second())
}

/// Minimal release metadata for suggestions.
#[derive(Debug, Clone)]
pub struct ReleaseMeta {
    pub tag: String,
    pub published_at: Option<String>,
}

/// Repository license metadata (SPDX id + optional full text).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoLicense {
    pub spdx: String,
    pub text: Option<String>,
}

/// Domain model for a forge release (the subset lx needs, normalized so every
/// forge source shares one release type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub prerelease: bool,
    pub draft: bool,
    pub html_url: String,
    pub assets: Vec<Asset>,
    /// Unix epoch seconds the release was published, when the provider
    /// reports one. Used as the reproducible-build timestamp source for
    /// generated package metadata (changelog date, copyright year) instead of
    /// wall-clock build time, so the same release always produces the same
    /// bytes regardless of when it's built.
    pub published_at: Option<i64>,
    /// The release's own markdown notes, when the provider reports any. Used
    /// as a ready-made "changelog" for `lx update --diff` instead of deriving
    /// one from commits/tags.
    #[serde(default)]
    pub body: Option<String>,
}

/// Domain model for a release asset (the subset lx needs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: Option<u64>,
    pub browser_download_url: String,
    /// Inline checksums the provider already published, as algorithm → hex
    /// (e.g. `sha256` from a GitHub/Gitea asset `digest`, or `md5` from a
    /// SourceForge feed). Verified after download when no sidecar is found.
    #[serde(default)]
    pub checksums: std::collections::BTreeMap<String, String>,
}
