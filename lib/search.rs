//! `lx search` — deb-get `search` parity plus `apt search` behavior:
//! regex over the curated index, full-text match against installed
//! packages' dpkg descriptions, installed/candidate versions per hit, and
//! exact-name matches sorted first.

use anyhow::{Context, Result};
use clap::Args;
use regex::Regex;

use crate::debs;
use crate::manifest::Manifest;

/// Search packages published under the latest-debs GitHub org
/// (deb-get `search` parity: regex over the curated index, with installed
/// state). The org's `<package>-debian` repos are the index; the `-debian`
/// suffix is stripped for display so `lx search eza` matches `eza-debian`.
#[derive(Debug, Clone, Args)]
pub struct SearchArgs {
    /// Regex matched (case-insensitively) against package name and
    /// description — including installed packages' full dpkg descriptions,
    /// like `apt search` (`--raw` prints names only). No pattern lists
    /// everything.
    pub pattern: Option<String>,

    /// Only list packages installed (per dpkg, like `deb-get list --installed`).
    #[arg(long, conflicts_with = "not_installed")]
    pub installed: bool,

    /// Only list packages not installed.
    #[arg(long, conflicts_with = "installed")]
    pub not_installed: bool,

    /// Print bare package names, one per line (for scripting).
    #[arg(long)]
    pub raw: bool,

    /// Search the bundled recipe index (embedded templates/) instead of
    /// the latest-debs org over the network. Offline; useful for
    /// bootstrapping new packages from starter configs.
    #[arg(long)]
    pub local: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct OrgRepo {
    name: String,
    description: Option<String>,
}

fn org_repos(org: &str, token: Option<&str>) -> Result<Vec<OrgRepo>> {
    let client = crate::http::new_client()?;
    let auth: Option<(&'static str, String)> = token
        .map(str::to_string)
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .filter(|t| !t.trim().is_empty())
        .map(|t| ("Authorization", format!("Bearer {t}")));
    let headers: Vec<(&'static str, String)> = {
        let mut h = vec![("Accept", "application/vnd.github+json".to_string())];
        h.extend(auth.clone());
        h
    };
    let mut out = Vec::new();
    let mut page = 1u32;
    loop {
        let url =
            format!("https://api.github.com/orgs/{org}/repos?per_page=100&page={page}&type=all");
        let resp = crate::http::send_get_with_retry_headers(&client, &url, &headers)
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            anyhow::bail!("GitHub API {} listing {org} repos", resp.status());
        }
        let batch: Vec<OrgRepo> = resp.json().context("failed to parse org repos JSON")?;
        let done = batch.len() < 100;
        out.extend(batch);
        if done {
            break;
        }
        page += 1;
    }
    Ok(out)
}

/// One search hit: display name, blurb, install state, and versions.
/// `candidate` is the manifest-recorded version when lx manages the
/// package (per-package latest-release lookup would cost one API call per
/// hit, so the index itself carries no versions).
struct Hit {
    name: String,
    desc: String,
    installed: bool,
    installed_version: Option<String>,
    candidate: Option<String>,
}

fn print_hits(hits: &[Hit], raw: bool, empty_msg: &str, pattern: Option<&str>) {
    if hits.is_empty() {
        println!("{empty_msg}");
        return;
    }
    // apt search sorts exact-name matches first, then alphabetically.
    let mut order: Vec<usize> = (0..hits.len()).collect();
    order.sort_by(|&a, &b| {
        let ea = pattern.is_some_and(|p| hits[a].name.eq_ignore_ascii_case(p));
        let eb = pattern.is_some_and(|p| hits[b].name.eq_ignore_ascii_case(p));
        eb.cmp(&ea).then_with(|| hits[a].name.cmp(&hits[b].name))
    });
    if raw {
        for &i in &order {
            println!("{}", hits[i].name);
        }
        return;
    }
    let pad = order.iter().map(|&i| hits[i].name.len()).max().unwrap_or(0);
    for &i in &order {
        let h = &hits[i];
        let mut tags = Vec::new();
        if let Some(v) = &h.installed_version {
            tags.push(format!("installed {v}"));
        } else if h.installed {
            tags.push("installed".to_string());
        }
        if let Some(c) = &h.candidate {
            // Skip echoing the candidate when it equals the installed one.
            if h.installed_version.as_ref() != Some(c) {
                tags.push(format!("candidate {c}"));
            }
        }
        let tag = if tags.is_empty() {
            String::new()
        } else {
            format!(" [ {} ]", tags.join(", "))
        };
        // Long dpkg descriptions collapse to their first line for the list.
        let blurb: String = h.desc.lines().next().unwrap_or_default().to_string();
        if blurb.is_empty() {
            println!("{:<pad$}{tag}", h.name);
        } else {
            println!("{:<pad$}  {blurb}{tag}", h.name);
        }
    }
}

pub fn run(args: SearchArgs, token: Option<&str>) -> Result<()> {
    let re = match &args.pattern {
        Some(p) => {
            Some(Regex::new(&format!("(?i){p}")).with_context(|| format!("invalid regex '{p}'"))?)
        }
        None => None,
    };

    let manifest = Manifest::load().unwrap_or_default();
    let mut hits: Vec<Hit> = Vec::new();

    if args.local {
        // Offline recipe index: the embedded starter templates. Names are
        // "<lang>/<app>"; matching covers the name, the description line,
        // and the full template body (full-text, like `apt search`).
        for (name, body) in crate::wizard::EMBEDDED_TEMPLATES {
            let short = name.rsplit('/').next().unwrap_or(name);
            let desc = body
                .lines()
                .find_map(|l| {
                    let t = l.trim();
                    t.strip_prefix("description:")
                        .or_else(|| t.strip_prefix("summary:"))
                        .map(|v| v.trim().trim_matches('"').to_string())
                })
                .unwrap_or_default();
            if let Some(re) = &re {
                if !re.is_match(name)
                    && !re.is_match(short)
                    && !re.is_match(&desc)
                    && !re.is_match(body)
                {
                    continue;
                }
            }
            let installed_version = debs::dpkg_installed_version(short);
            let installed = installed_version.is_some() || manifest.packages.contains_key(short);
            if args.installed && !installed {
                continue;
            }
            if args.not_installed && installed {
                continue;
            }
            hits.push(Hit {
                name: (*name).to_string(),
                desc: format!("{desc} (template)"),
                installed,
                installed_version,
                candidate: None,
            });
        }
        print_hits(
            &hits,
            args.raw,
            "no templates matched",
            args.pattern.as_deref(),
        );
        return Ok(());
    }

    let repos = org_repos(debs::LATEST_DEBS_ORG, token)?;
    for repo in &repos {
        let Some(package) = repo.name.strip_suffix("-debian") else {
            continue;
        };
        let desc = repo.description.as_deref().unwrap_or_default();
        // Full-text: the org blurb plus the installed package's dpkg
        // long description when present (apt-search behavior).
        let full = debs::dpkg_description(package);
        if let Some(re) = &re {
            let in_full = full.as_ref().is_some_and(|f| re.is_match(f));
            if !re.is_match(package) && !re.is_match(desc) && !in_full {
                continue;
            }
        }
        let installed_version = debs::dpkg_installed_version(package);
        let installed = installed_version.is_some() || manifest.packages.contains_key(package);
        if args.installed && !installed {
            continue;
        }
        if args.not_installed && installed {
            continue;
        }
        let candidate = manifest
            .current(package)
            .map(|e| e.version.clone())
            .filter(|v| !v.is_empty());
        // Prefer the dpkg long description's first line when the org
        // blurb is empty; keep the org blurb otherwise.
        let blurb = if desc.is_empty() {
            full.unwrap_or_default()
        } else {
            desc.to_string()
        };
        hits.push(Hit {
            name: package.to_string(),
            desc: blurb,
            installed,
            installed_version,
            candidate,
        });
    }
    print_hits(
        &hits,
        args.raw,
        "no packages matched",
        args.pattern.as_deref(),
    );
    Ok(())
}
