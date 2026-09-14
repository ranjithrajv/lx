// SPDX-License-Identifier: GPL-3.0-or-later

//! Centralized constants for `lx` — hosts, timeouts, TTLs, file modes.
//! Single source of truth for values previously duplicated across `lib/` and `src/`.

/// Default VCS hosts
pub const DEFAULT_GITHUB_HOST: &str = "github.com";
pub const DEFAULT_GITLAB_HOST: &str = "gitlab.com";
pub const DEFAULT_GITEA_HOST: &str = "codeberg.org";
pub const DEFAULT_FORGEJO_HOST: &str = "codeberg.org";
pub const DEFAULT_BITBUCKET_HOST: &str = "bitbucket.org";
pub const DEFAULT_GERRIT_HOST: &str = "review.gerrithub.io";
pub const DEFAULT_GITEE_HOST: &str = "gitee.com";
pub const DEFAULT_SOURCEFORGE_HOST: &str = "sourceforge.net";

/// Default API base URLs
pub const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";
pub const DEFAULT_GITLAB_API_URL: &str = "https://gitlab.com/api/v4";
pub const DEFAULT_GITEA_API_URL: &str = "https://codeberg.org/api/v1";
pub const DEFAULT_FORGEJO_API_URL: &str = "https://codeberg.org/api/v1";
pub const DEFAULT_BITBUCKET_API_URL: &str = "https://api.bitbucket.org/2.0";
pub const DEFAULT_GERRIT_API_URL: &str = "https://review.gerrithub.io/a";
pub const DEFAULT_GITEE_API_URL: &str = "https://gitee.com/api/v5";
pub const DEFAULT_SOURCEFORGE_API_URL: &str = "https://sourceforge.net";

/// Registry org for `lx install`'s `latest-debs` workflow
pub const LATEST_DEBS_ORG: &str = "latest-debs";

/// User-Agent for all HTTP clients. Version tracked to the crate version
/// so bumps don't leave a stale UA string behind.
pub const USER_AGENT: &str = concat!("lx/", env!("CARGO_PKG_VERSION"), " (latest package tool)");

/// Default filename for a package definition, used as the clap default on
/// every subcommand that takes a config path.
pub const DEFAULT_CONFIG_FILENAME: &str = "package.yaml";

/// Env var overriding [`DEFAULT_MAINTAINER`] when `maintainer:` is unset —
/// lets downstream users of lx attribute packages to themselves instead
/// of the latest-debs org.
pub const MAINTAINER_ENV_VAR: &str = "LX_MAINTAINER";

/// Cache TTLs (action parity)
pub const DOWNLOAD_CACHE_TTL_SECS: u64 = 86_400; // 24h
pub const API_CACHE_TTL_SECS: u64 = 300; // 5min

/// HTTP timeouts
pub const CONNECT_TIMEOUT_SECS: u64 = 30;
pub const READ_TIMEOUT_SECS: u64 = 300;

/// Debian suites known to the tool. Mirrors the action's
/// `(.debian_distributions // ["bullseye", "bookworm", "trixie", "forky", "sid"])`
/// default.
pub const DEFAULT_DEBIAN_DISTRIBUTIONS: &[&str] =
    &["bullseye", "bookworm", "trixie", "forky", "sid"];

/// Suite -> LTS support end (YYYY-MM-DD), mirroring the action's
/// system.yaml `distributions.details.<suite>.lts_support_ends` and its
/// `filter_expired_distributions`: once Debian drops a suite, lx stops
/// building for it even if a package.yaml still lists it. Suites absent
/// here (forky, sid) have no fixed end date and always pass.
pub const DISTRIBUTION_LTS_ENDS: &[(&str, &str)] = &[
    ("bullseye", "2026-08-31"),
    ("bookworm", "2028-06-30"),
    ("trixie", "2030-06-30"),
    // Ubuntu suites (standard-support, non-ESM, endoflife.date/ubuntu).
    ("jammy", "2027-04-30"),
    ("noble", "2029-05-30"),
    ("questing", "2026-07-01"),
    ("resolute", "2031-04-30"),
];

/// Ubuntu suites available as opt-in native builds via
/// `ubuntu_distributions` (mirrors the action's distributions.yaml).
pub const UBUNTU_DISTRIBUTIONS: &[&str] = &["jammy", "noble", "questing", "resolute"];

/// True when the distribution is an Ubuntu suite (builds FROM ubuntu:<suite>
/// in the bash action; natively here, same as Debian suites).
pub fn is_ubuntu_dist(dist: &str) -> bool {
    matches!(
        dist,
        "jammy" | "noble" | "questing" | "resolute" | "plucky" | "oracular"
    )
}

/// Default architectures for each Ubuntu suite (distributions.yaml).
pub fn ubuntu_archs(dist: &str) -> &'static [&'static str] {
    match dist {
        "jammy" => &["amd64", "arm64", "armhf", "ppc64el", "s390x"],
        "noble" | "questing" | "resolute" | "plucky" | "oracular" => {
            &["amd64", "arm64", "armhf", "ppc64el", "s390x", "riscv64"]
        }
        _ => &[],
    }
}

/// RPM-based distros
pub const DEFAULT_RPM_DISTRIBUTIONS: &[&str] = &["fedora", "el9", "el8", "opensuse"];

/// Arch rolling
pub const DEFAULT_ARCH_DISTRIBUTIONS: &[&str] = &["arch"];

/// Alpine Linux (apk). Rolling release, one suite.
pub const DEFAULT_APK_DISTRIBUTIONS: &[&str] = &["alpine"];

/// OpenWrt / opkg (ipk). Embedded Linux, one target family.
pub const DEFAULT_IPK_DISTRIBUTIONS: &[&str] = &["openwrt"];

/// MSIX (Windows). Not a Linux distro; one synthetic suite so the
/// distribution×architecture job matrix still applies.
pub const DEFAULT_MSIX_DISTRIBUTIONS: &[&str] = &["windows"];

/// macOS flat package (osxpkg). One synthetic suite.
pub const DEFAULT_OSX_DISTRIBUTIONS: &[&str] = &["macos"];

/// All Debian architectures the tool can target
pub const DEFAULT_ARCHITECTURES: &[&str] = &[
    "amd64", "arm64", "armel", "armhf", "i386", "ppc64el", "s390x", "riscv64", "loong64",
];

/// Architectures supported on every Debian suite (system.yaml)
pub const UNIVERSAL_ARCHS: &[&str] = &[
    "amd64", "arm64", "armhf", "ppc64el", "s390x", "riscv64", "loong64",
];

/// File modes (reproducible-build normalization)
pub const DIR_MODE: u32 = 0o755;
pub const FILE_MODE: u32 = 0o644;
pub const SYMLINK_MODE: u32 = 0o777;
pub const EXEC_MODE_MASK: u32 = 0o111;
pub const AR_MODE: u32 = 0o100644;

/// Progress / summary artefact paths
pub const DEFAULT_PROGRESS_PATH: &str = "/tmp/build_progress.json";
pub const SUMMARY_FILENAME: &str = "build-summary.json";

/// Debian package defaults
pub const DEFAULT_MAINTAINER: &str = "latest-debs maintainers <maintainers@latest-debs.org>";
pub const DEFAULT_SECTION: &str = "utils";
pub const DEFAULT_PRIORITY: &str = "optional";
pub const DEBHELPER_COMPAT: &str = "debhelper-compat (= 13)";
pub const STANDARDS_VERSION: &str = "4.6.2";

/// Homepage helper — provider-agnostic
pub fn homepage_for(host: &str, repo: &str) -> String {
    format!("https://{host}/{repo}")
}

/// Build a `https://{host}/{repo}` homepage, falling back to `default_host`
/// when `host` is empty and normalizing a configured host (strip scheme and a
/// trailing slash). Shared by every provider helper below.
pub fn homepage_with_host(repo: &str, host: &str, default_host: &str) -> String {
    let h = if host.is_empty() {
        default_host
    } else {
        host.trim_end_matches('/')
            .trim_start_matches("https://")
            .trim_start_matches("http://")
    };
    homepage_for(h, repo)
}

pub fn homepage_for_github(repo: &str) -> String {
    homepage_for(DEFAULT_GITHUB_HOST, repo)
}

pub fn homepage_for_gitlab(repo: &str, host: &str) -> String {
    homepage_with_host(repo, host, DEFAULT_GITLAB_HOST)
}

pub fn homepage_for_gitea(repo: &str, host: &str) -> String {
    homepage_with_host(repo, host, DEFAULT_GITEA_HOST)
}

pub fn homepage_for_forgejo(repo: &str, host: &str) -> String {
    homepage_with_host(repo, host, DEFAULT_FORGEJO_HOST)
}

pub fn homepage_for_bitbucket(repo: &str) -> String {
    // Bitbucket owner/repo → https://bitbucket.org/{repo}
    homepage_for(DEFAULT_BITBUCKET_HOST, repo)
}

pub fn homepage_for_gerrit(repo: &str, host: &str) -> String {
    homepage_with_host(repo, host, DEFAULT_GERRIT_HOST)
}

pub fn homepage_for_gitee(repo: &str, host: &str) -> String {
    homepage_with_host(repo, host, DEFAULT_GITEE_HOST)
}

/// SourceForge projects are identified by a bare project name (no owner),
/// so the homepage is the project page rather than a `host/repo` path.
pub fn homepage_for_sourceforge(project: &str) -> String {
    format!(
        "https://{}/projects/{}/",
        DEFAULT_SOURCEFORGE_HOST,
        project.trim_matches('/')
    )
}

/// Map Debian arch to RPM arch (reproducible, static)
pub fn to_rpm_arch(debian_arch: &str) -> &'static str {
    match debian_arch {
        "amd64" => "x86_64",
        "arm64" => "aarch64",
        "armhf" => "armhfp",
        "armel" => "armhfp",
        "i386" => "i386",
        "ppc64el" => "ppc64le",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        "loong64" => "loongarch64",
        other => Box::leak(other.to_string().into_boxed_str()) as &str,
    }
}

/// Map Debian arch to Alpine (apk) arch.
///
/// Alpine names differ from Debian for the 32-bit ARM family (`armv7`/
/// `armhf`) and `i386` (`x86`); everything else matches RPM-style names.
pub fn to_alpine_arch(debian_arch: &str) -> &'static str {
    match debian_arch {
        "amd64" => "x86_64",
        "arm64" => "aarch64",
        "armhf" => "armv7",
        "armel" => "armhf",
        "i386" => "x86",
        "ppc64el" => "ppc64le",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        "loong64" => "loongarch64",
        other => Box::leak(other.to_string().into_boxed_str()) as &str,
    }
}

/// Map Debian arch to OpenWrt (ipk) arch.
///
/// OpenWrt arch names are target-specific (`aarch64_generic`,
/// `arm_cortex-a9`, …); this is a best-effort default that callers can
/// override with `arch_variant:`. x86_64/s390x/riscv64 map 1:1.
pub fn to_openwrt_arch(debian_arch: &str) -> &'static str {
    match debian_arch {
        "amd64" => "x86_64",
        "arm64" => "aarch64_generic",
        "armhf" => "arm_cortex-a9",
        "armel" => "arm_cortex-a9",
        "i386" => "i386_pentium4",
        "ppc64el" => "powerpc64",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        "loong64" => "loongarch64",
        other => Box::leak(other.to_string().into_boxed_str()) as &str,
    }
}

/// Map Debian arch to pacman (Arch) arch
pub fn to_pacman_arch(debian_arch: &str) -> &'static str {
    match debian_arch {
        "amd64" => "x86_64",
        "arm64" => "aarch64",
        "armhf" => "armv7h",
        "armel" => "armv6h",
        "i386" => "i686",
        "ppc64el" => "ppc64le",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        "loong64" => "loong64",
        other => Box::leak(other.to_string().into_boxed_str()) as &str,
    }
}
