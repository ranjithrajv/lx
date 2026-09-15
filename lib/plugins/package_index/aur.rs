// SPDX-License-Identifier: GPL-3.0-or-later
//! The Arch User Repository backend. Searches the AUR RPC, fetches a
//! package's PKGBUILD, and (for install) converts it to a temporary
//! `package.yaml` and builds via the shared build path — so an AUR package
//! becomes the host's own native package format.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::{PackageIndex, ReadIndex};
use crate::index::{IndexHit, InstallOpts, InstallOutcome};
use crate::plugins::plugin::plugin_identity;

const AUR_RPC: &str = "https://aur.archlinux.org/rpc/?v=5";

pub struct AurSource {
    name: String,
}

impl AurSource {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    fn rpc(&self, query: &str) -> Result<String> {
        let url = format!("{AUR_RPC}&{query}");
        let client = crate::lx_lib::http::new_client()?;
        let resp = client.get(&url).send().context("AUR RPC request failed")?;
        if !resp.status().is_success() {
            bail!("AUR HTTP {}", resp.status());
        }
        resp.text().context("reading AUR response")
    }

    fn pkgbuild(&self, package: &str) -> Result<String> {
        let url = format!("https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h={package}");
        let client = crate::lx_lib::http::new_client()?;
        let resp = client.get(&url).send().context("fetching PKGBUILD")?;
        if !resp.status().is_success() {
            bail!("AUR PKGBUILD HTTP {}", resp.status());
        }
        resp.text().context("reading PKGBUILD")
    }
}

/// Deserialize a field that may be an explicit JSON null as an empty string.
fn null_to_default<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(serde::Deserialize)]
struct RpcResult {
    #[serde(default, rename = "Name")]
    name: String,
    #[serde(default, rename = "Description", deserialize_with = "null_to_default")]
    description: String,
    #[serde(default, rename = "NumVotes")]
    votes: u64,
    #[serde(default, rename = "Maintainer", deserialize_with = "null_to_default")]
    maintainer: String,
}

#[derive(serde::Deserialize)]
struct RpcMulti {
    #[serde(default)]
    results: Vec<RpcResult>,
}

plugin_identity!(
    AurSource,
    "aur",
    "Arch User Repository (PKGBUILD → native build)"
);

impl PackageIndex for AurSource {
    fn instance_name(&self) -> &str {
        &self.name
    }
}

impl ReadIndex for AurSource {
    fn search(&self, pattern: Option<&str>) -> Result<Vec<IndexHit>> {
        let json = match pattern {
            Some(p) => self.rpc(&format!("type=search&arg={p}")),
            None => self.rpc("type=info&arg[]=__none__"),
        }?;
        let data: RpcMulti = serde_json::from_str(&json)?;
        let mut hits = Vec::new();
        for r in &data.results {
            if r.name.is_empty() {
                continue;
            }
            hits.push(IndexHit {
                name: r.name.clone(),
                description: format!(
                    "{} ({} votes, maintainer: {})",
                    r.description,
                    r.votes,
                    if r.maintainer.is_empty() {
                        "none"
                    } else {
                        &r.maintainer
                    }
                ),
                source: self.name.clone(),
                installed: crate::consumer::installed_version(
                    &r.name,
                    crate::index::detect_host_format(),
                )
                .is_some(),
                available: vec!["build-from-pkgbuild".into()],
                ..Default::default()
            });
        }
        Ok(hits)
    }

    fn info(&self, package: &str) -> Result<Option<IndexHit>> {
        let json = self.rpc(&format!("type=info&arg[]={package}"))?;
        let data: RpcMulti = serde_json::from_str(&json)?;
        let r = data.results.into_iter().next();
        match r {
            Some(r) if !r.name.is_empty() => Ok(Some(IndexHit {
                name: r.name,
                description: r.description,
                source: self.name.clone(),
                installed: crate::consumer::installed_version(
                    package,
                    crate::index::detect_host_format(),
                )
                .is_some(),
                available: vec!["build-from-pkgbuild".into()],
                ..Default::default()
            })),
            _ => Ok(None),
        }
    }

    fn install(&self, package: &str, opts: InstallOpts) -> Result<Option<InstallOutcome>> {
        if opts.tag.is_some() {
            bail!("AUR has no prebuilt tags; it builds from the PKGBUILD");
        }
        let pkgbuild = self.pkgbuild(package)?;
        // A recipe that can't name a real upstream repo can't be built:
        // fail with guidance before prompting, rather than downloading from
        // a literal `OWNER/` placeholder.
        if upstream_github_repo(&pkgbuild).is_none() {
            bail!(
                "could not determine an upstream GitHub repository for AUR package \
                 '{package}' from its PKGBUILD.\n  Scaffold a reviewed config with \
                 `lx init --from-aur {package}`, or install it with an AUR helper \
                 (e.g. `yay -S {package}`)."
            );
        }
        let yaml = pkgbuild_to_yaml(package, &pkgbuild);
        let host_dist = crate::info::detect_host_dist();
        if !opts.yes
            && !crate::debs::confirm(
                &format!("build AUR package '{package}' on this {host_dist} host?"),
                false,
            )?
        {
            bail!("aborted");
        }
        println!("building AUR '{package}' from PKGBUILD …");
        // The recipe build installs the artifact and records it, so the AUR
        // package joins the lx-managed lifecycle (list/upgrade/rollback).
        super::lx_community::build_from_recipe(&yaml, package, &opts)
    }

    fn update(&self) -> Result<bool> {
        // AUR is RPC-only, no local cache to refresh.
        Ok(false)
    }
}

/// Best-effort conversion of a PKGBUILD into a starter `package.yaml`.
/// The result is a hint — review it before trusting the build. Arch
/// dependency names are kept verbatim with a warning; the build may need
/// manual host-name mapping.
fn pkgbuild_to_yaml(package: &str, pkgbuild: &str) -> String {
    let pkgver = pkgbuild_field(pkgbuild, "pkgver=");
    let url = pkgbuild_field(pkgbuild, "url=");
    let license = pkgbuild_field(pkgbuild, "license=");
    let depends = bash_array(pkgbuild, "depends");
    let makedepends = bash_array(pkgbuild, "makedepends");
    let repo = upstream_github_repo(pkgbuild);

    let mut out = String::new();
    out.push_str(&format!(
        "# Converted from AUR PKGBUILD for '{package}' — REVIEW ME.\n"
    ));
    out.push_str(&format!("package_name: {package}\n"));
    match &repo {
        Some(repo) => out.push_str(&format!("github_repo: {repo}\n")),
        None => out.push_str(&format!(
            "# Could not determine an upstream GitHub repository from the PKGBUILD.\n\
             # Upstream url: {url}\n\
             # Scaffold a reviewed config with `lx init --from-aur {package}`.\n"
        )),
    }
    if !pkgver.is_empty() {
        out.push_str(&format!("version: \"{pkgver}\"\n"));
    }
    if !license.is_empty() {
        out.push_str(&format!(
            "license_spdx: {license}   # AUR license field, verify SPDX\n"
        ));
    }
    // Upstream release assets usually carry the platform/arch in the name
    // (e.g. `herdr-linux-x86_64`); install the binary as the command instead,
    // so the package provides `<package>` and can replace a `curl | sh` copy.
    out.push_str(&format!(
        "binary_rename: {package}   # install the single binary as the command name\n"
    ));
    if !depends.is_empty() {
        out.push_str(&format!(
            "# WARNING: Arch dependency names kept verbatim — map to host names:\ndepends: \"{}\"\n",
            depends.join(", ")
        ));
    }
    if !makedepends.is_empty() {
        out.push_str(&format!(
            "# WARNING: Arch makedepends kept verbatim — map to host-distro names:\nbuild_depends: [{}]\n",
            makedepends.join(", ")
        ));
    }
    out.push_str(
        "# This PKGBUILD compiles from source. Consider:\n#   build_mode: source\n#   build_system: cmake   # or: custom + build_commands/install_commands\n#   build_suites: [trixie, forky, sid]\n#   architectures: [<this-host-arch>]\n",
    );
    out
}

/// Read a single `key=value` (or `key=(...)`) assignment from a PKGBUILD,
/// trimming quotes and surrounding parentheses.
fn pkgbuild_field(pkgbuild: &str, prefix: &str) -> String {
    pkgbuild
        .lines()
        .find_map(|l| l.trim().strip_prefix(prefix))
        .map(|v| v.trim().trim_matches(&['(', ')', '\'', '"'][..]).trim())
        .filter(|v| !v.is_empty())
        .unwrap_or("")
        .to_string()
}

/// The upstream GitHub repository (`owner/repo`) a PKGBUILD builds from:
/// the first GitHub URL in `source=()`, else a GitHub `url=`.
pub(crate) fn upstream_github_repo(pkgbuild: &str) -> Option<String> {
    if let Some(body) = bash_array_body(pkgbuild, "source") {
        for token in body.split_whitespace() {
            if let Some(repo) = github_repo_from_url(token) {
                return Some(repo);
            }
        }
    }
    github_repo_from_url(&pkgbuild_field(pkgbuild, "url="))
}

/// Extract `owner/repo` from a GitHub URL (https, `git+https`, the bash
/// array `name::url` form, `.git` suffixes and `/archive/...` paths).
/// `None` when the string names no GitHub repository.
pub(crate) fn github_repo_from_url(url: &str) -> Option<String> {
    let url = url.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c.is_whitespace());
    // bash arrays may use the `name::url` form.
    let url = url.rsplit("::").next().unwrap_or(url);
    let url = url.strip_prefix("git+").unwrap_or(url);
    let idx = url.find("github.com/")?;
    // Reject lookalike hosts such as `gist.github.com` (the character before
    // the host must be a path separator).
    if !url[..idx].ends_with('/') {
        return None;
    }
    let rest = &url[idx + "github.com/".len()..];
    let mut parts = rest.split('/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");
    let repo = repo.trim_end_matches(".git");
    let repo = repo.split(['?', '#']).next().unwrap_or(repo);
    if !is_repo_component(owner) || !is_repo_component(repo) {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// True when `s` looks like a GitHub owner/repo path segment.
fn is_repo_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The raw body of a bash array assignment `name=( ... )`, spanning multiple
/// lines. Returns the text between `(` and the first closing `)`.
fn bash_array_body(pkgbuild: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=(");
    let lines: Vec<&str> = pkgbuild.lines().collect();
    let start = lines.iter().position(|l| l.trim().starts_with(&prefix))?;
    let mut body = String::new();
    for (i, line) in lines.iter().enumerate().skip(start) {
        let text: &str = if i == start {
            let t = line.trim();
            &t[prefix.len()..]
        } else {
            line
        };
        match text.find(')') {
            Some(end) => {
                body.push_str(&text[..end]);
                return Some(body);
            }
            None => {
                body.push_str(text);
                body.push('\n');
            }
        }
    }
    Some(body)
}

/// Extract a bash array assignment (`name=(...)`) as a list, stripping
/// quotes and any version constraint. Handles multi-line arrays.
fn bash_array(pkgbuild: &str, name: &str) -> Vec<String> {
    let Some(body) = bash_array_body(pkgbuild, name) else {
        return Vec::new();
    };
    body.split(&['\'', '"'][..])
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.split_once(['>', '=', '<'])
                .map(|(n, _)| n)
                .unwrap_or(v)
                .trim()
        })
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_array_strips_quotes_and_versions() {
        let pkgbuild = "depends=('glibc' 'openssl>=3.0' \"zlib\")\n";
        assert_eq!(
            bash_array(pkgbuild, "depends"),
            vec!["glibc", "openssl", "zlib"]
        );
        assert!(bash_array(pkgbuild, "makedepends").is_empty());
    }

    #[test]
    fn pkgbuild_to_yaml_emits_makedepends_as_build_depends() {
        let pkgbuild = "pkgver=1.2.3\nurl=https://example.com/x\nlicense=('MIT')\n\
                        depends=('libfoo')\nmakedepends=('cmake' 'ninja' 'git')\n\nbuild() {\n  true\n}\n";
        let yaml = pkgbuild_to_yaml("x", pkgbuild);
        assert!(yaml.contains("depends: \"libfoo\""), "{yaml}");
        assert!(
            yaml.contains("build_depends: [cmake, ninja, git]"),
            "{yaml}"
        );
        assert!(yaml.contains("version: \"1.2.3\""), "{yaml}");
    }

    #[test]
    fn pkgbuild_to_yaml_omits_empty_makedepends() {
        let yaml = pkgbuild_to_yaml("x", "pkgver=1.0\n");
        assert!(!yaml.contains("build_depends"), "{yaml}");
    }

    #[test]
    fn bash_array_handles_multiline_arrays() {
        let pkgbuild = "depends=(\n  'glibc'\n  'openssl>=3.0'\n)\n";
        assert_eq!(bash_array(pkgbuild, "depends"), vec!["glibc", "openssl"]);
    }

    #[test]
    fn github_repo_from_url_handles_common_forms() {
        assert_eq!(
            github_repo_from_url("https://github.com/ogulcancelik/herdr").as_deref(),
            Some("ogulcancelik/herdr")
        );
        assert_eq!(
            github_repo_from_url("git+https://github.com/ogulcancelik/herdr.git").as_deref(),
            Some("ogulcancelik/herdr")
        );
        assert_eq!(
            github_repo_from_url(
                "herdr-0.9.0.tar.gz::https://github.com/ogulcancelik/herdr/archive/v0.9.0.tar.gz"
            )
            .as_deref(),
            Some("ogulcancelik/herdr")
        );
        // Lookalike hosts and non-GitHub URLs must not match.
        assert_eq!(
            github_repo_from_url("https://gist.github.com/foo/bar"),
            None
        );
        assert_eq!(github_repo_from_url("https://herdr.dev"), None);
        assert_eq!(github_repo_from_url(""), None);
    }

    #[test]
    fn upstream_repo_prefers_source_over_url() {
        let pkgbuild = "url=https://herdr.dev\n\
                        source=(\"herdr-$pkgver.tar.gz::https://github.com/ogulcancelik/herdr/archive/v$pkgver.tar.gz\")\n";
        assert_eq!(
            upstream_github_repo(pkgbuild).as_deref(),
            Some("ogulcancelik/herdr")
        );
    }

    #[test]
    fn upstream_repo_falls_back_to_github_url() {
        let pkgbuild = "url=https://github.com/herdrdev/herdr\nsource=(\"local.tar.gz\")\n";
        assert_eq!(
            upstream_github_repo(pkgbuild).as_deref(),
            Some("herdrdev/herdr")
        );
    }

    #[test]
    fn pkgbuild_to_yaml_uses_real_repo_and_never_owner_placeholder() {
        let pkgbuild = "pkgver=0.9.0\nurl=https://herdr.dev\n\
                        source=(\"$pkgname-$pkgver.tar.gz::https://github.com/ogulcancelik/herdr/archive/v$pkgver.tar.gz\")\n";
        let yaml = pkgbuild_to_yaml("herdr", pkgbuild);
        assert!(yaml.contains("github_repo: ogulcancelik/herdr"), "{yaml}");
        assert!(!yaml.contains("OWNER/"), "{yaml}");
    }

    #[test]
    fn pkgbuild_to_yaml_omits_repo_when_unknown() {
        let pkgbuild = "pkgver=1.0\nurl=https://example.com/x\nsource=(\"local.tar.gz\")\n";
        let yaml = pkgbuild_to_yaml("x", pkgbuild);
        assert!(!yaml.contains("github_repo:"), "{yaml}");
        assert!(!yaml.contains("OWNER/"), "{yaml}");
    }

    #[test]
    fn pkgbuild_to_yaml_renames_the_binary_to_the_command() {
        let pkgbuild = "pkgver=0.9.0\n\
                        source=(\"h.tar.gz::https://github.com/ogulcancelik/herdr/archive/v$pkgver.tar.gz\")\n";
        let yaml = pkgbuild_to_yaml("herdr", pkgbuild);
        assert!(yaml.contains("binary_rename: herdr"), "{yaml}");
    }
}
