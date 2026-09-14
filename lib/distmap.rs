// SPDX-License-Identifier: GPL-3.0-or-later

//! Cross-distribution **system package name** translation (deb ↔ rpm ↔ arch).
//!
//! `depmap` answers "what is this *language ecosystem* dependency called on
//! <format>?". This module answers a different question, needed by
//! `lx convert`: "this package depends on a system library; what is the same
//! library called on the *target distro's* format?".
//!
//! The table is curated and deliberately small-but-accurate. Rows are keyed by
//! upstream project and list the package name in each format. Versioned Debian
//! soname packages (`libssl3`, `libicu74`, …) are listed as aliases; the first
//! Debian alias is the canonical name used for reverse translation
//! (rpm/arch → deb). Unknown names return `None` so callers pass them through
//! unchanged rather than inventing a name.

/// One library across the three formats.
struct Row {
    /// Debian/Ubuntu package names: `[canonical, ..versioned sonames]`. The
    /// first entry is what reverse translation (rpm/arch → deb) emits.
    deb: &'static [&'static str],
    rpm: &'static str,
    arch: &'static str,
}

const TABLE: &[Row] = &[
    // --- C runtime / toolchain ---
    Row {
        deb: &["libc6"],
        rpm: "glibc",
        arch: "glibc",
    },
    Row {
        deb: &["libgcc-s1", "libgcc1"],
        rpm: "libgcc",
        arch: "gcc-libs",
    },
    Row {
        deb: &["libstdc++6"],
        rpm: "libstdc++",
        arch: "gcc-libs",
    },
    Row {
        deb: &["libgomp1"],
        rpm: "libgomp",
        arch: "gcc-libs",
    },
    // --- TLS / crypto ---
    Row {
        deb: &["libssl3", "libssl1.1", "libcrypto3"],
        rpm: "openssl-libs",
        arch: "openssl",
    },
    Row {
        deb: &["libsodium23"],
        rpm: "libsodium",
        arch: "libsodium",
    },
    Row {
        deb: &["libargon2-1"],
        rpm: "libargon2",
        arch: "argon2",
    },
    Row {
        deb: &["libnss3"],
        rpm: "nss",
        arch: "nss",
    },
    Row {
        deb: &["libkrb5-3"],
        rpm: "krb5-libs",
        arch: "krb5",
    },
    Row {
        deb: &["libssh2-1"],
        rpm: "libssh2",
        arch: "libssh2",
    },
    // --- Compression ---
    Row {
        deb: &["zlib1g"],
        rpm: "zlib",
        arch: "zlib",
    },
    Row {
        deb: &["libbz2-1.0"],
        rpm: "bzip2-libs",
        arch: "bzip2",
    },
    Row {
        deb: &["liblzma5"],
        rpm: "xz-libs",
        arch: "xz",
    },
    Row {
        deb: &["libzstd1"],
        rpm: "libzstd",
        arch: "zstd",
    },
    Row {
        deb: &["liblz4-1"],
        rpm: "lz4-libs",
        arch: "lz4",
    },
    // --- Data / databases ---
    Row {
        deb: &["libsqlite3-0"],
        rpm: "sqlite",
        arch: "sqlite",
    },
    Row {
        deb: &["libpq5"],
        rpm: "postgresql-libs",
        arch: "postgresql-libs",
    },
    Row {
        deb: &["libmariadb3"],
        rpm: "mariadb-connector-c",
        arch: "mariadb-libs",
    },
    Row {
        deb: &["libxml2"],
        rpm: "libxml2",
        arch: "libxml2",
    },
    Row {
        deb: &["libxslt1.1"],
        rpm: "libxslt",
        arch: "libxslt",
    },
    Row {
        deb: &["libyaml-0-2"],
        rpm: "libyaml",
        arch: "libyaml",
    },
    Row {
        deb: &["libicu74", "libicu72", "libicu70"],
        rpm: "libicu",
        arch: "icu",
    },
    Row {
        deb: &["libexpat1"],
        rpm: "expat",
        arch: "expat",
    },
    Row {
        deb: &["libffi8", "libffi7"],
        rpm: "libffi",
        arch: "libffi",
    },
    Row {
        deb: &["libprotobuf32", "libprotobuf23"],
        rpm: "protobuf",
        arch: "protobuf",
    },
    Row {
        deb: &["libsnappy1v5"],
        rpm: "snappy",
        arch: "snappy",
    },
    Row {
        deb: &["libevent-2.1-7"],
        rpm: "libevent",
        arch: "libevent",
    },
    // --- Networking / curl ---
    Row {
        deb: &["libcurl4"],
        rpm: "libcurl",
        arch: "curl",
    },
    Row {
        deb: &["libnghttp2-14"],
        rpm: "libnghttp2",
        arch: "libnghttp2",
    },
    // --- Compression-adjacent image / graphics ---
    Row {
        deb: &["libpng16-16"],
        rpm: "libpng",
        arch: "libpng",
    },
    Row {
        deb: &["libjpeg62-turbo"],
        rpm: "libjpeg-turbo",
        arch: "libjpeg-turbo",
    },
    Row {
        deb: &["libtiff6", "libtiff5"],
        rpm: "libtiff",
        arch: "libtiff",
    },
    Row {
        deb: &["libwebp7", "libwebp6"],
        rpm: "libwebp",
        arch: "libwebp",
    },
    Row {
        deb: &["libgd3"],
        rpm: "gd",
        arch: "gd",
    },
    Row {
        deb: &["libcairo2"],
        rpm: "cairo",
        arch: "cairo",
    },
    Row {
        deb: &["libvips"],
        rpm: "vips",
        arch: "vips",
    },
    Row {
        deb: &["libfreetype6"],
        rpm: "freetype",
        arch: "freetype2",
    },
    Row {
        deb: &["libfontconfig1"],
        rpm: "fontconfig",
        arch: "fontconfig",
    },
    Row {
        deb: &["libharfbuzz0b"],
        rpm: "harfbuzz",
        arch: "harfbuzz",
    },
    Row {
        deb: &["libmagickwand-6.q16-6"],
        rpm: "ImageMagick-libs",
        arch: "imagemagick",
    },
    // --- Terminal / text ---
    Row {
        deb: &["libncurses6", "libncursesw6"],
        rpm: "ncurses-libs",
        arch: "ncurses",
    },
    Row {
        deb: &["libreadline8"],
        rpm: "readline",
        arch: "readline",
    },
    Row {
        deb: &["libpcre3"],
        rpm: "pcre",
        arch: "pcre",
    },
    Row {
        deb: &["libpcre2-8-0"],
        rpm: "pcre2",
        arch: "pcre2",
    },
    Row {
        deb: &["libonig5"],
        rpm: "oniguruma",
        arch: "oniguruma",
    },
    // --- System / IPC ---
    Row {
        deb: &["libsystemd0", "libudev1"],
        rpm: "systemd-libs",
        arch: "systemd-libs",
    },
    Row {
        deb: &["libdbus-1-3"],
        rpm: "dbus-libs",
        arch: "dbus",
    },
    Row {
        deb: &["libglib2.0-0"],
        rpm: "glib2",
        arch: "glib2",
    },
    Row {
        deb: &["libcap2"],
        rpm: "libcap",
        arch: "libcap",
    },
    Row {
        deb: &["libacl1"],
        rpm: "libacl",
        arch: "acl",
    },
    Row {
        deb: &["libseccomp2"],
        rpm: "libseccomp",
        arch: "libseccomp",
    },
    Row {
        deb: &["libnl-3-200"],
        rpm: "libnl3",
        arch: "libnl",
    },
    Row {
        deb: &["libusb-1.0-0"],
        rpm: "libusb1",
        arch: "libusb",
    },
    Row {
        deb: &["libaio1"],
        rpm: "libaio",
        arch: "libaio",
    },
    Row {
        deb: &["libnuma1"],
        rpm: "numactl-libs",
        arch: "numactl",
    },
    // --- Audio/video ---
    Row {
        deb: &["libasound2"],
        rpm: "alsa-lib",
        arch: "alsa-lib",
    },
    Row {
        deb: &["libpulse0"],
        rpm: "pulseaudio-libs",
        arch: "libpulse",
    },
    Row {
        deb: &["libsndfile1"],
        rpm: "libsndfile",
        arch: "libsndfile",
    },
    Row {
        deb: &["libvorbis0a"],
        rpm: "libvorbis",
        arch: "libvorbis",
    },
    Row {
        deb: &["libflac12", "libflac8"],
        rpm: "flac-libs",
        arch: "flac",
    },
    Row {
        deb: &["libopus0"],
        rpm: "opus",
        arch: "opus",
    },
    Row {
        deb: &["libavcodec59", "libavcodec58"],
        rpm: "ffmpeg-libs",
        arch: "ffmpeg",
    },
    // --- Toolkits / X11 ---
    Row {
        deb: &["libgtk-3-0"],
        rpm: "gtk3",
        arch: "gtk3",
    },
    Row {
        deb: &["libx11-6"],
        rpm: "libX11",
        arch: "libx11",
    },
    // --- Misc libraries used by language ecosystems ---
    Row {
        deb: &["libsass"],
        rpm: "libsass",
        arch: "libsass",
    },
    Row {
        deb: &["libbcrypt"],
        rpm: "libbcrypt",
        arch: "bcrypt",
    },
    Row {
        deb: &["libopenblas0"],
        rpm: "openblas",
        arch: "openblas",
    },
    Row {
        deb: &["libgrpc++1"],
        rpm: "grpc-cpp",
        arch: "grpc",
    },
    Row {
        deb: &["libzip4"],
        rpm: "libzip",
        arch: "libzip",
    },
    Row {
        deb: &["libgit2-1.7", "libgit2-1.5"],
        rpm: "libgit2",
        arch: "libgit2",
    },
];

/// Translate a system package name from one package format to another.
///
/// Accepts `from`/`to` of `deb`, `rpm`, or `arch`. Returns `None` when the
/// name isn't in the curated table (or the formats are unknown/equal), so the
/// caller can keep the original name instead of guessing.
pub fn translate(name: &str, from: &str, to: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if from == to {
        return Some(name.to_string());
    }
    if !matches!(from, "deb" | "rpm" | "arch") || !matches!(to, "deb" | "rpm" | "arch") {
        return None;
    }
    let row = TABLE.iter().find(|row| match from {
        "deb" => row.deb.iter().any(|d| d.eq_ignore_ascii_case(name)),
        "rpm" => row.rpm.eq_ignore_ascii_case(name),
        "arch" => row.arch.eq_ignore_ascii_case(name),
        _ => false,
    })?;
    let target = match to {
        "deb" => row.deb.first().copied(),
        "rpm" => Some(row.rpm),
        "arch" => Some(row.arch),
        _ => None,
    }?;
    Some(target.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deb_to_rpm_and_arch() {
        assert_eq!(translate("libc6", "deb", "rpm").as_deref(), Some("glibc"));
        assert_eq!(translate("libc6", "deb", "arch").as_deref(), Some("glibc"));
        assert_eq!(
            translate("libssl3", "deb", "rpm").as_deref(),
            Some("openssl-libs")
        );
        assert_eq!(
            translate("libssl3", "deb", "arch").as_deref(),
            Some("openssl")
        );
        assert_eq!(translate("zlib1g", "deb", "rpm").as_deref(), Some("zlib"));
        assert_eq!(
            translate("libcurl4", "deb", "arch").as_deref(),
            Some("curl")
        );
    }

    #[test]
    fn rpm_and_arch_to_deb() {
        assert_eq!(translate("glibc", "rpm", "deb").as_deref(), Some("libc6"));
        assert_eq!(translate("glibc", "arch", "deb").as_deref(), Some("libc6"));
        assert_eq!(
            translate("openssl-libs", "rpm", "deb").as_deref(),
            Some("libssl3")
        );
        assert_eq!(
            translate("openssl", "arch", "deb").as_deref(),
            Some("libssl3")
        );
        assert_eq!(
            translate("curl", "arch", "deb").as_deref(),
            Some("libcurl4")
        );
        assert_eq!(
            translate("systemd-libs", "rpm", "deb").as_deref(),
            Some("libsystemd0")
        );
    }

    #[test]
    fn rpm_and_arch_cross() {
        assert_eq!(
            translate("openssl-libs", "rpm", "arch").as_deref(),
            Some("openssl")
        );
        assert_eq!(translate("curl", "arch", "rpm").as_deref(), Some("libcurl"));
    }

    #[test]
    fn case_insensitive_and_aliases() {
        assert_eq!(
            translate("LibSSL3", "deb", "rpm").as_deref(),
            Some("openssl-libs")
        );
        // Old soname aliases resolve to the same project.
        assert_eq!(
            translate("libssl1.1", "deb", "rpm").as_deref(),
            Some("openssl-libs")
        );
        assert_eq!(translate("libicu72", "deb", "arch").as_deref(), Some("icu"));
    }

    #[test]
    fn unknown_and_identity() {
        assert_eq!(translate("some-random-pkg", "deb", "rpm"), None);
        assert_eq!(translate("glibc", "rpm", "rpm").as_deref(), Some("glibc"));
        assert_eq!(translate("", "deb", "rpm"), None);
        assert_eq!(translate("glibc", "rpm", "weird"), None);
    }
}
