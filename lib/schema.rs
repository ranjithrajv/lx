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
        "$id": "https://lpt.goreleaser.com/schema.json",
        "title": "lpt package.yaml",
        "description": "Configuration for lpt — Latest Package Tool",
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
            "debian_distributions": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Debian suites to target (default: bullseye, bookworm, trixie, forky, sid; expired-LTS suites are dropped automatically)"
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
                "description": "Maintainer scripts from the build environment (deb: DEBIAN/{preinst,postinst,prerm,postrm}; rpm: %pre/%post/%preun/%postun)",
                "properties": {
                    "preinstall": {"type": "string"},
                    "postinstall": {"type": "string"},
                    "preremove": {"type": "string"},
                    "postremove": {"type": "string"}
                }
            },
            "signature": {
                "type": "object",
                "additionalProperties": false,
                "description": "Package signing. deb: detached <pkg>.sig via gpg; rpm: PGP signature embedded natively. Passphrase via $LPT_SIGN_PASSPHRASE or $NFPM_PASSPHRASE.",
                "properties": {
                    "key_file": {
                        "type": "string",
                        "description": "Path to an ASCII-armored secret key"
                    },
                    "key_id": {
                        "type": "string",
                        "description": "Optional key id / fingerprint (gpg --local-user)"
                    }
                }
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
                "enum": ["deb", "rpm", "arch"],
                "description": "Package format plugin (default: deb)"
            },
            "source": {
                "type": "string",
                "enum": ["github", "github-sync", "gitlab", "gitea", "forgejo", "bitbucket", "gerrit"],
                "description": "Source provider for auto-discovery (default: github)"
            },
            "source_provider": {
                "type": "string",
                "description": "Alias for source"
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
            }
        }
    })
}
