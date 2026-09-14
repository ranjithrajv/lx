// SPDX-License-Identifier: GPL-3.0-or-later

//! The Repology backend — cross-distro package metadata.
//!
//! Repology tracks what version of each project ships in 200+ distro
//! repositories. This source is **metadata-only**: it can tell you that
//! eza 0.18 is in Debian 12 and 0.20 is in Arch, but it cannot install
//! anything. Install is rejected with a prompt to use a recipe source
//! (lx-community, aur) instead.
//!
//! Data comes from the repology API (`/api/v1/project/{name}`) and is
//! cached as JSON under `~/.cache/lx/repology/`. The `update` command
//! fetches multiple pages (see `PAGES_TO_FETCH`) with a 1.1 s delay between
//! requests to respect repology's 1 req/s rate limit.

use anyhow::{bail, Context, Result};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{PackageIndex, ReadIndex};
use crate::debs::{detect_dist, detect_dpkg_arch};
use crate::index::{IndexHit, InstallOpts};
use crate::plugins::plugin::plugin_identity;

const API_BASE: &str = "https://repology.org/api/v1";
const PROJECTS_PAGE: &str = "projects/";
const STALE_HOURS: u64 = 24;
/// Number of pages to fetch on update. Repology returns ~200 projects per
/// page; 5 pages ≈ 1000 projects. At 1 req/s this adds ~5 s to `lx index update`.
const PAGES_TO_FETCH: usize = 5;
/// Delay between paginated requests to respect repology's 1 req/s rate limit.
const PAGE_FETCH_DELAY_MS: u64 = 1100;

pub struct RepologySource {
    name: String,
    cache: PathBuf,
}

/// One package entry as returned by repology's /api/v1/project/{name}.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepologyPackage {
    pub repo: String,
    #[serde(default)]
    pub srcname: Option<String>,
    #[serde(default)]
    pub binname: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    #[serde(default)]
    pub licenses: Option<Vec<String>>,
    #[serde(default)]
    pub maintainers: Option<Vec<String>>,
}

/// Cached project: its packages + when we last refreshed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepologyProject {
    pub packages: Vec<RepologyPackage>,
    #[serde(default)]
    pub last_updated: u64,
}

impl RepologySource {
    pub fn new(name: &str) -> Self {
        let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            name: name.to_string(),
            cache: base.join("lx").join("repology"),
        }
    }

    /// Path to the bulk cache (first-page projects).
    fn cache_index(&self) -> PathBuf {
        self.cache.join("projects.json")
    }

    /// Path to a single project's cached data.
    fn project_cache(&self, name: &str) -> PathBuf {
        // Namespace under a `per-project/` subdir to avoid clashing with
        // the bulk index filename.
        self.cache.join("per-project").join(format!("{name}.json"))
    }

    /// Ensure the cache directory exists.
    fn ensure_cache_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.cache)?;
        std::fs::create_dir_all(self.cache.join("per-project"))?;
        Ok(())
    }

    /// True if the bulk cache is missing or older than STALE_HOURS.
    pub fn stale(&self) -> bool {
        match self.cache_index().metadata() {
            Ok(m) => match m.modified() {
                Ok(t) => t
                    .elapsed()
                    .map(|d| d.as_secs() / 3600 >= STALE_HOURS)
                    .unwrap_or(true),
                Err(_) => true,
            },
            Err(_) => true,
        }
    }

    /// Load the bulk project cache.
    pub fn load_cache(&self) -> Result<BTreeMap<String, RepologyProject>> {
        let path = self.cache_index();
        if !path.is_file() {
            return Ok(BTreeMap::new());
        }
        let text = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&text).unwrap_or_default())
    }

    /// Save the bulk project cache.
    fn save_cache(&self, projects: &BTreeMap<String, RepologyProject>) -> Result<()> {
        self.ensure_cache_dir()?;
        let path = self.cache_index();
        let json = serde_json::to_string_pretty(projects)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// Load a single project from its per-project cache.
    fn load_project(&self, name: &str) -> Option<RepologyProject> {
        let path = self.project_cache(name);
        if !path.is_file() {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str::<RepologyProject>(&text).ok()
    }

    /// Save a single project to its per-project cache.
    fn save_project(&self, name: &str, project: &RepologyProject) -> Result<()> {
        self.ensure_cache_dir()?;
        let path = self.project_cache(name);
        let json = serde_json::to_string_pretty(project)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// GET a repology API path, returning the raw body.
    fn api_get(&self, path: &str) -> Result<String> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{API_BASE}/{path}")
        };
        let client = crate::http::new_client()?;
        let resp = client
            .get(&url)
            .header(
                "User-Agent",
                format!(
                    "lx/{} (https://github.com/ranjithrajv/lx)",
                    env!("CARGO_PKG_VERSION")
                ),
            )
            .send()
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            bail!("repology API HTTP {} for {url}", resp.status());
        }
        resp.text().context("reading repology response")
    }

    /// Fetch a single project's packages from the API and cache it.
    pub fn fetch_project(&self, name: &str) -> Result<Option<RepologyProject>> {
        let body = self.api_get(&format!("project/{name}"))?;
        // repology returns a JSON array of packages for this project
        let packages: Vec<RepologyPackage> = serde_json::from_str(&body)
            .with_context(|| format!("parsing repology project '{name}'"))?;
        if packages.is_empty() {
            return Ok(None);
        }
        let project = RepologyProject {
            packages,
            last_updated: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        self.save_project(name, &project)?;
        Ok(Some(project))
    }

    /// Search the local cache for projects matching the pattern. Falls back
    /// to per-project API lookups for names not in the bulk cache.
    pub fn search_cached(&self, pattern: &Regex) -> Result<Vec<(String, RepologyProject)>> {
        let cache = self.load_cache()?;
        let mut results = Vec::new();
        for (name, project) in &cache {
            if pattern.is_match(name) {
                results.push((name.clone(), project.clone()));
                continue;
            }
            // Also match against summaries and srcnames.
            for pkg in &project.packages {
                let haystack = format!(
                    "{} {} {}",
                    name,
                    pkg.summary.as_deref().unwrap_or(""),
                    pkg.srcname.as_deref().unwrap_or("")
                );
                if pattern.is_match(&haystack) {
                    results.push((name.clone(), project.clone()));
                    break;
                }
            }
        }
        Ok(results)
    }

    /// Look up a single project: check cache first, then API.
    pub fn lookup_project(&self, name: &str) -> Result<Option<RepologyProject>> {
        if let Some(p) = self.load_project(name) {
            // Also check the bulk cache (may have summary info)
            let cache = self.load_cache()?;
            if let Some(bulk) = cache.get(name) {
                return Ok(Some(bulk.clone()));
            }
            return Ok(Some(p));
        }
        // Not cached — try the API.
        self.fetch_project(name)
    }

    /// Compute distro summary for a project: newest version, repo count,
    /// outdated count, vulnerable count.
    pub fn distro_summary(project: &RepologyProject) -> (Option<String>, usize, usize, usize) {
        let mut newest: Option<String> = None;
        let mut repos = 0;
        let mut outdated = 0;
        let mut vulnerable = 0;
        for pkg in &project.packages {
            repos += 1;
            match pkg.status.as_str() {
                "newest" | "devel" | "unique" | "rolling" => {
                    if newest.is_none() {
                        newest = Some(pkg.version.clone());
                    }
                }
                "outdated" | "legacy" => outdated += 1,
                "vulnerable" => vulnerable += 1,
                _ => {}
            }
        }
        (newest, repos, outdated, vulnerable)
    }

    /// Determine the repology repo name for the current host distro, e.g.
    /// "debian_12", "ubuntu_24.04", "arch", "fedora_42".
    pub fn host_repo_name() -> Option<String> {
        let arch = detect_dpkg_arch().ok()?;
        match arch.as_str() {
            // Arch doesn't use versioned repos in repology.
            _ if std::process::Command::new("pacman")
                .arg("--version")
                .output()
                .is_ok() =>
            {
                Some("arch".to_string())
            }
            _ => {
                let dist = detect_dist().unwrap_or_default();
                if dist.is_empty() {
                    return None;
                }
                // Try to detect the family from /etc/os-release.
                let os_release = std::fs::read_to_string("/etc/os-release").ok()?;
                let id = os_release
                    .lines()
                    .find_map(|l| l.strip_prefix("ID="))
                    .map(|v| v.trim_matches('"'))
                    .unwrap_or("");
                let version_id = os_release
                    .lines()
                    .find_map(|l| l.strip_prefix("VERSION_ID="))
                    .map(|v| v.trim_matches('"'))
                    .unwrap_or("");
                // Map to repology repo naming.
                let repo = match id {
                    "debian" => format!("debian_{dist}"),
                    "ubuntu" => format!("ubuntu_{}", version_id),
                    "fedora" => format!("fedora_{}", version_id),
                    _ => format!("{id}_{dist}"),
                };
                Some(repo)
            }
        }
    }

    /// Get the host distro's version of a project, if tracked.
    pub fn host_version(project: &RepologyProject) -> Option<(String, String)> {
        let host_repo = Self::host_repo_name()?;
        project
            .packages
            .iter()
            .find(|p| p.repo == host_repo)
            .map(|p| (p.version.clone(), p.status.clone()))
    }

    /// Count how many packages are outdated on the host distro.
    pub fn count_outdated(
        projects: &BTreeMap<String, RepologyProject>,
    ) -> Vec<(String, String, String)> {
        // (name, host_version, newest_version)
        let host_repo = match Self::host_repo_name() {
            Some(r) => r,
            None => return Vec::new(),
        };
        let mut outdated = Vec::new();
        for (name, project) in projects {
            let host = project.packages.iter().find(|p| p.repo == host_repo);
            let newest = project
                .packages
                .iter()
                .find(|p| p.status == "newest" || p.status == "devel");
            if let (Some(h), Some(n)) = (host, newest) {
                if (h.status == "outdated" || h.status == "legacy") && h.version != n.version {
                    outdated.push((name.clone(), h.version.clone(), n.version.clone()));
                }
            }
        }
        outdated
    }

    /// Return a list of packages where the host distro lags upstream, as
    /// IndexHit structs suitable for display by `lx upgrade --all`.
    pub fn outdated_packages(&self) -> Vec<IndexHit> {
        let projects = match self.load_cache() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
        let outdated = Self::count_outdated(&projects);
        outdated
            .into_iter()
            .map(|(name, host_version, newest)| IndexHit {
                name: name.clone(),
                source: "repology".to_string(),
                host_version: Some(host_version),
                newest: Some(newest),
                installed: crate::debs::dpkg_installed_version(&name).is_some(),
                ..Default::default()
            })
            .collect()
    }
}

plugin_identity!(
    RepologySource,
    "repology",
    "Repology cross-distro metadata (read-only)"
);

impl PackageIndex for RepologySource {
    fn instance_name(&self) -> &str {
        &self.name
    }
}

impl ReadIndex for RepologySource {
    fn search(&self, pattern: Option<&str>) -> Result<Vec<IndexHit>> {
        let re = match pattern {
            Some(p) => Some(Regex::new(&format!("(?i){p}"))?),
            None => None,
        };

        let mut hits = Vec::new();

        // Search the bulk cache first.
        if let Some(ref re) = re {
            let results = self.search_cached(re)?;
            for (name, project) in results {
                let (newest, repos, outdated, vulnerable) = Self::distro_summary(&project);
                let host = Self::host_version(&project);
                let desc = project
                    .packages
                    .first()
                    .and_then(|p| p.summary.clone())
                    .unwrap_or_default();
                let installed = crate::debs::dpkg_installed_version(&name).is_some();
                hits.push(IndexHit {
                    name,
                    description: desc,
                    source: self.name.clone(),
                    installed,
                    available: newest.clone().map(|v| vec![v]).unwrap_or_default(),
                    newest,
                    repos,
                    outdated,
                    vulnerable,
                    host_version: host.clone().map(|(v, _)| v),
                    host_status: host.map(|(_, s)| s),
                });
            }
        } else {
            // No pattern — return everything in the cache.
            let cache = self.load_cache()?;
            for (name, project) in &cache {
                let (newest, repos, outdated, vulnerable) = Self::distro_summary(project);
                let host = Self::host_version(project);
                let desc = project
                    .packages
                    .first()
                    .and_then(|p| p.summary.clone())
                    .unwrap_or_default();
                let installed = crate::debs::dpkg_installed_version(name).is_some();
                hits.push(IndexHit {
                    name: name.clone(),
                    description: desc,
                    source: self.name.clone(),
                    installed,
                    available: newest.clone().map(|v| vec![v]).unwrap_or_default(),
                    newest: newest.clone(),
                    repos,
                    outdated,
                    vulnerable,
                    host_version: host.clone().map(|(v, _)| v),
                    host_status: host.map(|(_, s)| s),
                });
            }
        }

        Ok(hits)
    }

    fn info(&self, package: &str) -> Result<Option<IndexHit>> {
        let project = match self.lookup_project(package)? {
            Some(p) => p,
            None => return Ok(None),
        };
        let (newest, repos, outdated, vulnerable) = Self::distro_summary(&project);
        let host = Self::host_version(&project);
        let desc = project
            .packages
            .first()
            .and_then(|p| p.summary.clone())
            .unwrap_or_default();
        let installed = crate::debs::dpkg_installed_version(package).is_some();
        Ok(Some(IndexHit {
            name: package.to_string(),
            description: desc,
            source: self.name.clone(),
            installed,
            available: newest.clone().map(|v| vec![v]).unwrap_or_default(),
            newest,
            repos,
            outdated,
            vulnerable,
            host_version: host.clone().map(|(v, _)| v),
            host_status: host.map(|(_, s)| s),
        }))
    }

    fn install(&self, package: &str, _opts: InstallOpts) -> Result<()> {
        // Repology is metadata-only. Refuse install and point the user
        // to recipe sources that can actually provide a binary.
        bail!(
            "repology is a metadata-only index and cannot install '{package}'. \
             Try `lx index install {package}` (searches recipe sources), \
             or `lx install {package}` if a prebuilt exists."
        )
    }

    fn update(&self) -> Result<bool> {
        // Fetch multiple pages of projects from repology's bulk API,
        // paginating by the last project name of each page. Respects the
        // 1 req/s rate limit with a delay between requests.
        let mut projects: BTreeMap<String, RepologyProject> = BTreeMap::new();
        let mut page_url = Some(PROJECTS_PAGE.to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for page_num in 0..PAGES_TO_FETCH {
            let url = match &page_url {
                Some(u) => u.clone(),
                None => break,
            };
            let body = self.api_get(&url)?;
            let raw: serde_json::Value = serde_json::from_str(&body)?;

            let obj = raw
                .as_object()
                .context("repology projects response is not an object")?;

            if obj.is_empty() {
                break; // No more pages.
            }

            let mut last_name = String::new();
            for (name, packages_raw) in obj {
                last_name = name.clone();
                if let Ok(packages) =
                    serde_json::from_value::<Vec<RepologyPackage>>(packages_raw.clone())
                {
                    if !packages.is_empty() {
                        projects.insert(
                            name.clone(),
                            RepologyProject {
                                packages,
                                last_updated: now,
                            },
                        );
                    }
                }
            }

            // Next page starts after the last project name (inclusive).
            // Repology's convention: /api/v1/projects/{last_name}/ returns the
            // page starting at that name.
            if page_num < PAGES_TO_FETCH - 1 {
                page_url = Some(format!("projects/{last_name}/"));
                std::thread::sleep(std::time::Duration::from_millis(PAGE_FETCH_DELAY_MS));
            }
        }

        let was_empty = self.load_cache()?.is_empty();
        self.save_cache(&projects)?;

        if projects.is_empty() {
            // Nothing new AND nothing cached — treat as no update.
            return Ok(!was_empty);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_repo_name_detects_arch() {
        // On a non-Arch host this returns a debian_* or ubuntu_* value;
        // on Arch it returns "arch". Just verify it doesn't panic.
        let _ = RepologySource::host_repo_name();
    }

    #[test]
    fn distro_summary_counts_statuses() {
        let project = RepologyProject {
            packages: vec![
                RepologyPackage {
                    repo: "arch".into(),
                    srcname: None,
                    binname: None,
                    version: "1.0".into(),
                    status: "newest".into(),
                    summary: Some("test".into()),
                    categories: None,
                    licenses: None,
                    maintainers: None,
                },
                RepologyPackage {
                    repo: "debian_12".into(),
                    srcname: None,
                    binname: None,
                    version: "0.9".into(),
                    status: "outdated".into(),
                    summary: None,
                    categories: None,
                    licenses: None,
                    maintainers: None,
                },
                RepologyPackage {
                    repo: "fedora_40".into(),
                    srcname: None,
                    binname: None,
                    version: "0.8".into(),
                    status: "vulnerable".into(),
                    summary: None,
                    categories: None,
                    licenses: None,
                    maintainers: None,
                },
            ],
            last_updated: 0,
        };
        let (newest, repos, outdated, vulnerable) = RepologySource::distro_summary(&project);
        assert_eq!(newest.as_deref(), Some("1.0"));
        assert_eq!(repos, 3);
        assert_eq!(outdated, 1);
        assert_eq!(vulnerable, 1);
    }
}
