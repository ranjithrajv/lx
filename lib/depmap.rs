// SPDX-License-Identifier: GPL-3.0-or-later

//! Dependency mapping: registry packages → system packages.
//!
//! Maps language-ecosystem dependencies to their Debian/RPM/Arch equivalents.
//! This is what makes lx a true *2deb replacement — without it, registry
//! packages can't declare runtime dependencies on system libraries.
//!
//! Each ecosystem has its own mapping rules. The mappings are heuristic and
//! best-effort; users can always override with `depends:` in package.yaml.

use std::collections::BTreeMap;

/// Map a registry dependency name to a system package name for the given
/// format and ecosystem.
///
/// Returns `None` when no mapping is known — the caller should skip or warn.
pub fn map_dependency(ecosystem: &str, dep_name: &str, format: &str) -> Option<String> {
    match ecosystem {
        "npm" => map_npm_dep(dep_name, format),
        "python" => map_python_dep(dep_name, format),
        "gem" => map_gem_dep(dep_name, format),
        "go" => map_go_dep(dep_name, format),
        "hex" => map_hex_dep(dep_name, format),
        "dart" => map_dart_dep(dep_name, format),
        "cargo" => map_cargo_dep(dep_name, format),
        "maven" => map_maven_dep(dep_name, format),
        "composer" => map_composer_dep(dep_name, format),
        "cpan" => map_cpan_dep(dep_name, format),
        "nuget" => map_nuget_dep(dep_name, format),
        _ => None,
    }
}

/// Map multiple dependencies at once, returning only successful mappings.
pub fn map_dependencies(ecosystem: &str, deps: &[String], format: &str) -> Vec<String> {
    deps.iter()
        .filter_map(|d| map_dependency(ecosystem, d, format))
        .collect()
}

// --- npm (Node.js) ---
// Common npm packages that map to system libraries.
fn map_npm_dep(name: &str, format: &str) -> Option<String> {
    let deb = match name {
        "sharp" => "libvips",
        "canvas" => "libcairo2",
        "sqlite3" => "libsqlite3-0",
        "bcrypt" => "libbcrypt",
        "node-sass" => "libsass",
        "puppeteer" => "libnss3",
        _ => return None,
    };
    Some(to_format(deb, format))
}

// --- Python ---
// Common Python packages → system libraries.
fn map_python_dep(name: &str, format: &str) -> Option<String> {
    let deb = match name {
        "pillow" => "libjpeg62-turbo",
        "lxml" => "libxml2",
        "cryptography" => "libssl3",
        "psycopg2" => "libpq5",
        "mysqlclient" => "libmariadb3",
        "numpy" => "libopenblas0",
        "cffi" => "libffi8",
        "pyyaml" => "libyaml-0-2",
        _ => return None,
    };
    Some(to_format(deb, format))
}

// --- Ruby gems ---
fn map_gem_dep(name: &str, format: &str) -> Option<String> {
    let deb = match name {
        "nokogiri" => "libxml2",
        "pg" => "libpq5",
        "mysql2" => "libmariadb3",
        "sqlite3" => "libsqlite3-0",
        "rmagick" => "libmagickwand-6.q16-6",
        "grpc" => "libgrpc++1",
        _ => return None,
    };
    Some(to_format(deb, format))
}

// --- Go ---
// Go modules rarely need system deps (static binaries), but some do.
fn map_go_dep(_name: &str, _format: &str) -> Option<String> {
    // Go binaries are typically statically linked. No common mappings.
    None
}

// --- Elixir/Hex ---
fn map_hex_dep(name: &str, format: &str) -> Option<String> {
    let deb = match name {
        "comeonin" => "libbcrypt",
        "argon2_elixir" => "libargon2-1",
        "ex_libsodium" => "libsodium23",
        _ => return None,
    };
    Some(to_format(deb, format))
}

// --- Dart/Flutter ---
fn map_dart_dep(_name: &str, _format: &str) -> Option<String> {
    // Dart binaries are compiled ahead-of-time. No common mappings.
    None
}

// --- Rust/cargo ---
fn map_cargo_dep(name: &str, format: &str) -> Option<String> {
    let deb = match name {
        "openssl-sys" => "libssl3",
        "sqlite3-sys" => "libsqlite3-0",
        "libsqlite3-sys" => "libsqlite3-0",
        "pq-sys" => "libpq5",
        "mysqlclient-sys" => "libmariadb3",
        "zlib-sys" => "zlib1g",
        "libz-sys" => "zlib1g",
        "reqwest" => "libssl3",
        "git2" => "libgit2-1.7",
        "sass-rs" => "libsass",
        _ => return None,
    };
    Some(to_format(deb, format))
}

// --- Maven (Java) ---
fn map_maven_dep(_name: &str, _format: &str) -> Option<String> {
    // Java dependencies are bundled in the jar. No common system mappings.
    None
}

// --- Composer (PHP) ---
fn map_composer_dep(name: &str, format: &str) -> Option<String> {
    let deb = match name {
        "ext-curl" => "libcurl4",
        "ext-gd" => "libgd3",
        "ext-imagick" => "libmagickwand-6.q16-6",
        "ext-intl" => "libicu74",
        "ext-mbstring" => "libonig5",
        "ext-mysqli" => "libmariadb3",
        "ext-pdo_sqlite" => "libsqlite3-0",
        "ext-zip" => "libzip4",
        _ => return None,
    };
    Some(to_format(deb, format))
}

// --- CPAN (Perl) ---
fn map_cpan_dep(name: &str, format: &str) -> Option<String> {
    let deb = match name {
        "DBD::Pg" => "libpq5",
        "DBD::mysql" => "libmariadb3",
        "DBD::SQLite" => "libsqlite3-0",
        "XML::LibXML" => "libxml2",
        "GD" => "libgd3",
        "Image::Magick" => "libmagickwand-6.q16-6",
        "Net::SSLeay" => "libssl3",
        "IO::Socket::SSL" => "libssl3",
        _ => return None,
    };
    Some(to_format(deb, format))
}

// --- NuGet (.NET) ---
fn map_nuget_dep(_name: &str, _format: &str) -> Option<String> {
    // .NET dependencies are bundled. No common system mappings.
    None
}

/// Convert a Debian package name to the equivalent for the target format.
fn to_format(deb_name: &str, format: &str) -> String {
    match format {
        "deb" => deb_name.to_string(),
        "rpm" => deb_to_rpm_name(deb_name),
        "arch" => deb_to_arch_name(deb_name),
        _ => deb_name.to_string(),
    }
}

/// Best-effort Debian → RPM package name conversion.
fn deb_to_rpm_name(deb: &str) -> String {
    match deb {
        "libssl3" => "openssl-libs".to_string(),
        "libsqlite3-0" => "sqlite".to_string(),
        "libpq5" => "postgresql-libs".to_string(),
        "libmariadb3" => "mariadb-connector-c".to_string(),
        "libcurl4" => "libcurl".to_string(),
        "libgd3" => "gd".to_string(),
        "libxml2" => "libxml2".to_string(),
        "libvips" => "vips".to_string(),
        "libcairo2" => "cairo".to_string(),
        "zlib1g" => "zlib".to_string(),
        "libffi8" => "libffi".to_string(),
        "libgit2-1.7" => "libgit2".to_string(),
        "libicu74" => "libicu".to_string(),
        "libnss3" => "nss".to_string(),
        "libsodium23" => "libsodium".to_string(),
        "libgrpc++1" => "grpc-cpp".to_string(),
        "libonig5" => "oniguruma".to_string(),
        "libzip4" => "libzip".to_string(),
        "libopenblas0" => "openblas".to_string(),
        "libargon2-1" => "libargon2".to_string(),
        "libmagickwand-6.q16-6" => "ImageMagick-libs".to_string(),
        "libsass" => "libsass".to_string(),
        "libyaml-0-2" => "libyaml".to_string(),
        "libbcrypt" => "libbcrypt".to_string(),
        _ => deb.to_string(),
    }
}

/// Best-effort Debian → Arch package name conversion.
fn deb_to_arch_name(deb: &str) -> String {
    match deb {
        "libssl3" => "openssl".to_string(),
        "libsqlite3-0" => "sqlite".to_string(),
        "libpq5" => "postgresql-libs".to_string(),
        "libmariadb3" => "mariadb-libs".to_string(),
        "libcurl4" => "curl".to_string(),
        "libgd3" => "gd".to_string(),
        "libxml2" => "libxml2".to_string(),
        "libvips" => "vips".to_string(),
        "libcairo2" => "cairo".to_string(),
        "zlib1g" => "zlib".to_string(),
        "libffi8" => "libffi".to_string(),
        "libgit2-1.7" => "libgit2".to_string(),
        "libicu74" => "icu".to_string(),
        "libnss3" => "nss".to_string(),
        "libsodium23" => "libsodium".to_string(),
        "libgrpc++1" => "grpc".to_string(),
        "libonig5" => "oniguruma".to_string(),
        "libzip4" => "libzip".to_string(),
        "libopenblas0" => "openblas".to_string(),
        "libargon2-1" => "argon2".to_string(),
        "libmagickwand-6.q16-6" => "imagemagick".to_string(),
        "libsass" => "libsass".to_string(),
        "libyaml-0-2" => "libyaml".to_string(),
        "libbcrypt" => "bcrypt".to_string(),
        _ => deb.to_string(),
    }
}

/// Infer system dependencies for a registry package from its fetched files.
///
/// Reads dependency files from the package directory (package.json,
/// requirements.txt, Cargo.toml, etc.) and maps them to system packages.
/// Returns the comma-separated depends string, or None if no mappable deps.
pub fn infer_deps_from_dir(
    ecosystem: &str,
    package_dir: &std::path::Path,
    format: &str,
) -> Option<String> {
    let deps = match ecosystem {
        "npm" => {
            let pkg_json = package_dir.join("package.json");
            let deps = read_npm_deps(&pkg_json)?;
            map_dependencies("npm", &deps, format)
        }
        "python" => {
            let req_txt = package_dir.join("requirements.txt");
            if req_txt.exists() {
                let deps = read_python_deps(&req_txt)?;
                map_dependencies("python", &deps, format)
            } else {
                let setup_py = package_dir.join("setup.py");
                let deps = read_python_setup_deps(&setup_py)?;
                map_dependencies("python", &deps, format)
            }
        }
        "cargo" => {
            let cargo_toml = package_dir.join("Cargo.toml");
            let deps = read_cargo_deps(&cargo_toml)?;
            map_dependencies("cargo", &deps, format)
        }
        _ => Vec::new(),
    };

    if deps.is_empty() {
        None
    } else {
        Some(deps.join(", "))
    }
}

/// Read dependency names from a package.json (simple regex-free parse).
fn read_npm_deps(path: &std::path::Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("\"dependencies\"") {
            in_deps = true;
            continue;
        }
        if in_deps {
            if t == "}" || t == "}," {
                break;
            }
            if let Some(name) = t.split(':').next() {
                let name = name.trim().trim_matches('"').trim_matches(',');
                if !name.is_empty() {
                    deps.push(name.to_string());
                }
            }
        }
    }
    if deps.is_empty() {
        None
    } else {
        Some(deps)
    }
}

/// Read dependency names from a requirements.txt.
fn read_python_deps(path: &std::path::Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let deps: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                return None;
            }
            // Extract package name before version specifier.
            let name = t.split(&['=', '>', '<', '!', '~', ' ']).next().unwrap_or(t);
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect();
    if deps.is_empty() {
        None
    } else {
        Some(deps)
    }
}

/// Read dependency names from a setup.py (heuristic).
fn read_python_setup_deps(path: &std::path::Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut deps = Vec::new();
    let mut in_install_requires = false;
    for line in text.lines() {
        let t = line.trim();
        if t.contains("install_requires") {
            in_install_requires = true;
            continue;
        }
        if in_install_requires {
            if t == "]" || t == "]," {
                break;
            }
            if let Some(name) = t
                .trim_matches(&['[', ']', ',', '\'', '"'][..])
                .split('=')
                .next()
            {
                let name = name.trim();
                if !name.is_empty() {
                    deps.push(name.to_string());
                }
            }
        }
    }
    if deps.is_empty() {
        None
    } else {
        Some(deps)
    }
}

/// Read dependency names from a Cargo.toml.
fn read_cargo_deps(path: &std::path::Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t == "[dependencies]" {
            in_deps = true;
            continue;
        }
        if in_deps {
            if t.starts_with('[') {
                break;
            }
            if let Some(name) = t.split(&['=', ' ']).next() {
                let name = name.trim();
                if !name.is_empty() {
                    deps.push(name.to_string());
                }
            }
        }
    }
    if deps.is_empty() {
        None
    } else {
        Some(deps)
    }
}

/// Get all known mappings for an ecosystem (for documentation/debugging).
pub fn known_mappings(ecosystem: &str) -> BTreeMap<&'static str, &'static str> {
    let mut map = BTreeMap::new();
    let dummy_deps: Vec<&str> = match ecosystem {
        "npm" => vec![
            "sharp",
            "canvas",
            "sqlite3",
            "bcrypt",
            "node-sass",
            "puppeteer",
        ],
        "python" => vec![
            "pillow",
            "lxml",
            "cryptography",
            "psycopg2",
            "mysqlclient",
            "numpy",
            "cffi",
            "pyyaml",
        ],
        "gem" => vec!["nokogiri", "pg", "mysql2", "sqlite3", "rmagick", "grpc"],
        "rust" | "cargo" => vec![
            "openssl-sys",
            "sqlite3-sys",
            "pq-sys",
            "libz-sys",
            "reqwest",
            "git2",
        ],
        "hex" => vec!["comeonin", "argon2_elixir", "ex_libsodium"],
        "composer" => vec![
            "ext-curl",
            "ext-gd",
            "ext-imagick",
            "ext-intl",
            "ext-mbstring",
            "ext-zip",
        ],
        "cpan" => vec![
            "DBD::Pg",
            "DBD::mysql",
            "DBD::SQLite",
            "XML::LibXML",
            "GD",
            "Net::SSLeay",
        ],
        _ => return map,
    };
    for dep in dummy_deps {
        if let Some(mapped) = map_dependency(ecosystem, dep, "deb") {
            map.insert(dep, Box::leak(mapped.into_boxed_str()));
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_npm_sharp_to_libvips() {
        assert_eq!(
            map_dependency("npm", "sharp", "deb"),
            Some("libvips".to_string())
        );
    }

    #[test]
    fn maps_python_pillow_to_libjpeg() {
        assert_eq!(
            map_dependency("python", "pillow", "deb"),
            Some("libjpeg62-turbo".to_string())
        );
    }

    #[test]
    fn maps_cargo_openssl_to_libssl() {
        assert_eq!(
            map_dependency("cargo", "openssl-sys", "deb"),
            Some("libssl3".to_string())
        );
    }

    #[test]
    fn unknown_dep_returns_none() {
        assert_eq!(map_dependency("npm", "lodash", "deb"), None);
    }

    #[test]
    fn go_deps_are_typically_none() {
        assert_eq!(map_dependency("go", "gin-gonic/gin", "deb"), None);
    }

    #[test]
    fn rpm_conversion() {
        assert_eq!(
            map_dependency("npm", "sharp", "rpm"),
            Some("vips".to_string())
        );
    }

    #[test]
    fn arch_conversion() {
        assert_eq!(
            map_dependency("npm", "sharp", "arch"),
            Some("vips".to_string())
        );
    }
}
