// SPDX-License-Identifier: GPL-3.0-or-later

//! `deb-get` catalog compatibility — a **safe, static** reader.
//!
//! `deb-get` describes each package as a small bash file that it `source`s at
//! runtime. `lx` deliberately never executes upstream recipes (the same reason
//! an AUR `PKGBUILD` becomes comments, not code), so this module parses the
//! *declarative subset* of a definition instead:
//!
//! * `KEY=value` / `KEY="value"` / `KEY='value'` assignments;
//! * the method getter calls (`get_github_releases "owner/repo"`,
//!   `get_gitlab_releases`, `get_website`) as *markers* — never run; `lx`
//!   resolves the release itself through its own forge clients;
//! * `00-builtin`-style function bodies are not read (they are code in the
//!   deb-get script, not files here).
//!
//! Anything the parser cannot express statically — a `post_download`
//! hook, a `URL=$(…)` computation, arbitrary control flow — is recorded in
//! [`DebGetPackage::unsupported`] and surfaced to the user rather than
//! guessed at or executed.
//!
//! Catalog layout (under the root, default `/etc/deb-get`, override with
//! `$LX_DEBGET_DIR`):
//!
//! ```text
//! <root>/01-main.repo        # first line = base URL (external repos)
//! <root>/01-main.d/<pkg>     # one definition file per package
//! <root>/99-local.d/<pkg>    # user overrides, highest precedence
//! ```
//!
//! Repo precedence follows deb-get: the two-digit prefix orders repos, and the
//! highest priority (largest number, e.g. `99-local`) wins for a duplicate
//! package name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Default catalog root (deb-get's own `/etc/deb-get`).
pub const DEFAULT_ROOT: &str = "/etc/deb-get";

/// Environment override for the catalog root (used by tests and for a
/// user-local catalog).
pub const ROOT_ENV: &str = "LX_DEBGET_DIR";

/// How a [`DebGetPackage`] is fetched/installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebGetKind {
    /// A third-party apt repository (`APT_REPO_URL` + a signing key).
    Apt,
    /// A Launchpad PPA (`PPA="ppa:user/archive"`).
    Ppa,
    /// A GitHub release asset (`get_github_releases`).
    Github,
    /// A GitLab release asset (`get_gitlab_releases`).
    Gitlab,
    /// A `.deb` scraped from a project page (`get_website`).
    Website,
    /// A literal `URL=` to a `.deb`.
    Direct,
    /// No recognizable method.
    #[default]
    Unknown,
}

/// One parsed `deb-get` package definition.
#[derive(Debug, Clone, Default)]
pub struct DebGetPackage {
    /// Package name (the definition file name).
    pub name: String,
    /// Source repo directory (e.g. `01-main`, `99-local`).
    pub repo: String,
    pub defver: Option<u32>,
    pub pretty_name: String,
    pub summary: String,
    pub website: String,
    pub eula: Option<String>,
    pub archs_supported: Vec<String>,
    pub codenames_supported: Vec<String>,
    pub kind: DebGetKind,
    pub apt_repo_url: Option<String>,
    pub apt_repo_options: Option<String>,
    pub apt_list_name: Option<String>,
    pub asc_key_url: Option<String>,
    pub gpg_key_url: Option<String>,
    pub gpg_key_id: Option<String>,
    pub ppa: Option<String>,
    /// `owner/repo` from `get_github_releases` / `get_gitlab_releases`.
    pub forge_repo: Option<String>,
    /// A literal `URL=` value (only used by [`DebGetKind::Direct`]).
    pub url: Option<String>,
    /// The definition sets `URL`/`VERSION_PUBLISHED` via shell computation we
    /// do not execute. `lx` can still resolve these for github/gitlab; for
    /// direct/website it means the URL is unavailable.
    pub dynamic_url: bool,
    /// A `post_download` hook exists (deb-get may verify here). Recorded so the
    /// install path can refuse to silently skip it.
    pub has_post_download: bool,
    /// Human-readable reasons this definition is not fully usable statically.
    pub unsupported: Vec<String>,
}

impl DebGetPackage {
    /// True when every required field for the resolved kind is present.
    pub fn is_supported(&self) -> bool {
        self.unsupported.is_empty()
    }

    /// deb-get's host gate: `ARCHS_SUPPORTED`/`CODENAMES_SUPPORTED`. An empty
    /// list means "any". `codename` is the host's `VERSION_CODENAME`.
    pub fn supports(&self, arch: &str, codename: Option<&str>) -> bool {
        let arch_ok = arch.is_empty()
            || self.archs_supported.is_empty()
            || self.archs_supported.iter().any(|a| a == "all" || a == arch);
        let code_ok = self.codenames_supported.is_empty()
            || codename.is_some_and(|c| self.codenames_supported.iter().any(|x| x == c));
        arch_ok && code_ok
    }

    /// deb-get `METHOD` name for this definition.
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            DebGetKind::Apt => "apt",
            DebGetKind::Ppa => "ppa",
            DebGetKind::Github => "github",
            DebGetKind::Gitlab => "gitlab",
            DebGetKind::Website => "website",
            DebGetKind::Direct => "direct",
            DebGetKind::Unknown => "unknown",
        }
    }
}

/// A loaded catalog: package name -> definition, with repo precedence applied.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub root: PathBuf,
    /// The winning definition per package name (highest repo priority).
    pub packages: BTreeMap<String, DebGetPackage>,
    /// Every definition, one per `(repo, name)`, in repo-priority order. Used
    /// by per-repo listings (`lx list --catalog --format pretty/csv`), which — like
    /// deb-get — show each repo's own copy even when a higher-priority repo
    /// overrides it.
    pub definitions: Vec<DebGetPackage>,
}

impl Catalog {
    /// Load every `<repo>.d/*` definition under `root`, applying deb-get's
    /// "highest repo priority wins" rule for duplicate names.
    pub fn load(root: &Path) -> Self {
        let mut definitions: Vec<DebGetPackage> = Vec::new();
        let mut by_name: BTreeMap<String, DebGetPackage> = BTreeMap::new();

        for repo_dir in repo_dirs(root) {
            let repo = repo_dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.strip_suffix(".d").unwrap_or(n).to_string())
                .unwrap_or_default();
            let mut entries: Vec<PathBuf> = match std::fs::read_dir(&repo_dir) {
                Ok(rd) => rd
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .collect(),
                Err(_) => continue,
            };
            entries.sort();
            for path in entries {
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let pkg = parse_definition(&name, &repo, &text);
                // Repo dirs are visited highest-priority first (see
                // `repo_dirs`), so the first definition of a name wins.
                by_name.entry(name).or_insert_with(|| pkg.clone());
                definitions.push(pkg);
            }
        }

        Catalog {
            root: root.to_path_buf(),
            packages: by_name,
            definitions,
        }
    }

    /// Every definition from `repo` (with deb-get's `01-main` also matching
    /// the script-internal `00-builtin` bucket), sorted by name.
    pub fn definitions_in(&self, repo: &str) -> Vec<&DebGetPackage> {
        let mut v: Vec<&DebGetPackage> = self
            .definitions
            .iter()
            .filter(|p| p.repo == repo || (repo == "01-main" && p.repo == "00-builtin"))
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// The configured catalog root: `$LX_DEBGET_DIR`, else `/etc/deb-get` when
    /// that exists (an actual deb-get install), else a user-writable
    /// `~/.local/share/lx/debget` so `lx index update` works without root.
    pub fn default_root() -> PathBuf {
        if let Some(v) = std::env::var_os(ROOT_ENV).filter(|v| !v.is_empty()) {
            return PathBuf::from(v);
        }
        let system = PathBuf::from(DEFAULT_ROOT);
        if system.is_dir() {
            return system;
        }
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("lx")
            .join("debget")
    }

    /// Load from [`Catalog::default_root`].
    pub fn load_default() -> Self {
        Self::load(&Self::default_root())
    }

    pub fn get(&self, name: &str) -> Option<&DebGetPackage> {
        self.packages.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

/// Repo directories under `root`, highest priority first. `NN-name.d` sorts by
/// the two-digit prefix descending, so `99-local.d` precedes `01-main.d`.
fn repo_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<(u32, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    for entry in rd.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".d") else {
            continue;
        };
        // Priority = the leading two-digit number, if any.
        let prio = stem
            .split_once('-')
            .and_then(|(n, _)| n.parse::<u32>().ok())
            .unwrap_or(0);
        dirs.push((prio, path));
    }
    // Highest priority first; tie-break by name for determinism.
    dirs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    dirs.into_iter().map(|(_, p)| p).collect()
}

/// Parse one definition file. Never executes anything.
pub fn parse_definition(name: &str, repo: &str, text: &str) -> DebGetPackage {
    let mut p = DebGetPackage {
        name: name.to_string(),
        repo: repo.to_string(),
        kind: DebGetKind::Unknown,
        ..Default::default()
    };

    let mut func_names: Vec<String> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Function definitions: capture the name, never the body. `get_*`
        // getters and `post_download` are the ones we care about.
        if let Some(fname) = function_name(line) {
            func_names.push(fname);
            continue;
        }

        // Getter markers: `get_github_releases "owner/repo" [ref]`.
        if let Some(rest) = line.strip_prefix("get_github_releases") {
            p.kind = DebGetKind::Github;
            p.forge_repo = first_quoted(rest);
            continue;
        }
        if let Some(rest) = line.strip_prefix("get_gitlab_releases") {
            p.kind = DebGetKind::Gitlab;
            p.forge_repo = first_quoted(rest);
            continue;
        }
        if line.starts_with("get_website") {
            p.kind = DebGetKind::Website;
            continue;
        }

        // Declarative assignment.
        if let Some((key, value)) = parse_assignment(line) {
            let dynamic = value.contains("$(") || value.contains('`');
            match key.as_str() {
                "DEFVER" => p.defver = value.parse().ok(),
                "ARCHS_SUPPORTED" => p.archs_supported = split_words(&value),
                "CODENAMES_SUPPORTED" => p.codenames_supported = split_words(&value),
                "PRETTY_NAME" => p.pretty_name = value,
                "SUMMARY" => p.summary = value,
                "WEBSITE" => p.website = value,
                "EULA" => p.eula = Some(value),
                "APT_REPO_URL" => p.apt_repo_url = Some(value),
                "APT_REPO_OPTIONS" => p.apt_repo_options = Some(value),
                "APT_LIST_NAME" => p.apt_list_name = Some(value),
                "ASC_KEY_URL" => p.asc_key_url = Some(value),
                "GPG_KEY_URL" => p.gpg_key_url = Some(value),
                "GPG_KEY_ID" => p.gpg_key_id = Some(value),
                "PPA" => p.ppa = Some(value),
                "GH_REPO" => p.forge_repo = Some(value),
                "GITLAB_REPO" => p.forge_repo = Some(value),
                "URL" => {
                    if dynamic {
                        p.dynamic_url = true;
                    } else {
                        p.url = Some(value);
                    }
                }
                // Anything unknown at top level is harmless to ignore; only
                // *dynamic* values (command substitution) matter.
                _ => {}
            }
        }
    }

    // Resolve the method, mirroring deb-get's own precedence.
    if p.apt_repo_url.is_some() {
        p.kind = DebGetKind::Apt;
    } else if p.ppa.is_some() {
        p.kind = DebGetKind::Ppa;
    } else if p.kind == DebGetKind::Unknown {
        if p.url.is_some() {
            p.kind = DebGetKind::Direct;
        } else if p.dynamic_url {
            p.unsupported
                .push("URL is computed at runtime (shell) and cannot be read statically".into());
        } else {
            p.unsupported.push(
                "no recognized method (expected URL, GH_REPO/GITLAB_REPO, APT_REPO_URL, or PPA)"
                    .into(),
            );
        }
    }

    // Hooks we deliberately do not run.
    if func_names.iter().any(|f| f == "post_download") {
        p.has_post_download = true;
        p.unsupported.push(
            "defines a `post_download` hook (verification/rename step); lx does not execute it"
                .into(),
        );
    }

    // Required fields per deb-get's own validation.
    for (field, present) in [
        ("PRETTY_NAME", !p.pretty_name.is_empty()),
        ("SUMMARY", !p.summary.is_empty()),
        ("WEBSITE", !p.website.is_empty()),
    ] {
        if !present {
            p.unsupported.push(format!("missing required {field}"));
        }
    }
    match p.kind {
        DebGetKind::Apt
            if p.asc_key_url.is_none() && p.gpg_key_url.is_none() && p.gpg_key_id.is_none() =>
        {
            p.unsupported
                .push("apt repo has no signing key (ASC_KEY_URL/GPG_KEY_URL/GPG_KEY_ID)".into())
        }
        DebGetKind::Github | DebGetKind::Gitlab if p.forge_repo.is_none() => p
            .unsupported
            .push("no owner/repo for the release getter".into()),
        DebGetKind::Direct if p.url.is_none() => p.unsupported.push("no literal URL".into()),
        _ => {}
    }

    p
}

/// `NAME() { … }` or `function NAME { … }` / `function NAME() {`.
fn function_name(line: &str) -> Option<String> {
    let line = line.strip_prefix("function ").unwrap_or(line);
    let open = line.find('(')?.max(0);
    let name_part = if line.starts_with("function ") {
        line.split_whitespace().nth(1)
    } else {
        Some(&line[..open])
    }?;
    let name = name_part.trim().trim_end_matches("()");
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    // Only a function definition if it looks like one.
    if line.contains('{') || line.contains("()") {
        Some(name.to_string())
    } else {
        None
    }
}

/// `export KEY=value` / `KEY=value` / `KEY="value"` / `KEY='value'`.
fn parse_assignment(line: &str) -> Option<(String, String)> {
    let line = line.strip_prefix("export ").unwrap_or(line);
    // Skip control-flow keywords that contain no '=' anyway.
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value = unquote(rest.trim());
    Some((key.to_string(), value))
}

/// Strip one layer of surrounding single/double quotes, decoding the escapes
/// deb-get definitions commonly use (`\n`, `\"`).
fn unquote(v: &str) -> String {
    let bytes = v.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        let inner = &v[1..v.len() - 1];
        if bytes[0] == b'"' {
            return inner.replace("\\n", "\n").replace("\\\"", "\"");
        }
        return inner.to_string();
    }
    v.to_string()
}

/// The first double-quoted token in `s` (used for getter arguments).
fn first_quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let rest = &s[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn split_words(v: &str) -> Vec<String> {
    v.split_whitespace().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_apt_definition() {
        let text = r#"
DEFVER=2
ARCHS_SUPPORTED="amd64 arm64"
ASC_KEY_URL="https://dl.google.com/linux/linux_signing_key.pub"
APT_LIST_NAME="google-chrome"
APT_REPO_URL="https://dl.google.com/linux/chrome/deb/ stable main"
PRETTY_NAME="Google Chrome"
WEBSITE="https://www.google.com/chrome/"
SUMMARY="Fast, Secure Browser from Google."
"#;
        let p = parse_definition("google-chrome-stable", "01-main", text);
        assert_eq!(p.kind, DebGetKind::Apt);
        assert_eq!(p.defver, Some(2));
        assert_eq!(p.archs_supported, ["amd64", "arm64"]);
        assert_eq!(p.apt_list_name.as_deref(), Some("google-chrome"));
        assert_eq!(
            p.apt_repo_url.as_deref(),
            Some("https://dl.google.com/linux/chrome/deb/ stable main")
        );
        assert_eq!(p.pretty_name, "Google Chrome");
        assert!(p.unsupported.is_empty(), "{:?}", p.unsupported);
    }

    #[test]
    fn detects_github_getter_and_ignores_its_shell_block() {
        let text = r#"
DEFVER=1
ARCHS_SUPPORTED="amd64 arm64"
get_github_releases "cli/cli" "latest"
if [ "${ACTION}" != "prettylist" ]; then
    URL=$(grep -m 1 "browser_download_url.*${HOST_ARCH}\.deb\"" "${CACHE_FILE}" | cut -d '"' -f 4)
    VERSION_PUBLISHED=$(cut -d '/' -f 8 <<< "${URL//v/}")
fi
PRETTY_NAME="GitHub CLI"
WEBSITE="https://cli.github.com/"
SUMMARY="GitHub CLI brings GitHub to your terminal."
"#;
        let p = parse_definition("gh", "01-main", text);
        assert_eq!(p.kind, DebGetKind::Github);
        assert_eq!(p.forge_repo.as_deref(), Some("cli/cli"));
        assert!(p.dynamic_url, "the computed URL should be flagged dynamic");
        // github resolves the URL itself, so a dynamic shell URL is not fatal.
        assert!(p.unsupported.is_empty(), "{:?}", p.unsupported);
    }

    #[test]
    fn flags_a_post_download_hook() {
        let text = r#"
DEFVER=1
URL="https://example.com/app.deb"
PRETTY_NAME="App"
WEBSITE="https://example.com"
SUMMARY="An app"
function post_download() {
    sha256sum -c <<< "abc123  ${CACHE_DIR}/${FILE}"
}
"#;
        let p = parse_definition("app", "01-main", text);
        assert_eq!(p.kind, DebGetKind::Direct);
        assert!(p.has_post_download);
        assert!(p.unsupported.iter().any(|r| r.contains("post_download")));
    }

    #[test]
    fn direct_without_url_is_unsupported() {
        let p = parse_definition(
            "x",
            "01-main",
            "DEFVER=1\nPRETTY_NAME=\"X\"\nWEBSITE=\"https://x\"\nSUMMARY=\"x\"\n",
        );
        assert_eq!(p.kind, DebGetKind::Unknown);
        assert!(p
            .unsupported
            .iter()
            .any(|r| r.contains("URL") || r.contains("method")));
    }

    #[test]
    fn unquote_handles_single_double_and_bare() {
        assert_eq!(unquote("\"a b\""), "a b");
        assert_eq!(unquote("'a b'"), "a b");
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(unquote("\"line1\\nline2\""), "line1\nline2");
    }

    #[test]
    fn catalog_applies_repo_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("01-main.d")).unwrap();
        std::fs::create_dir_all(root.join("99-local.d")).unwrap();
        let base = "DEFVER=1\nURL=\"https://example.com/a.deb\"\nPRETTY_NAME=\"A\"\nWEBSITE=\"https://a\"\nSUMMARY=\"a\"\n";
        std::fs::write(root.join("01-main.d/app"), base).unwrap();
        // The local override must win.
        std::fs::write(
            root.join("99-local.d/app"),
            "DEFVER=1\nURL=\"https://local.example.com/a.deb\"\nPRETTY_NAME=\"A local\"\nWEBSITE=\"https://a\"\nSUMMARY=\"a\"\n",
        )
        .unwrap();
        std::fs::write(root.join("01-main.d/only"), base).unwrap();

        let cat = Catalog::load(root);
        assert_eq!(cat.packages.len(), 2);
        assert_eq!(cat.get("app").unwrap().repo, "99-local");
        assert_eq!(
            cat.get("app").unwrap().url.as_deref(),
            Some("https://local.example.com/a.deb")
        );
        assert_eq!(cat.get("only").unwrap().repo, "01-main");
        // Both copies of `app` survive for per-repo listings.
        assert_eq!(cat.definitions.len(), 3);
        assert_eq!(cat.definitions_in("01-main").len(), 2);
        assert_eq!(cat.definitions_in("99-local").len(), 1);
    }

    #[test]
    fn supports_gates_on_arch_and_codename() {
        let text = "DEFVER=1\nARCHS_SUPPORTED=\"amd64 arm64\"\nCODENAMES_SUPPORTED=\"jammy noble\"\nURL=\"https://example.com/a.deb\"\nPRETTY_NAME=\"A\"\nWEBSITE=\"https://a\"\nSUMMARY=\"a\"\n";
        let p = parse_definition("app", "01-main", text);
        assert!(p.supports("amd64", Some("jammy")));
        assert!(p.supports("arm64", Some("noble")));
        assert!(!p.supports("armhf", Some("jammy")), "arch mismatch");
        assert!(!p.supports("amd64", Some("bookworm")), "codename mismatch");
        assert!(!p.supports("amd64", None), "missing codename");
        // An unknown host arch skips the arch gate.
        assert!(p.supports("", Some("jammy")));
    }

    #[test]
    fn arch_gate_defaults_to_any() {
        let text = "DEFVER=1\nURL=\"https://example.com/a.deb\"\nPRETTY_NAME=\"A\"\nWEBSITE=\"https://a\"\nSUMMARY=\"a\"\n";
        let p = parse_definition("app", "01-main", text);
        assert!(p.supports("riscv64", None));
    }

    #[test]
    fn repo_dirs_sort_by_priority_descending() {
        let dir = tempfile::tempdir().unwrap();
        for r in ["01-main.d", "00-builtin.d", "99-local.d", "05-extra.d"] {
            std::fs::create_dir_all(dir.path().join(r)).unwrap();
        }
        let names: Vec<String> = repo_dirs(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["99-local.d", "05-extra.d", "01-main.d", "00-builtin.d"]
        );
    }
}
