// SPDX-License-Identifier: GPL-3.0-or-later
//! The Arch User Repository backend. Searches the AUR RPC, fetches a
//! package's PKGBUILD, and (for install) converts it to a temporary
//! `package.yaml` and builds via the shared build path — so AUR packages
//! become native `.deb`s on a Debian host.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::{Capabilities, PackageIndex};
use crate::index::{IndexHit, InstallOpts};
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
    fn capabilities(&self) -> Capabilities {
        Capabilities::READ
    }

    fn instance_name(&self) -> &str {
        &self.name
    }

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
                installed: crate::debs::dpkg_installed_version(&r.name).is_some(),
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
                installed: crate::debs::dpkg_installed_version(package).is_some(),
                available: vec!["build-from-pkgbuild".into()],
                ..Default::default()
            })),
            _ => Ok(None),
        }
    }

    fn install(&self, package: &str, opts: InstallOpts) -> Result<()> {
        if opts.tag.is_some() {
            bail!("AUR has no prebuilt tags; it builds from the PKGBUILD");
        }
        let pkgbuild = self.pkgbuild(package)?;
        let yaml = pkgbuild_to_yaml(package, &pkgbuild);
        if !opts.yes
            && !crate::debs::confirm(
                &format!("build AUR package '{package}' on this Debian host?"),
                false,
            )?
        {
            bail!("aborted");
        }
        println!("building AUR '{package}' from PKGBUILD …");
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
/// manual Debian-name mapping.
fn pkgbuild_to_yaml(package: &str, pkgbuild: &str) -> String {
    let field = |prefix: &str| -> String {
        pkgbuild
            .lines()
            .find_map(|l| l.trim().strip_prefix(prefix))
            .map(|v| v.trim().trim_matches(&['(', ')', '\'', '"'][..]).trim())
            .filter(|v| !v.is_empty())
            .unwrap_or("")
            .to_string()
    };
    let pkgver = field("pkgver=");
    let url = field("url=");
    let license = field("license=");
    let depends = bash_array(pkgbuild, "depends");
    let makedepends = bash_array(pkgbuild, "makedepends");

    let mut out = String::new();
    out.push_str(&format!(
        "# Converted from AUR PKGBUILD for '{package}' — REVIEW ME.\n"
    ));
    out.push_str(&format!("package_name: {package}\n"));
    if !url.is_empty() {
        out.push_str(&format!(
            "github_repo: OWNER/{package}   # guessed from AUR url: {url}\n"
        ));
    }
    if !pkgver.is_empty() {
        out.push_str(&format!("version: \"{pkgver}\"\n"));
    }
    if !license.is_empty() {
        out.push_str(&format!(
            "license_spdx: {license}   # AUR license field, verify SPDX\n"
        ));
    }
    if !depends.is_empty() {
        out.push_str(&format!(
            "# WARNING: Arch dependency names kept verbatim — map to Debian names:\ndepends: \"{}\"\n",
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

/// Extract a single-line bash array assignment (`name=(...)`) as a list,
/// stripping quotes and any version constraint. Multi-line arrays are not
/// handled — the simple form is the common case.
fn bash_array(pkgbuild: &str, name: &str) -> Vec<String> {
    let prefix = format!("{name}=(");
    let Some(rest) = pkgbuild
        .lines()
        .find_map(|l| l.trim().strip_prefix(&prefix))
    else {
        return Vec::new();
    };
    rest.split(&[')', '\'', '"'])
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
}
