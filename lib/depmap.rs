// SPDX-License-Identifier: GPL-3.0-or-later

//! Dependency mapping and resolution: registry packages → system packages.
//!
//! Full dependency resolution reads dependency files from fetched registry
//! packages, parses version constraints, resolves them against the target
//! distribution, and maps them to system package names.
//!
//! This is what makes lx a true *2deb replacement.

use std::collections::BTreeSet;

/// A resolved dependency with optional version constraint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedDep {
    pub system_name: String,
    pub version: Option<String>,
}

impl ResolvedDep {
    pub fn to_control_string(&self) -> String {
        match &self.version {
            Some(v) => format!("{} ({})", self.system_name, v),
            None => self.system_name.clone(),
        }
    }
}

/// Resolve all dependencies for a registry package from its fetched files.
///
/// Returns a sorted, deduplicated list of system package dependencies.
pub fn resolve_deps_from_dir(
    ecosystem: &str,
    package_dir: &std::path::Path,
    format: &str,
) -> Vec<ResolvedDep> {
    let raw_deps = read_raw_deps(ecosystem, package_dir);
    let mut resolved = BTreeSet::new();

    for (dep_name, version) in raw_deps {
        if let Some(system_name) = map_dependency(ecosystem, &dep_name, format) {
            let version = version.and_then(|v| normalize_version(&v, format));
            resolved.insert(ResolvedDep {
                system_name,
                version,
            });
        }
    }

    resolved.into_iter().collect()
}

/// Read raw (name, version) pairs from the package's dependency files.
fn read_raw_deps(ecosystem: &str, package_dir: &std::path::Path) -> Vec<(String, Option<String>)> {
    match ecosystem {
        "npm" => read_npm_deps(&package_dir.join("package.json")),
        "python" => {
            let req = package_dir.join("requirements.txt");
            if req.exists() {
                read_requirements_txt(&req)
            } else {
                read_setup_py_deps(&package_dir.join("setup.py"))
            }
        }
        "cargo" => read_cargo_deps(&package_dir.join("Cargo.toml")),
        "gem" => read_gemfile(&package_dir.join("Gemfile")),
        "cpan" => {
            let makefile = package_dir.join("Makefile.PL");
            if makefile.exists() {
                read_makefile_pl_deps(&makefile)
            } else {
                read_build_pl_deps(&package_dir.join("Build.PL"))
            }
        }
        "composer" => read_composer_deps(&package_dir.join("composer.json")),
        "maven" => read_maven_deps(&package_dir.join("pom.xml")),
        "hex" => read_mix_deps(&package_dir.join("mix.exs")),
        "dart" => read_pubspec_deps(&package_dir.join("pubspec.yaml")),
        "go" => read_go_mod_deps(&package_dir.join("go.mod")),
        _ => Vec::new(),
    }
}

/// Normalize a version constraint string to Debian/RPM format.
fn normalize_version(version: &str, format: &str) -> Option<String> {
    let v = version.trim();
    if v.is_empty() || v == "*" || v == "latest" || v == "any" {
        return None;
    }

    // Handle common version constraint prefixes.
    if let Some(rest) = v.strip_prefix(">=") {
        match format {
            "deb" => Some(format!(">= {}", rest.trim())),
            "rpm" => Some(format!(">= {}", rest.trim())),
            "arch" => Some(format!(">={}", rest.trim())),
            _ => None,
        }
    } else if let Some(rest) = v.strip_prefix(">") {
        match format {
            "deb" => Some(format!(">> {}", rest.trim())),
            "rpm" => Some(format!("> {}", rest.trim())),
            "arch" => Some(format!(">{}", rest.trim())),
            _ => None,
        }
    } else if let Some(rest) = v.strip_prefix("~>") {
        // Pessimistic constraint: ~> 1.2 means >= 1.2, < 2.0
        match format {
            "deb" => Some(format!(">= {}", rest.trim())),
            _ => Some(format!(">= {}", rest.trim())),
        }
    } else if let Some(rest) = v.strip_prefix("^") {
        // Caret constraint: ^1.2.3 means >= 1.2.3, < 2.0.0
        match format {
            "deb" => Some(format!(">= {}", rest.trim())),
            _ => Some(format!(">= {}", rest.trim())),
        }
    } else if v.starts_with('=') {
        Some(format!("= {}", v.trim_start_matches('=').trim()))
    } else if v
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        // Bare version number.
        Some(format!(">= {}", v.trim_matches(&[' ', '~', '^', '*'][..])))
    } else {
        None
    }
}

/// Map a registry dependency name to a system package name.
pub fn map_dependency(ecosystem: &str, dep_name: &str, format: &str) -> Option<String> {
    let deb = match ecosystem {
        "npm" => map_npm_dep(dep_name),
        "python" => map_python_dep(dep_name),
        "gem" | "ruby" => map_gem_dep(dep_name),
        "go" => return None,
        "hex" | "elixir" => map_hex_dep(dep_name),
        "dart" => return None,
        "cargo" | "rust" => map_cargo_dep(dep_name),
        "maven" | "java" => return None,
        "composer" | "php" => map_composer_dep(dep_name),
        "cpan" | "perl" => map_cpan_dep(dep_name),
        "nuget" => return None,
        _ => return None,
    };
    deb.map(|d| to_format(d, format))
}

// --- Ecosystem mappings ---

fn map_npm_dep(name: &str) -> Option<&'static str> {
    match name {
        "sharp" => Some("libvips"),
        "canvas" => Some("libcairo2"),
        "sqlite3" => Some("libsqlite3-0"),
        "bcrypt" => Some("libbcrypt"),
        "node-sass" => Some("libsass"),
        "puppeteer" => Some("libnss3"),
        "mongodb" => Some("libmongoc-1.0-0"),
        "usb" => Some("libusb-1.0-0"),
        "serialport" => Some("libserialport0"),
        "zlib" => Some("zlib1g"),
        _ => None,
    }
}

fn map_python_dep(name: &str) -> Option<&'static str> {
    match name {
        "pillow" | "Pillow" => Some("libjpeg62-turbo"),
        "lxml" => Some("libxml2"),
        "cryptography" => Some("libssl3"),
        "psycopg2" => Some("libpq5"),
        "psycopg2-binary" => Some("libpq5"),
        "mysqlclient" => Some("libmariadb3"),
        "mysql-connector-python" => Some("libmariadb3"),
        "numpy" => Some("libopenblas0"),
        "cffi" => Some("libffi8"),
        "pyyaml" => Some("libyaml-0-2"),
        "markdown" => Some("libmarkdown2"),
        "grpc" | "grpcio" => Some("libgrpc++1"),
        _ => None,
    }
}

fn map_gem_dep(name: &str) -> Option<&'static str> {
    match name {
        "nokogiri" => Some("libxml2"),
        "pg" => Some("libpq5"),
        "mysql2" => Some("libmariadb3"),
        "sqlite3" => Some("libsqlite3-0"),
        "rmagick" => Some("libmagickwand-6.q16-6"),
        "grpc" => Some("libgrpc++1"),
        "ffi" => Some("libffi8"),
        "mini_magick" => Some("libmagickwand-6.q16-6"),
        _ => None,
    }
}

fn map_hex_dep(name: &str) -> Option<&'static str> {
    match name {
        "comeonin" => Some("libbcrypt"),
        "argon2_elixir" => Some("libargon2-1"),
        "ex_libsodium" => Some("libsodium23"),
        "bcrypt_elixir" => Some("libbcrypt"),
        "gettext" => Some("libgettextpo0"),
        _ => None,
    }
}

fn map_cargo_dep(name: &str) -> Option<&'static str> {
    match name {
        "openssl-sys" => Some("libssl3"),
        "sqlite3-sys" | "libsqlite3-sys" => Some("libsqlite3-0"),
        "pq-sys" => Some("libpq5"),
        "mysqlclient-sys" | "mysql-sys" => Some("libmariadb3"),
        "zlib-sys" | "libz-sys" => Some("zlib1g"),
        "reqwest" => Some("libssl3"),
        "git2" => Some("libgit2-1.7"),
        "sass-rs" => Some("libsass"),
        "curl-sys" => Some("libcurl4"),
        "flate2-sys" | "libflate" => Some("zlib1g"),
        "libgit2-sys" => Some("libgit2-1.7"),
        "libssh2-sys" => Some("libssh2-1"),
        "libusb-sys" => Some("libusb-1.0-0"),
        _ => None,
    }
}

fn map_composer_dep(name: &str) -> Option<&'static str> {
    match name {
        "ext-curl" => Some("libcurl4"),
        "ext-gd" => Some("libgd3"),
        "ext-imagick" => Some("libmagickwand-6.q16-6"),
        "ext-intl" => Some("libicu74"),
        "ext-mbstring" => Some("libonig5"),
        "ext-mysqli" | "ext-pdo_mysql" => Some("libmariadb3"),
        "ext-pdo_sqlite" | "ext-sqlite3" => Some("libsqlite3-0"),
        "ext-zip" => Some("libzip4"),
        "ext-openssl" | "ext-sodium" => Some("libsodium23"),
        "ext-zlib" => Some("zlib1g"),
        _ => None,
    }
}

fn map_cpan_dep(name: &str) -> Option<&'static str> {
    match name {
        "DBD::Pg" => Some("libpq5"),
        "DBD::mysql" => Some("libmariadb3"),
        "DBD::SQLite" => Some("libsqlite3-0"),
        "XML::LibXML" => Some("libxml2"),
        "XML::Parser" => Some("libexpat1"),
        "GD" => Some("libgd3"),
        "Image::Magick" => Some("libmagickwand-6.q16-6"),
        "Net::SSLeay" => Some("libssl3"),
        "IO::Socket::SSL" => Some("libssl3"),
        "LWP::UserAgent" => Some("libwww-perl"),
        "JSON::XS" => Some("libjson-xs-perl"),
        "DBI" => Some("libdbi-perl"),
        _ => None,
    }
}

/// Convert Debian package name to target format.
fn to_format(deb_name: &str, format: &str) -> String {
    match format {
        "deb" => deb_name.to_string(),
        "rpm" => deb_to_rpm_name(deb_name).to_string(),
        "arch" => deb_to_arch_name(deb_name).to_string(),
        _ => deb_name.to_string(),
    }
}

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

// --- Dependency file readers ---

fn read_npm_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
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
            if let Some((name, version)) = t.split_once(':') {
                let name = name.trim().trim_matches('"').trim_matches(',');
                let version = version.trim().trim_matches('"').trim_matches(',').trim();
                if !name.is_empty() {
                    deps.push((
                        name.to_string(),
                        if version.is_empty() {
                            None
                        } else {
                            Some(version.to_string())
                        },
                    ));
                }
            }
        }
    }
    deps
}

fn read_requirements_txt(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with('-') {
                return None;
            }
            // Parse package[extras]>=version.
            let name = t
                .split(&['=', '>', '<', '!', '~', '[', ';', ' '])
                .next()
                .unwrap_or(t)
                .trim();
            let version = if t.contains(&['=', '>', '<', '~'][..]) {
                t.split(&['=', '>', '<', '~']).nth(1).map(|v| {
                    v.trim()
                        .trim_matches(&['=', '>', '<', '~', ' '][..])
                        .to_string()
                })
            } else {
                None
            };
            if name.is_empty() {
                None
            } else {
                Some((name.to_string(), version))
            }
        })
        .collect()
}

fn read_setup_py_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t.contains("install_requires") && t.contains('[') {
            in_deps = true;
            continue;
        }
        if in_deps {
            if t == "]" || t == "]," {
                break;
            }
            let name = t
                .trim_matches(&['[', ']', ',', '\'', '"'][..])
                .split(&['=', '>', '<', '!', '~'])
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                deps.push((name.to_string(), None));
            }
        }
    }
    deps
}

fn read_cargo_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
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
            if let Some((name, rest)) = t.split_once('=') {
                let name = name.trim();
                let version = rest.trim().trim_matches(&[' ', '"', '{'][..]);
                let version = version
                    .split(',')
                    .next()
                    .unwrap_or(version)
                    .trim()
                    .trim_matches('"');
                deps.push((
                    name.to_string(),
                    if version.is_empty() {
                        None
                    } else {
                        Some(version.to_string())
                    },
                ));
            }
        }
    }
    deps
}

fn read_gemfile(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.starts_with("gem ") {
                let parts: Vec<&str> = t.split_whitespace().collect();
                let name = parts.get(1)?.trim_matches(&[' ', '\'', '"'][..]);
                let version = parts
                    .get(2)
                    .map(|v| v.trim_matches(&[' ', '\'', ','][..]).to_string());
                Some((name.to_string(), version))
            } else {
                None
            }
        })
        .collect()
}

fn read_makefile_pl_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_prereq = false;
    for line in text.lines() {
        let t = line.trim();
        if t.contains("PREREQ_PM") && t.contains('{') {
            in_prereq = true;
            continue;
        }
        if in_prereq {
            if t == "}" || t == "}," {
                break;
            }
            if let Some((name, _)) = t.split_once('\'') {
                let parts: Vec<&str> = name.split("'=>'").collect();
                if !parts.is_empty() {
                    let name = parts[0].trim_matches('\'');
                    if !name.is_empty() {
                        deps.push((name.to_string(), None));
                    }
                }
            }
        }
    }
    deps
}

fn read_build_pl_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    read_makefile_pl_deps(path) // Same format for deps section.
}

fn read_composer_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("\"require\"") {
            in_deps = true;
            continue;
        }
        if in_deps {
            if t == "}" || t == "}," {
                break;
            }
            if let Some((name, version)) = t.split_once(':') {
                let name = name.trim().trim_matches('"').trim_matches(',');
                let version = version.trim().trim_matches('"').trim_matches(',').trim();
                if !name.is_empty() && !name.starts_with("php") {
                    deps.push((
                        name.to_string(),
                        if version.is_empty() {
                            None
                        } else {
                            Some(version.to_string())
                        },
                    ));
                }
            }
        }
    }
    deps
}

fn read_maven_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    // Simple XML parsing for <dependency> blocks.
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_dep = false;
    let mut group_id = String::new();
    let mut artifact_id = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t == "<dependency>" {
            in_dep = true;
            group_id.clear();
            artifact_id.clear();
            continue;
        }
        if t == "</dependency>" {
            in_dep = false;
            if !artifact_id.is_empty() {
                deps.push((artifact_id.clone(), None));
            }
            continue;
        }
        if in_dep {
            if t.starts_with("<groupId>") {
                group_id = t.replace("<groupId>", "").replace("</groupId>", "");
            } else if t.starts_with("<artifactId>") {
                artifact_id = t.replace("<artifactId>", "").replace("</artifactId>", "");
            }
        }
    }
    deps
}

fn read_mix_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t == "defp deps do" {
            in_deps = true;
            continue;
        }
        if in_deps {
            if t == "end" {
                break;
            }
            if t.starts_with("{:") {
                let parts: Vec<&str> = t.trim_matches(&['{', '}', ','][..]).split(',').collect();
                if let Some(name) = parts.first() {
                    let name = name.trim().trim_matches(':').trim_matches(&[' ', '\''][..]);
                    if !name.is_empty() {
                        deps.push((name.to_string(), None));
                    }
                }
            }
        }
    }
    deps
}

fn read_pubspec_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("dependencies:") {
            in_deps = true;
            continue;
        }
        if in_deps {
            if t.starts_with("dev_dependencies:") || (!t.starts_with(' ') && !t.starts_with('#')) {
                break;
            }
            if let Some((name, version)) = t.split_once(':') {
                let name = name.trim().trim_matches('#');
                let version = version.trim().trim_matches(&[' ', '^', '~'][..]);
                if !name.is_empty() && !name.starts_with("//") {
                    deps.push((
                        name.to_string(),
                        if version.is_empty() || version == "any" {
                            None
                        } else {
                            Some(version.to_string())
                        },
                    ));
                }
            }
        }
    }
    deps
}

fn read_go_mod_deps(path: &std::path::Path) -> Vec<(String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("require (") {
            in_block = true;
            continue;
        }
        if in_block {
            if t == ")" {
                break;
            }
            let parts: Vec<&str> = t.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].trim();
                let version = parts[1].trim().trim_start_matches('v');
                if !name.is_empty() {
                    deps.push((
                        name.to_string(),
                        if version.is_empty() {
                            None
                        } else {
                            Some(version.to_string())
                        },
                    ));
                }
            }
        } else if t.starts_with("require ") && !t.contains('(') {
            let parts: Vec<&str> = t.split_whitespace().collect();
            if parts.len() >= 3 {
                let name = parts[1].trim();
                let version = parts[2].trim().trim_start_matches('v');
                deps.push((
                    name.to_string(),
                    if version.is_empty() {
                        None
                    } else {
                        Some(version.to_string())
                    },
                ));
            }
        }
    }
    deps
}

// --- Legacy API for backward compatibility ---

/// Map multiple dependencies (legacy API).
pub fn map_dependencies(ecosystem: &str, deps: &[String], format: &str) -> Vec<String> {
    deps.iter()
        .filter_map(|d| map_dependency(ecosystem, d, format))
        .collect()
}

/// Infer dependencies (legacy API, now with full resolution).
pub fn infer_deps_from_dir(
    ecosystem: &str,
    package_dir: &std::path::Path,
    format: &str,
) -> Option<String> {
    let resolved = resolve_deps_from_dir(ecosystem, package_dir, format);
    if resolved.is_empty() {
        None
    } else {
        Some(
            resolved
                .iter()
                .map(|d| d.to_control_string())
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_npm_sharp() {
        assert_eq!(
            map_dependency("npm", "sharp", "deb"),
            Some("libvips".to_string())
        );
    }

    #[test]
    fn map_python_pillow() {
        assert_eq!(
            map_dependency("python", "pillow", "deb"),
            Some("libjpeg62-turbo".to_string())
        );
    }

    #[test]
    fn map_cargo_openssl() {
        assert_eq!(
            map_dependency("cargo", "openssl-sys", "deb"),
            Some("libssl3".to_string())
        );
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(map_dependency("npm", "lodash", "deb"), None);
    }

    #[test]
    fn version_gte_normalized_deb() {
        assert_eq!(
            normalize_version(">= 1.2.3", "deb"),
            Some(">= 1.2.3".to_string())
        );
    }

    #[test]
    fn version_tilde_normalized() {
        assert_eq!(
            normalize_version("~> 1.2", "deb"),
            Some(">= 1.2".to_string())
        );
    }

    #[test]
    fn version_star_returns_none() {
        assert_eq!(normalize_version("*", "deb"), None);
    }

    #[test]
    fn version_caret_normalized() {
        assert_eq!(
            normalize_version("^1.2.3", "deb"),
            Some(">= 1.2.3".to_string())
        );
    }

    #[test]
    fn resolved_dep_to_string_no_version() {
        let dep = ResolvedDep {
            system_name: "libvips".to_string(),
            version: None,
        };
        assert_eq!(dep.to_control_string(), "libvips");
    }

    #[test]
    fn resolved_dep_to_string_with_version() {
        let dep = ResolvedDep {
            system_name: "libssl3".to_string(),
            version: Some(">= 3.0".to_string()),
        };
        assert_eq!(dep.to_control_string(), "libssl3 (>= 3.0)");
    }
}
