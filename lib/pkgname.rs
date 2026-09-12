// SPDX-License-Identifier: GPL-3.0-or-later

//! Package naming conventions per ecosystem and format.
//!
//! Each language ecosystem has established naming conventions for system
//! packages. Applying these makes registry-source packages consistent with
//! what users expect from their distribution.
//!
//! Examples:
//!   Debian: libfoo-bar-perl, ruby-foo, node-foo, python3-foo
//!   RPM:    perl-Foo-Bar, ruby-foo, nodejs-foo, python3-foo
//!   Arch:   perl-foo-bar, ruby-foo, nodejs-foo, python-foo

/// Generate a convention-compliant package name for the given ecosystem and format.
///
/// If `user_name` is provided (user set `package_name` explicitly), it is
/// returned as-is. Otherwise, the name is derived from `registry_name`
/// using ecosystem conventions.
pub fn conventional_name(
    ecosystem: &str,
    registry_name: &str,
    format: &str,
    user_name: Option<&str>,
) -> String {
    // Respect explicit user choice.
    if let Some(name) = user_name {
        if !name.is_empty() {
            return name.to_string();
        }
    }

    match format {
        "deb" => deb_name(ecosystem, registry_name),
        "rpm" => rpm_name(ecosystem, registry_name),
        "arch" => arch_name(ecosystem, registry_name),
        _ => registry_name.to_string(),
    }
}

/// Debian naming conventions.
///
/// Policy: https://www.debian.org/doc/debian-policy/ch-controlfields.html#s-f-package-name
/// - Perl libraries: lib<name>-perl (e.g., libjson-perl)
/// - Ruby gems: ruby-<name> (e.g., rake → ruby-rake)
/// - Node modules: node-<name> (e.g., underscore → node-underscore)
/// - Python packages: python3-<name> (e.g., requests → python3-requests)
/// - PHP PEAR/Composer: php-<name>
/// - Lua rocks: lua-<name>
/// - Haskell: libghc-<name>-dev (libraries), <name> (tools)
/// - Rust: <name> (cargo handles this)
/// - Go: golang-<name>-dev (libraries), <name> (tools)
fn deb_name(ecosystem: &str, registry_name: &str) -> String {
    let normalized = normalize(registry_name);
    match ecosystem {
        "perl" => format!("lib{normalized}-perl"),
        "gem" | "ruby" => format!("ruby-{normalized}"),
        "npm" => format!("node-{normalized}"),
        "python" => format!("python3-{normalized}"),
        "composer" | "php" => format!("php-{normalized}"),
        "cpan" => format!("lib{normalized}-perl"),
        "hex" | "elixir" => format!("elixir-{normalized}"),
        "lua" => format!("lua-{normalized}"),
        "haskell" => format!("libghc-{normalized}-dev"),
        "maven" | "java" => normalized, // Java uses upstream names
        "cargo" | "rust" | "go" | "dart" | "nuget" => normalized, // Compiled tools keep name
        _ => normalized,
    }
}

/// RPM naming conventions (Fedora/EL).
///
/// Fedora: https://docs.fedoraproject.org/en-US/packaging-guidelines/Naming/
/// - Perl: perl-<Name::Name> → perl-Name-Name
/// - Ruby: ruby-<%= pkgname %>
/// - Node: nodejs-<name>
/// - Python: python3-<name>
/// - PHP: php-<name>
/// - Go: golang-<name>
/// - Rust: <name>
fn rpm_name(ecosystem: &str, registry_name: &str) -> String {
    let normalized = normalize(registry_name);
    let rpm_formatted = registry_name.replace("::", "-").replace('_', "-");
    match ecosystem {
        "perl" => format!("perl-{rpm_formatted}"),
        "gem" | "ruby" => format!("ruby-{normalized}"),
        "npm" => format!("nodejs-{normalized}"),
        "python" => format!("python3-{normalized}"),
        "composer" | "php" => format!("php-{normalized}"),
        "cpan" => format!("perl-{rpm_formatted}"),
        "hex" | "elixir" => format!("elixir-{normalized}"),
        "lua" => format!("lua-{normalized}"),
        "haskell" => format!("ghc-{normalized}"),
        "go" => format!("golang-{normalized}"),
        "maven" | "java" => normalized,
        "cargo" | "rust" | "dart" | "nuget" => normalized,
        _ => normalized,
    }
}

/// Arch Linux naming conventions.
///
/// https://wiki.archlinux.org/title/Package_guidelines#Package_naming
/// - Perl: perl-<name>
/// - Ruby: ruby-<name>
/// - Node: nodejs-<name>
/// - Python: python-<name> (or python2-<name>)
/// - PHP: php-<name>
/// - Go: go-<name> (libraries), <name> (tools)
/// - Rust: <name>
fn arch_name(ecosystem: &str, registry_name: &str) -> String {
    let normalized = normalize(registry_name);
    match ecosystem {
        "perl" | "cpan" => format!("perl-{normalized}"),
        "gem" | "ruby" => format!("ruby-{normalized}"),
        "npm" => format!("nodejs-{normalized}"),
        "python" => format!("python-{normalized}"),
        "composer" | "php" => format!("php-{normalized}"),
        "hex" | "elixir" => format!("elixir-{normalized}"),
        "lua" => format!("lua-{normalized}"),
        "haskell" => format!("haskell-{normalized}"),
        "go" => format!("go-{normalized}"),
        "maven" | "java" => normalized,
        "cargo" | "rust" | "dart" | "nuget" => normalized,
        _ => normalized,
    }
}

/// Normalize a registry name to a Debian-style package name.
///
/// Converts to lowercase, replaces `::` and `_` with `-`, strips common
/// prefixes/suffixes.
fn normalize(name: &str) -> String {
    name.to_lowercase()
        .replace("::", "-")
        .replace('_', "-")
        .trim_start_matches("lib")
        .trim_end_matches("-js")
        .trim_end_matches("-rb")
        .trim_end_matches("-py")
        .trim_end_matches("-perl")
        .trim_matches('-')
        .to_string()
}

/// Auto-detect whether a package should be `Architecture: all` or `Architecture: any`.
///
/// - `all`: Pure code (bytecode, scripts, interpreted) — runs on any CPU.
/// - `any`: Contains compiled extensions or binaries — CPU-specific.
pub fn detect_architecture(ecosystem: &str, has_compiled_extensions: bool) -> &'static str {
    if has_compiled_extensions {
        return "any";
    }
    match ecosystem {
        // Pure interpreted languages — always architecture-independent.
        "python" | "perl" | "ruby" | "npm" | "composer" | "php" | "lua" | "dart" => "all",
        // Compiled languages — architecture-specific.
        "cargo" | "rust" | "go" | "maven" | "java" | "haskell" | "hex" | "elixir" => "any",
        // Mixed — depends on content (handled by has_compiled_extensions flag).
        "cpan" | "gem" | "nuget" => "any",
        _ => "any",
    }
}

/// True if the package directory contains compiled extensions (.so, .bundle, .node).
pub fn has_compiled_extensions(dir: &std::path::Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.ends_with(".so")
                    || name.ends_with(".bundle")
                    || name.ends_with(".node")
                    || name.ends_with(".dylib")
                {
                    return true;
                }
            } else if let Ok(subdir) = std::fs::read_dir(&path) {
                for subentry in subdir.flatten() {
                    let subpath = subentry.path();
                    if subpath.is_file() {
                        let subname = subpath.file_name().unwrap_or_default().to_string_lossy();
                        if subname.ends_with(".so")
                            || subname.ends_with(".bundle")
                            || subname.ends_with(".node")
                            || subname.ends_with(".dylib")
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deb_perl_lib_name() {
        assert_eq!(
            conventional_name("perl", "JSON", "deb", None),
            "libjson-perl"
        );
    }

    #[test]
    fn deb_ruby_gem_name() {
        assert_eq!(conventional_name("gem", "rake", "deb", None), "ruby-rake");
    }

    #[test]
    fn deb_node_name() {
        assert_eq!(
            conventional_name("npm", "underscore", "deb", None),
            "node-underscore"
        );
    }

    #[test]
    fn deb_python_name() {
        assert_eq!(
            conventional_name("python", "requests", "deb", None),
            "python3-requests"
        );
    }

    #[test]
    fn rpm_perl_name() {
        assert_eq!(
            conventional_name("perl", "JSON::XS", "rpm", None),
            "perl-JSON-XS"
        );
    }

    #[test]
    fn arch_python_name() {
        assert_eq!(
            conventional_name("python", "requests", "arch", None),
            "python-requests"
        );
    }

    #[test]
    fn respects_explicit_user_name() {
        assert_eq!(
            conventional_name("perl", "JSON", "deb", Some("my-custom-name")),
            "my-custom-name"
        );
    }

    #[test]
    fn pure_python_is_arch_all() {
        assert_eq!(detect_architecture("python", false), "all");
    }

    #[test]
    fn python_with_so_is_arch_any() {
        assert_eq!(detect_architecture("python", true), "any");
    }

    #[test]
    fn compiled_rust_is_arch_any() {
        assert_eq!(detect_architecture("rust", false), "any");
    }

    #[test]
    fn has_compiled_detects_so() {
        // This test would need a temp dir with a .so file.
        // For now, test the function signature compiles.
        let _ = has_compiled_extensions;
    }
}
