// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use clap::Args;
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct SchemaArgs {
    /// Write schema to file instead of stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

pub fn run(args: SchemaArgs) -> Result<()> {
    let schema = generate_schema();
    let pretty = serde_json::to_string_pretty(&schema)?;
    if let Some(path) = args.output {
        std::fs::write(&path, format!("{pretty}\n"))?;
        println!("Wrote JSON schema to {}", path.display());
    } else {
        println!("{pretty}");
    }
    Ok(())
}

pub fn generate_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://lx.goreleaser.com/schema.json",
        "title": "lx package.yaml",
        "description": "Configuration for lx — Latest Package Tool",
        "type": "object",
        "required": ["package_name", "github_repo"],
        "additionalProperties": false,
        "$defs": {
            "formatOverrides": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "depends": {"type": "string"},
                    "recommends": {"type": "string"},
                    "suggests": {"type": "string"},
                    "conflicts": {"type": "string"},
                    "replaces": {"type": "string"},
                    "provides": {"type": "string"},
                    "breaks": {"type": "string"},
                    "predepends": {"type": "string"}
                }
            }
        },
        "properties": {
            "package_name": {
                "type": "string",
                "description": "Name of the Debian package (required)"
            },
            "github_repo": {
                "type": "string",
                "description": "Source repo in owner/repo form",
                "pattern": ".+/.+"
            },
            "repo": {
                "type": "string",
                "description": "Alias for github_repo"
            },
            "gitlab_repo": {
                "type": "string",
                "description": "Alias for github_repo when source is gitlab"
            },
            "artifact_format": {
                "type": "string",
                "enum": ["tar.gz", "tgz", "zip", "raw"],
                "description": "Archive format of the release asset"
            },
            "description": {
                "type": "string",
                "description": "Human-readable package description (legacy alias: summary)"
            },
            "summary": {
                "type": "string",
                "description": "Legacy debian-multiarch-builder alias of description"
            },
            "license": {
                "type": "string",
                "description": "Legacy alias of license_spdx"
            },
            "vendor": {
                "type": "string",
                "description": "Legacy key recorded as a Vendor control field"
            },
            "dependencies": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Legacy list form of depends"
            },
            "download_pattern": {
                "type": "string",
                "description": "Legacy single asset pattern with {version}/{arch}/{package_name} placeholders; expanded into per-arch release patterns when no architectures block is present"
            },
            "architecture_map": {
                "type": "object",
                "additionalProperties": {"type": "string"},
                "description": "Legacy Debian-arch -> upstream-name map used to expand download_pattern"
            },
            "distribution_arch_overrides": {
                "type": "object",
                "description": "Per-architecture distribution allow-lists replacing the built-in support matrix for that arch",
                "additionalProperties": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "distributions": {
                            "type": "array",
                            "items": {"type": "string"}
                        }
                    }
                }
            },
            "max_parallel": {
                "type": "integer",
                "minimum": 0,
                "description": "Default max concurrent builds when --max-parallel is not passed (0 = auto-tune)"
            },
            "parallel_builds": {
                "type": "boolean",
                "description": "false pins the build to a single worker"
            },
            "build_mode": {
                "type": "string",
                "enum": ["binary", "source"],
                "description": "binary (default) repacks release assets; source compiles upstream on the host (cmake/custom)"
            },
            "build_system": {
                "type": "string",
                "enum": ["cmake", "cargo", "go", "meson", "autotools", "make", "custom"],
                "description": "Source-mode build system (cmake default; custom uses build_commands/install_commands)"
            },
            "upstream_url": {
                "type": "string",
                "description": "Source tarball URL root for build_mode: source (default: github archive)"
            },
            "upstream_ref": {
                "type": "string",
                "description": "Source-mode tag to fetch (default: resolved version)"
            },
            "build_depends": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Host packages the source compile needs, in host-distro names. Caller/CI installs them by default; `lx build --install-build-deps` installs the missing ones via the host package manager."
            },
            "cmake_flags": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Extra flags for the cmake configure step"
            },
            "prebuild_steps": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Shell steps run in the source dir after unpack, before configure"
            },
            "build_commands": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Custom build commands replacing cmake (build_system: custom)"
            },
            "install_commands": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Custom install commands into $DESTDIR (build_system: custom, required)"
            },
            "build_suites": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Source-mode suites to build (default: configured distributions)"
            },
            "skip_suites": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Source-mode suites to skip"
            },
            "debian_distributions": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Debian suites to target (default: bullseye, bookworm, trixie, forky, sid; expired-LTS suites are dropped automatically)"
            },
            "ubuntu_distributions": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Ubuntu suites to target natively (opt-in, e.g. jammy, noble)"
            },
            "architectures": {
                "description": "Per-arch release asset patterns or list restricting auto-discovery",
                "oneOf": [
                    {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "List of Debian architectures to restrict auto-discovery"
                    },
                    {
                        "type": "object",
                        "additionalProperties": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "release_pattern": {"type": "string"}
                            }
                        },
                        "description": "Map of arch -> {release_pattern}"
                    }
                ]
            },
            "binary_path": {
                "type": "string",
                "description": "Path to binary within extracted archive"
            },
            "binary_rename": {
                "type": "string",
                "description": "Rename the installed binary to this name"
            },
            "bundle": {
                "type": "boolean",
                "description": "Install whole binary_path tree under /usr/lib/<package_name>/ and symlink bin executables"
            },
            "depends": {
                "type": "string",
                "description": "Comma-separated Depends field"
            },
            "recommends": {
                "type": "string",
                "description": "Comma-separated Recommends field"
            },
            "suggests": {
                "type": "string",
                "description": "Comma-separated Suggests field"
            },
            "conflicts": {
                "type": "string",
                "description": "Comma-separated Conflicts field"
            },
            "replaces": {
                "type": "string",
                "description": "Comma-separated Replaces field"
            },
            "provides": {
                "type": "string",
                "description": "Comma-separated Provides field"
            },
            "breaks": {
                "type": "string",
                "description": "Comma-separated Breaks field"
            },
            "predepends": {
                "type": "string",
                "description": "Comma-separated Pre-Depends field"
            },
            "section": {
                "type": "string",
                "description": "Debian Section field (default: utils)"
            },
            "priority": {
                "type": "string",
                "enum": ["required", "important", "standard", "optional", "extra"],
                "description": "Debian Priority field (default: optional)"
            },
            "arch_variant": {
                "type": "string",
                "description": "Debian arch variant appended to Architecture (e.g. amd64v3). Mirrors nfpm deb.arch_variant."
            },
            "architecture": {
                "type": "string",
                "enum": ["auto", "all", "any"],
                "description": "Override package architecture (default: auto). all = pure code (arch-independent), any = compiled binary. Auto-detected from registry source."
            },
            "version_schema": {
                "type": "string",
                "enum": ["semver", "none"],
                "description": "Version parsing schema (default: semver). semver strips v-prefix and normalizes; none uses as-is. Mirrors nfpm version_schema."
            },
            "umask": {
                "type": "string",
                "description": "Octal umask applied to files without explicit mode (e.g. 0o002). Mirrors nfpm umask."
            },
            "packager": {
                "type": "string",
                "description": "Packager string (org/person packaging the software). RPM: packager header tag; deb: Packager control field. Falls back to maintainer. Mirrors nfpm rpm.packager."
            },
            "disable_globbing": {
                "type": "boolean",
                "description": "Disable glob expansion in contents: src patterns. Mirrors nfpm disable_globbing."
            },
            "fields": {
                "type": "object",
                "additionalProperties": {"type": "string"},
                "description": "Additional arbitrary control fields (e.g. Bugs, Homepage)"
            },
            "compression": {
                "type": "string",
                "description": "Compression for control.tar and data.tar (default: gzip)",
                "enum": ["gzip", "gz", "xz", "zstd", "zst", "none"],
                "examples": ["gzip", "xz", "zstd", "none", "gzip:9"]
            },
            "contents": {
                "type": "array",
                "description": "Extra files/dirs/symlinks layered into the staged install tree (relative src resolves against the working directory)",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["dst"],
                    "properties": {
                        "src": {
                            "type": "string",
                            "description": "Source path in the build environment; not required for type dir/ghost"
                        },
                        "dst": {
                            "type": "string",
                            "pattern": "^/",
                            "description": "Absolute destination path inside the package"
                        },
                        "type": {
                            "type": "string",
                            "enum": [
                                "file",
                                "config",
                                "config|noreplace",
                                "config|missingok",
                                "tree",
                                "symlink",
                                "dir",
                                "ghost"
                            ],
                            "description": "Entry type (default file); config* also registers a deb conffile; ghost is RPM-only"
                        },
                        "packager": {
                            "type": "string",
                            "enum": ["deb", "rpm", "arch"],
                            "description": "If set, only apply this entry when building that format (nfpm parity)"
                        }
                    }
                }
            },
            "overrides": {
                "type": "object",
                "description": "Per-format relation-field overrides; an overridden field replaces the top-level value (empty string clears it)",
                "additionalProperties": false,
                "propertyNames": {"enum": ["deb", "rpm", "arch"]},
                "additionalProperties": false,
                "properties": {
                    "deb": {"$ref": "#/$defs/formatOverrides"},
                    "rpm": {"$ref": "#/$defs/formatOverrides"},
                    "arch": {"$ref": "#/$defs/formatOverrides"}
                }
            },
            "scripts": {
                "type": "object",
                "additionalProperties": false,
                "description": "Maintainer scripts from the build environment. deb: preinstall→DEBIAN/preinst, postinstall→DEBIAN/postinst, preremove→DEBIAN/prerm, postremove→DEBIAN/postrm, preupgrade_script→DEBIAN/preupgrade, postupgrade_script→DEBIAN/postupgrade (all mode 0755). rpm: preinstall→%pre, postinstall→%post, preremove→%preun, postremove→%postun, pretrans→%pretrans (also pre-upgrade), posttrans→%posttrans (also post-upgrade), verify→%verify. arch: preupgrade→pre_upgrade(), postupgrade→post_upgrade() in .INSTALL.",
                "properties": {
                    "preinstall": {"type": "string", "description": "preinstall script path (deb: DEBIAN/preinst; rpm: %pre)"},
                    "postinstall": {"type": "string", "description": "postinstall script path (deb: DEBIAN/postinst; rpm: %post)"},
                    "preremove": {"type": "string", "description": "preremove script path (deb: DEBIAN/prerm; rpm: %preun)"},
                    "postremove": {"type": "string", "description": "postremove script path (deb: DEBIAN/postrm; rpm: %postun)"},
                    "pretrans": {"type": "string", "description": "RPM %pretrans transaction script path (ignored by deb/arch)"},
                    "posttrans": {"type": "string", "description": "RPM %posttrans transaction script path (ignored by deb/arch)"},
                    "verify": {"type": "string", "description": "RPM %verify script path (ignored by deb/arch)"},
                    "preupgrade": {"type": "string", "description": "Arch pre_upgrade() hook path (ignored by deb/rpm)"},
                    "postupgrade": {"type": "string", "description": "Arch post_upgrade() hook path (ignored by deb/rpm)"},
                    "preupgrade_script": {"type": "string", "description": "Pre-upgrade script path (deb: DEBIAN/preupgrade; rpm: %pretrans; arch: pre_upgrade())"},
                    "postupgrade_script": {"type": "string", "description": "Post-upgrade script path (deb: DEBIAN/postupgrade; rpm: %posttrans; arch: post_upgrade())"}
                }
            },
            "rpm": {
                "type": "object",
                "additionalProperties": false,
                "description": "RPM-specific configuration: triggers. Mirrors fpm's --rpm-trigger-* flags. Ignored by deb/arch.",
                "properties": {
                    "trigger_pre_install": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers before install (%triggerprein). Each entry is 'package: script_path'."
                    },
                    "trigger_post_install": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers after install (%triggerin). Each entry is 'package: script_path'."
                    },
                    "trigger_pre_uninstall": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers before uninstall (%triggerun). Each entry is 'package: script_path'."
                    },
                    "trigger_post_uninstall": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers after uninstall (%triggerpostun). Each entry is 'package: script_path'."
                    }
                }
            },
            "deb": {
                "type": "object",
                "additionalProperties": false,
                "description": "Debian-specific configuration: debconf templates/config, maintainer triggers, and rules. Ignored by rpm/arch.",
                "properties": {
                    "rules": {"type": "string", "description": "Path to debian/rules Makefile (copied as DEBIAN/rules, mode 0755)"},
                    "templates": {"type": "string", "description": "Path to debconf templates file (copied as DEBIAN/templates, mode 0644)"},
                    "config": {"type": "string", "description": "Path to debconf config script (copied as DEBIAN/config, mode 0755)"},
                    "triggers_interest": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers this package registers interest in (interest lines in DEBIAN/triggers)"
                    },
                    "triggers_activate": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers this package activates (activate lines in DEBIAN/triggers)"
                    },
                    "triggers_interest_await": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers this package registers interest in, waiting for the trigger (interest_await lines in DEBIAN/triggers)"
                    },
                    "triggers_interest_noawait": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers this package registers interest in, without waiting (interest_noawait lines in DEBIAN/triggers)"
                    },
                    "triggers_activate_await": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers this package activates, waiting for the trigger (activate_await lines in DEBIAN/triggers)"
                    },
                    "triggers_activate_noawait": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Triggers this package activates, without waiting (activate_noawait lines in DEBIAN/triggers)"
                    }
                }
            },
            "signature": {
                "type": "object",
                "additionalProperties": false,
                "description": "Package signing. deb detach (default): sibling <pkg>.sig; deb debsign: embedded _gpgorigin; rpm: PGP embedded natively. Passphrase via $LX_SIGN_PASSPHRASE or $NFPM_PASSPHRASE. String fields expand ${VAR} / ${VAR:-default}.",
                "properties": {
                    "key_file": {
                        "type": "string",
                        "description": "Path to an ASCII-armored secret key (env-expandable)"
                    },
                    "key_id": {
                        "type": "string",
                        "description": "Optional key id / fingerprint (gpg --local-user)"
                    },
                    "method": {
                        "type": "string",
                        "enum": ["detach", "debsign"],
                        "description": "Deb signing method (default detach). Ignored for rpm/arch."
                    },
                    "type": {
                        "type": "string",
                        "enum": ["origin", "maint", "archive"],
                        "description": "Debsign role → ar member _gpg{type} (default origin). Ignored unless method is debsign."
                    }
                }
            },
            "local_payload": {
                "type": "string",
                "description": "Local archive or directory for --local builds (skips upstream download; existence checked at build time; env-expandable)"
            },
            "license_spdx": {
                "type": "string",
                "description": "SPDX license identifier"
            },
            "maintainer": {
                "type": "string",
                "description": "Maintainer string (e.g. Name <email>)"
            },
            "version": {
                "type": "string",
                "description": "Pin a specific upstream version (default: latest)"
            },
            "build_version": {
                "type": "string",
                "description": "Debian revision (default: 1)"
            },
            "epoch": {
                "type": "string",
                "description": "Debian epoch for version resets"
            },
            "package_format": {
                "type": "string",
                "enum": ["deb", "rpm", "arch", "apk", "ipk"],
                "description": "Package format plugin (default: deb)"
            },
            "source": {
                "type": "string",
                "enum": ["github", "gitlab", "gitea", "forgejo", "bitbucket", "gerrit", "gitee", "sourceforge", "custom"],
                "description": "Source provider for auto-discovery (default: github)"
            },
            "source_provider": {
                "type": "string",
                "description": "Alias for source"
            },
            "registry_source": {
                "type": "string",
                "enum": ["npm", "python", "gem", "cargo", "go", "hex", "dart", "nuget", "maven", "composer"],
                "description": "Registry source plugin for language package managers (npm, python, gem, cargo, nuget, maven, composer)"
            },
            "gitlab_host": {
                "type": "string",
                "description": "Self-hosted GitLab host"
            },
            "gitea_host": {
                "type": "string",
                "description": "Self-hosted Gitea host"
            },
            "forgejo_host": {
                "type": "string",
                "description": "Self-hosted Forgejo host"
            },
            "bitbucket_host": {
                "type": "string",
                "description": "Self-hosted Bitbucket host"
            },
            "gerrit_host": {
                "type": "string",
                "description": "Gerrit host (e.g. review.gerrithub.io or a self-hosted instance); falls back to $GERRIT_HOST"
            },
            "gitee_host": {
                "type": "string",
                "description": "Gitee host (default gitee.com); only used when source = gitee"
            },
            "template_scripts": {
                "type": "boolean",
                "description": "Enable ERB-like templating for maintainer scripts. When true, <%= name %>, <%= version %>, <%= maintainer %>, <%= description %>, <%= homepage %>, <%= license %>, <%= arch %>, <%= dist %>, <%= iteration %>, <%= epoch %>, <%= vendor %>, <%= packager %>, <%= prefix %> expressions are replaced at build time. Mirrors fpm --template-scripts."
            }
        }
    })
}
