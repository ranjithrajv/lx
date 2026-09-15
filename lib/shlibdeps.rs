// SPDX-License-Identifier: GPL-3.0-or-later

//! `dpkg-shlibdeps` parity: symbol-version-aware shared-library dependency
//! resolution.
//!
//! [`crate::elfdeps`] answers "which sonames does this binary need?"
//! (`DT_NEEDED`). This module answers the next question — "which *package*,
//! at which *minimum version*, provides each of them?" — the way
//! `dpkg-shlibdeps` does: read the binary's required symbol versions
//! (`.gnu.version_r` / `VERNEED`) and match them against the host's dpkg
//! `symbols` and `shlibs` databases, emitting versioned `Depends`
//! relations.
//!
//! Two consumers, two failure policies:
//!
//! * the build pipeline ([`crate::scandeps`], [`crate::bindep`]) and the
//!   standalone command all fall back to a per-soname package-manager lookup
//!   (`dpkg -S` / `rpm -q` / `pacman -o`) for sonames the dpkg `symbols` /
//!   `shlibs` databases don't cover — so the result on rpm/pacman hosts
//!   matches the old lookup and non-essential libraries still resolve;
//! * the standalone [`ShlibdepsArgs`] command is fail-closed by default — a
//!   soname that not even the package-manager lookup can place is an error
//!   unless `--ignore-missing-info`.

use anyhow::{bail, Result};
use clap::Args;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use object::read::elf::{ElfFile, FileHeader, Sym as _};
use object::ReadRef;

/// Default dpkg administrative directory, matching `dpkg-shlibdeps`.
pub const DEFAULT_DPKG_ADMINDIR: &str = "/var/lib/dpkg";

// ---------------------------------------------------------------------------
// ELF: required libraries + their required symbol versions
// ---------------------------------------------------------------------------

/// One shared library a binary needs, plus the `(symbol, version)` pairs it
/// references from it. `symbols` is empty for an unversioned library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibNeeded {
    pub soname: String,
    /// `(symbol name, version tag)` pairs, e.g. `("malloc", "GLIBC_2.2.5")`.
    pub symbols: BTreeSet<(String, String)>,
}

impl LibNeeded {
    /// A requirement carrying no symbol-version information.
    pub fn bare(soname: &str) -> Self {
        Self {
            soname: soname.to_string(),
            symbols: BTreeSet::new(),
        }
    }
}

/// Read an ELF's `DT_NEEDED` libraries (via [`crate::elfdeps`]) and attach
/// the symbol versions it requires from each (`.gnu.version_r`), so callers
/// can compute a versioned dependency the way `dpkg-shlibdeps` does.
pub fn needed_libraries_verbose(bytes: &[u8]) -> Result<Vec<LibNeeded>> {
    let sonames = crate::elfdeps::needed_libraries(bytes)?;
    let mut by_soname: BTreeMap<String, LibNeeded> = sonames
        .iter()
        .map(|s| (s.clone(), LibNeeded::bare(s)))
        .collect();

    let file = object::File::parse(bytes)
        .map_err(|e| anyhow::anyhow!("failed to parse as an ELF/object file: {e}"))?;
    match file {
        object::File::Elf32(f) => collect_symbol_versions(&f, &mut by_soname)?,
        object::File::Elf64(f) => collect_symbol_versions(&f, &mut by_soname)?,
        _ => {}
    }

    Ok(sonames
        .into_iter()
        .map(|s| by_soname.remove(&s).unwrap_or_else(|| LibNeeded::bare(&s)))
        .collect())
}

/// Collect the `VERNEED` symbol requirements from one parsed ELF, grouping
/// by the library (`vn_file`) that provides each versioned symbol.
fn collect_symbol_versions<'d, Elf, R>(
    f: &ElfFile<'d, Elf, R>,
    by_soname: &mut BTreeMap<String, LibNeeded>,
) -> Result<()>
where
    Elf: FileHeader<Endian = object::Endianness>,
    R: ReadRef<'d>,
{
    let endian = f.endian();
    let data = f.data();
    let Some(versions) = f
        .elf_section_table()
        .versions(endian, data)
        .map_err(|e| anyhow::anyhow!("parsing ELF version sections: {e}"))?
    else {
        return Ok(());
    };
    let dynsyms = f.elf_dynamic_symbol_table();
    for (index, sym) in dynsyms.symbols().iter().enumerate() {
        if !sym.is_undefined(endian) {
            continue;
        }
        let versym_index = versions.version_index(endian, object::SymbolIndex(index));
        let Some(version) = versions
            .version(versym_index.index())
            .map_err(|e| anyhow::anyhow!("invalid ELF symbol version index: {e}"))?
        else {
            continue;
        };
        // `file()` is set only for VERNEED (requirements), not VERDEF.
        let Some(lib) = version.file() else {
            continue;
        };
        let soname = String::from_utf8_lossy(lib).into_owned();
        let Some(entry) = by_soname.get_mut(&soname) else {
            continue; // a versioned symbol from a library not in DT_NEEDED
        };
        let Ok(symbol_name) = dynsyms.symbol_name(endian, sym) else {
            continue;
        };
        entry.symbols.insert((
            String::from_utf8_lossy(symbol_name).into_owned(),
            String::from_utf8_lossy(version.name()).into_owned(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tree scanning
// ---------------------------------------------------------------------------

/// The shared-library picture of a set of ELF files.
#[derive(Debug, Default, Clone)]
pub struct Scan {
    /// Non-essential libraries the scanned files need, with required symbol
    /// versions merged across files.
    pub needs: Vec<LibNeeded>,
    /// Every non-essential soname found (for declared-vs-actual diffing).
    pub sonames: BTreeSet<String>,
    /// Sonames the scanned tree itself provides (`libfoo.so.*` files).
    pub provided: BTreeSet<String>,
}

/// Scan ELF files for required libraries and symbol versions. Unreadable or
/// non-ELF inputs are skipped rather than failing the whole scan, matching
/// the robustness of the existing dependency scanners.
pub fn scan_elfs(paths: &[PathBuf]) -> Scan {
    let mut merged: BTreeMap<String, LibNeeded> = BTreeMap::new();
    let mut provided = BTreeSet::new();

    for path in paths {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.contains(".so") {
                provided.insert(name.to_string());
            }
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(libs) = needed_libraries_verbose(&bytes) else {
            continue;
        };
        for lib in libs {
            if crate::elfdeps::is_essential_libc_soname(&lib.soname) {
                continue;
            }
            let entry = merged
                .entry(lib.soname.clone())
                .or_insert_with(|| LibNeeded::bare(&lib.soname));
            entry.symbols.extend(lib.symbols);
        }
    }

    let sonames = merged.keys().cloned().collect();
    Scan {
        needs: merged.into_values().collect(),
        sonames,
        provided,
    }
}

// ---------------------------------------------------------------------------
// dpkg symbols / shlibs databases
// ---------------------------------------------------------------------------

/// One library stanza of a dpkg `symbols` file.
#[derive(Debug, Clone, Default)]
struct SymbolsStanza {
    soname: String,
    package: String,
    /// Header minimum version. `None` when the header is the `#MINVER#`
    /// placeholder (dpkg substitutes it, installed files keep it literal).
    min_version: Option<String>,
    /// `(symbol, version tag)` -> first version providing it.
    symbols: BTreeMap<(String, String), String>,
}

/// One line of a dpkg `shlibs` file, reduced to its relation.
#[derive(Debug, Clone)]
struct ShlibsEntry {
    relation: String,
}

/// The host's (or a given `--admindir`'s) dpkg dependency information:
/// `info/*.symbols` and `info/*.shlibs`, indexed by soname.
#[derive(Debug, Default)]
pub struct ShlibsDb {
    symbols: BTreeMap<String, SymbolsStanza>,
    shlibs: BTreeMap<String, ShlibsEntry>,
    /// `shlibs.local` overrides, highest precedence (dpkg's working-directory
    /// override for private libraries).
    local: BTreeMap<String, ShlibsEntry>,
    /// Real package names present in `<admindir>/status`.
    packages: BTreeSet<String>,
    /// `Provides:` name -> first real package providing it (from status).
    provides: BTreeMap<String, String>,
}

impl ShlibsDb {
    /// Load every `symbols`/`shlibs` file under `<admindir>/info`. Returns an
    /// empty database when the directory doesn't exist (non-dpkg hosts).
    pub fn open(admindir: &Path) -> Self {
        let mut db = ShlibsDb::default();
        let Ok(entries) = std::fs::read_dir(admindir.join("info")) else {
            return db;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.ends_with(".symbols") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for stanza in parse_symbols_file(&text) {
                    db.symbols.insert(stanza.soname.clone(), stanza);
                }
            } else if name.ends_with(".shlibs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (key, entry) in parse_shlibs_file(&text) {
                    db.shlibs.insert(key, entry);
                }
            }
        }
        db.load_status(admindir);
        db
    }

    /// Read `<admindir>/status` to build the real-package set and the virtual
    /// `Provides:` map, used to substitute a real package for a virtual name
    /// a shlibs/symbols stanza references.
    fn load_status(&mut self, admindir: &Path) {
        let Ok(text) = std::fs::read_to_string(admindir.join("status")) else {
            return;
        };
        for block in text.split("\n\n") {
            let mut package: Option<String> = None;
            let mut provides: Vec<String> = Vec::new();
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("Package: ") {
                    package = Some(v.trim().to_string());
                } else if let Some(v) = line.strip_prefix("Provides: ") {
                    provides.extend(v.split(',').map(|s| s.trim().to_string()));
                }
            }
            let Some(pkg) = package else { continue };
            self.packages.insert(pkg.clone());
            for prov in provides {
                let name = prov
                    .split_whitespace()
                    .next()
                    .unwrap_or(&prov)
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    self.provides.entry(name).or_insert_with(|| pkg.clone());
                }
            }
        }
    }

    /// The host's dependency database, parsed once per process. Reads
    /// `$LX_DPKG_ADMINDIR` when set, so a cross-suite/target rootfs can be
    /// resolved against instead of the build host (default `/var/lib/dpkg`).
    pub fn host() -> &'static ShlibsDb {
        static HOST: OnceLock<ShlibsDb> = OnceLock::new();
        HOST.get_or_init(|| {
            let dir = std::env::var("LX_DPKG_ADMINDIR")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_DPKG_ADMINDIR));
            ShlibsDb::open(&dir)
        })
    }

    /// Load a `shlibs.local` override file (same line format as `shlibs`).
    /// Its entries take precedence over both the `symbols` and `shlibs`
    /// databases, matching `dpkg-shlibdeps`. Chains onto
    /// [`open`](Self::open)/[`host`](Self::host).
    pub fn with_shlibs_local(mut self, path: &Path) -> Self {
        if let Ok(text) = std::fs::read_to_string(path) {
            for (key, entry) in parse_shlibs_file(&text) {
                self.local.insert(key, entry);
            }
        }
        self
    }

    /// True when there is no dpkg dependency information at all (e.g. an
    /// rpm/pacman host). Callers use the heuristic package-manager lookup
    /// instead.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty() && self.shlibs.is_empty() && self.local.is_empty()
    }
}

/// The working-directory `shlibs.local` `dpkg-shlibdeps` honours, if one
/// exists: `debian/shlibs.local` first, then `shlibs.local`.
pub fn find_shlibs_local() -> Option<PathBuf> {
    for candidate in ["debian/shlibs.local", "shlibs.local"] {
        let path = Path::new(candidate);
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    None
}

/// Parse a dpkg `symbols` file into per-library stanzas. Format (5.6.20):
///
/// ```text
/// libfoo.so.1 libfoo1 #MINVER#
///  foo@Base 1.2.3
///  bar@FOO_1.1 2.0
/// ```
fn parse_symbols_file(text: &str) -> Vec<SymbolsStanza> {
    let mut stanzas = Vec::new();
    let mut current: Option<SymbolsStanza> = None;

    let flush = |current: &mut Option<SymbolsStanza>, stanzas: &mut Vec<SymbolsStanza>| {
        if let Some(s) = current.take() {
            if !s.soname.is_empty() {
                stanzas.push(s);
            }
        }
    };

    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut current, &mut stanzas);
            continue;
        }
        if line.starts_with(|c: char| c.is_whitespace()) {
            let Some(cur) = current.as_mut() else {
                continue;
            };
            let t = line.trim();
            if t.starts_with('*') || t.starts_with('|') || t.starts_with('#') {
                continue;
            }
            // Symbol names can themselves contain '@' (C++ mangling), so
            // split on the last one; the rest is "version min-version".
            if let Some((sym, rest)) = t.rsplit_once('@') {
                let mut it = rest.split_whitespace();
                let version = it.next().unwrap_or("").to_string();
                let min = it.next().unwrap_or("").to_string();
                if !version.is_empty() {
                    cur.symbols.insert((sym.trim().to_string(), version), min);
                }
            }
        } else {
            flush(&mut current, &mut stanzas);
            let mut it = line.split_whitespace();
            let soname = it.next().unwrap_or("").to_string();
            let package = it.next().unwrap_or("").to_string();
            let min_raw = it.next().unwrap_or("");
            let min_version = if min_raw.is_empty() || min_raw == "#MINVER#" {
                None
            } else {
                Some(
                    min_raw
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .to_string(),
                )
            };
            current = Some(SymbolsStanza {
                soname,
                package,
                min_version,
                symbols: BTreeMap::new(),
            });
        }
    }
    flush(&mut current, &mut stanzas);
    stanzas
}

/// Parse a dpkg `shlibs` file into `"library soversion" -> relation`.
/// Format: `libfoo 1 libfoo1 (>= 1.0)`.
fn parse_shlibs_file(text: &str) -> Vec<(String, ShlibsEntry)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let library = it.next().unwrap_or("");
        let soversion = it.next().unwrap_or("");
        let relation = it.collect::<Vec<_>>().join(" ");
        if library.is_empty() || relation.is_empty() {
            continue;
        }
        out.push((format!("{library} {soversion}"), ShlibsEntry { relation }));
    }
    out
}

/// `libfoo.so.1` -> `"libfoo 1"`, the key used by dpkg `shlibs` files.
fn soname_key(soname: &str) -> Option<String> {
    let (base, version) = soname.rsplit_once(".so.")?;
    Some(format!("{base} {version}"))
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// The outcome of resolving a set of [`LibNeeded`] against a [`ShlibsDb`].
#[derive(Debug, Default, Clone)]
pub struct Resolution {
    /// Versioned relations, e.g. `libfoo1 (>= 1.2.3)`, deduplicated by
    /// package and sorted.
    pub relations: Vec<String>,
    /// Lowercased package names already covered by `relations`.
    pub resolved_names: BTreeSet<String>,
    /// Sonames with no `symbols` or `shlibs` entry.
    pub unresolved: Vec<String>,
    /// Required symbols absent from an existing `symbols` stanza, formatted
    /// `symbol@VERSION (from libfoo.so.1)`. `dpkg-shlibdeps` treats these as
    /// fatal; the best-effort build path ignores them.
    pub missing_symbols: Vec<String>,
}

/// Resolve required libraries to versioned Debian relations using the dpkg
/// `symbols` database first, then `shlibs`. Essential libc sonames and
/// sonames the package provides itself are skipped.
pub fn resolve(
    needs: &[LibNeeded],
    db: &ShlibsDb,
    exclude_pkg: Option<&str>,
    self_provided: &BTreeSet<String>,
) -> Resolution {
    // package -> highest required minimum version (None = unversioned).
    let mut by_pkg: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut unresolved = Vec::new();
    let mut missing_symbols = Vec::new();

    for need in needs {
        if crate::elfdeps::is_essential_libc_soname(&need.soname)
            || self_provided.contains(&need.soname)
        {
            continue;
        }

        let key = soname_key(&need.soname);

        // Precedence matches dpkg-shlibdeps: an explicit shlibs.local
        // override, then the package's symbols stanza (per-symbol minima),
        // then the coarse shlibs relation.
        let mut relation = key
            .as_ref()
            .and_then(|k| db.local.get(k))
            .map(|e| choose_alternative(&e.relation));

        if relation.is_none() {
            if let Some(stanza) = db.symbols.get(&need.soname) {
                let (rel, missing) = resolve_symbols(stanza, &need.symbols);
                relation = rel;
                for (sym, ver) in missing {
                    missing_symbols.push(format!("{sym}@{ver} (from {})", need.soname));
                }
            }
        }
        if relation.is_none() {
            relation = key
                .as_ref()
                .and_then(|k| db.shlibs.get(k))
                .map(|e| choose_alternative(&e.relation));
        }

        match relation {
            Some(rel) => {
                let (name, version) = split_relation(&rel);
                if name.is_empty() {
                    unresolved.push(need.soname.clone());
                    continue;
                }
                let name = virtual_real_name(db, &name).unwrap_or(name);
                let slot = by_pkg.entry(name).or_insert(None);
                if let Some(v) = version {
                    let take = match slot {
                        Some(existing) => debian_version_cmp(&v, existing) == Ordering::Greater,
                        None => true,
                    };
                    if take {
                        *slot = Some(v);
                    }
                }
            }
            None => unresolved.push(need.soname.clone()),
        }
    }

    let mut relations = Vec::new();
    let mut resolved_names = BTreeSet::new();
    for (name, version) in by_pkg {
        if exclude_pkg.is_some_and(|ex| name.split(':').next() == Some(ex)) {
            continue;
        }
        resolved_names.insert(name.to_ascii_lowercase());
        relations.push(match version {
            Some(v) => format!("{name} (>= {v})"),
            None => name,
        });
    }
    Resolution {
        relations,
        resolved_names,
        unresolved,
        missing_symbols,
    }
}

/// Compute the relation for a library from its symbols stanza: the maximum
/// minimum-version over the required symbols, falling back to the stanza's
/// header minimum version when no symbol matches. Also returns the required
/// symbols whose *name* does not appear anywhere in the stanza — the case
/// `dpkg-shlibdeps` reports as "symbol not found". A different version tag
/// on a known symbol is not flagged, to avoid false positives from tag
/// spelling differences.
fn resolve_symbols(
    stanza: &SymbolsStanza,
    required: &BTreeSet<(String, String)>,
) -> (Option<String>, Vec<(String, String)>) {
    if stanza.package.is_empty() {
        return (None, Vec::new());
    }
    let mut best: Option<String> = None;
    let mut missing = Vec::new();
    for (sym, ver) in required {
        match stanza.symbols.get(&(sym.clone(), ver.clone())) {
            Some(min) => {
                if min.is_empty() {
                    continue;
                }
                let take = match &best {
                    Some(existing) => debian_version_cmp(min, existing) == Ordering::Greater,
                    None => true,
                };
                if take {
                    best = Some(min.clone());
                }
            }
            None => {
                let name_known = stanza.symbols.keys().any(|(s, _)| s == sym);
                if !name_known {
                    missing.push((sym.clone(), ver.clone()));
                }
            }
        }
    }
    let version = best.or_else(|| stanza.min_version.clone());
    let rel = match version {
        Some(v) => format!("{} (>= {v})", stanza.package),
        None => stanza.package.clone(),
    };
    (Some(rel), missing)
}

/// Choose which `a | b` alternative of a shlibs relation to use: the first
/// whose package is installed on this host, else the first. This mirrors
/// `dpkg-shlibdeps`, which uses the installed package to disambiguate.
fn choose_alternative(rel: &str) -> String {
    let alts: Vec<&str> = rel
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    match alts.as_slice() {
        [] => rel.trim().to_string(),
        [only] => (*only).to_string(),
        _ => {
            for alt in &alts {
                let name = alt.split_whitespace().next().unwrap_or("");
                let base = name.split(':').next().unwrap_or(name);
                if !base.is_empty() && crate::scandeps::pkg_installed_version(base).is_some() {
                    return (*alt).to_string();
                }
            }
            alts[0].to_string()
        }
    }
}

/// If `name` is a virtual package provided by a real one (e.g. a time64
/// transition, `libfoo1` provided by `libfoo1t64`), return the real name,
/// preserving any `:arch` qualifier. `None` when `name` is itself a real
/// package or nothing in `<admindir>/status` provides it.
fn virtual_real_name(db: &ShlibsDb, name: &str) -> Option<String> {
    let (base, suffix) = match name.split_once(':') {
        Some((b, a)) => (b, format!(":{a}")),
        None => (name, String::new()),
    };
    if db.packages.contains(base) {
        return None;
    }
    db.provides.get(base).map(|real| format!("{real}{suffix}"))
}

/// Like [`crate::scandeps::pkg_owner`], but on dpkg hosts preserves the
/// multiarch `pkg:arch` qualifier `dpkg -S` reports, dropping it only for the
/// host's native architecture. Returns `None` on non-dpkg hosts (callers then
/// fall back to [`crate::scandeps::pkg_owner`]).
fn pkg_owner_qualified(soname: &str) -> Option<String> {
    let out = std::process::Command::new("dpkg")
        .args(["-S", soname])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let line = text.lines().next()?;
    // `dpkg -S` prints `pkg[:arch]: /path` (or `pkg1, pkg2: /path`).
    let (left, _path) = line.split_once(": ")?;
    let first = left.split(',').next()?.trim();
    let (name, arch) = match first.split_once(':') {
        Some((n, a)) => (n.trim(), Some(a.trim())),
        None => (first, None),
    };
    if name.is_empty() {
        return None;
    }
    match arch {
        Some(a) if !a.is_empty() && Some(a) != host_arch().as_deref() => {
            Some(format!("{name}:{a}"))
        }
        _ => Some(name.to_string()),
    }
}

/// The host's native dpkg architecture (`dpkg --print-architecture`).
fn host_arch() -> Option<String> {
    let out = std::process::Command::new("dpkg")
        .arg("--print-architecture")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

/// Split a relation into its package name and `(>= ...)` version, if any.
/// Only the first alternative of an `a | b` relation is used.
fn split_relation(rel: &str) -> (String, Option<String>) {
    let first = rel.split('|').next().unwrap_or(rel).trim();
    let name = first
        .split_whitespace()
        .next()
        .unwrap_or(first)
        .trim()
        .to_string();
    let version = (|| {
        let open = first.find('(')?;
        let close = first[open..].find(')')? + open;
        let inner = first[open + 1..close].trim();
        let v = inner.trim_start_matches(['>', '<', '=', ' ']).trim();
        (!v.is_empty()).then(|| v.to_string())
    })();
    (name, version)
}

/// Compare two Debian version strings per dpkg's algorithm (epoch, upstream
/// version, revision; `~` sorts before everything, digits compare
/// numerically). Needed to pick the highest minimum-version when several
/// required symbols come from the same package.
pub fn debian_version_cmp(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (u64, &str, &str) {
        let (epoch, rest) = match v.split_once(':') {
            Some((e, r)) if !e.is_empty() && e.bytes().all(|c| c.is_ascii_digit()) => {
                (e.parse().unwrap_or(0), r)
            }
            _ => (0, v),
        };
        match rest.rsplit_once('-') {
            Some((up, rev)) => (epoch, up, rev),
            None => (epoch, rest, ""),
        }
    }
    fn order(c: u8) -> i32 {
        if c.is_ascii_digit() {
            0
        } else if c == b'~' {
            -1
        } else if c.is_ascii_alphabetic() {
            c as i32
        } else {
            c as i32 + 256
        }
    }
    fn verrevcmp(a: &str, b: &str) -> Ordering {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit())
            {
                let ac = if i < a.len() && !a[i].is_ascii_digit() {
                    order(a[i])
                } else {
                    0
                };
                let bc = if j < b.len() && !b[j].is_ascii_digit() {
                    order(b[j])
                } else {
                    0
                };
                if ac != bc {
                    return ac.cmp(&bc);
                }
                if i < a.len() && !a[i].is_ascii_digit() {
                    i += 1;
                }
                if j < b.len() && !b[j].is_ascii_digit() {
                    j += 1;
                }
            }
            while i < a.len() && a[i] == b'0' {
                i += 1;
            }
            while j < b.len() && b[j] == b'0' {
                j += 1;
            }
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if i - si != j - sj {
                return (i - si).cmp(&(j - sj));
            }
            match a[si..i].cmp(&b[sj..j]) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        Ordering::Equal
    }

    let (ea, ua, ra) = split(a);
    let (eb, ub, rb) = split(b);
    match ea.cmp(&eb) {
        Ordering::Equal => {}
        other => return other,
    }
    match verrevcmp(ua, ub) {
        Ordering::Equal => verrevcmp(ra, rb),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Standalone command (`lx shlibdeps`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Args)]
pub struct ShlibdepsArgs {
    /// ELF executables/libraries, or directories to scan recursively.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,

    /// dpkg administrative directory to read `info/*.symbols` +
    /// `info/*.shlibs` from.
    #[arg(long, default_value = DEFAULT_DPKG_ADMINDIR)]
    pub admindir: PathBuf,

    /// Warn instead of failing when a library has no dependency information.
    #[arg(long)]
    pub ignore_missing_info: bool,

    /// Append `shlibs:Depends=<relations>` to this substvars file.
    #[arg(short = 'T', long)]
    pub substvars: Option<PathBuf>,

    /// Print the relation to stdout even when `-T` is given.
    #[arg(short = 'O', long)]
    pub print: bool,

    /// Additional ELF executables to analyze (repeatable), as in
    /// `dpkg-shlibdeps -e`.
    #[arg(short = 'e', long = "executable", value_name = "ELF")]
    pub executables: Vec<PathBuf>,

    /// The package being built; its own name is excluded from the emitted
    /// relations, as in `dpkg-shlibdeps -p`.
    #[arg(short = 'p', long = "package", value_name = "PKG")]
    pub package: Option<String>,

    /// Packages to treat as declared build dependencies, as in
    /// `dpkg-shlibdeps -d`. Accepted for compatibility.
    #[arg(short = 'd', long = "depends", value_name = "PKG")]
    pub depends: Vec<String>,

    /// Additional library search directories, as in `dpkg-shlibdeps -l`.
    /// Accepted for compatibility (resolution is by soname, natively).
    #[arg(short = 'l', long = "library-path", value_name = "DIR")]
    pub library_paths: Vec<PathBuf>,

    /// objdump binary to use, as in `dpkg-shlibdeps -S`. Accepted for
    /// compatibility (ELF parsing is native, not `objdump`-based).
    #[arg(short = 'S', long = "objdump", value_name = "PATH")]
    pub objdump: Option<PathBuf>,

    /// Override the `shlibs.local` file to read. Defaults to
    /// `debian/shlibs.local` then `shlibs.local` in the working directory.
    #[arg(long = "shlibs-local", value_name = "FILE")]
    pub shlibs_local: Option<PathBuf>,
}

/// `lx shlibdeps` — the standalone, fail-closed `dpkg-shlibdeps` equivalent.
pub fn run(args: ShlibdepsArgs) -> Result<()> {
    let mut elfs = Vec::new();
    for path in args.paths.iter().chain(args.executables.iter()) {
        collect_elfs(path, &mut elfs)?;
    }
    if elfs.is_empty() {
        bail!(
            "no ELF files found under: {}",
            args.paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let scan = scan_elfs(&elfs);
    let db = match args.shlibs_local.clone().or_else(find_shlibs_local) {
        Some(local) => ShlibsDb::open(&args.admindir).with_shlibs_local(&local),
        None => ShlibsDb::open(&args.admindir),
    };
    let res = resolve(&scan.needs, &db, args.package.as_deref(), &scan.provided);

    // Fall back to the host package-manager lookup (dpkg -S / rpm -q /
    // pacman -Qo) for sonames the dpkg symbols/shlibs databases don't cover,
    // exactly as the build pipeline (bindep, scandeps) does — so this command
    // resolves libraries on rpm/pacman hosts, not just dpkg ones.
    let Resolution {
        relations: base_relations,
        resolved_names: base_names,
        unresolved: base_unresolved,
        missing_symbols,
    } = res;

    let mut relations = base_relations;
    let mut resolved_names = base_names;
    let mut unresolved = Vec::new();
    for soname in base_unresolved {
        if let Some(pkg) =
            pkg_owner_qualified(&soname).or_else(|| crate::scandeps::pkg_owner(&soname))
        {
            if resolved_names.insert(pkg.to_ascii_lowercase()) {
                relations.push(pkg);
            }
        } else {
            unresolved.push(soname);
        }
    }

    // A required symbol absent from an existing `symbols` stanza is fatal,
    // matching dpkg-shlibdeps; `--ignore-missing-info` downgrades it.
    if !missing_symbols.is_empty() {
        if args.ignore_missing_info {
            for m in &missing_symbols {
                eprintln!("shlibdeps: warning: missing symbol information: {m}");
            }
        } else {
            bail!(
                "no dependency information for symbol(s): {}\n\
                 (pass --ignore-missing-info to warn instead of failing)",
                missing_symbols.join(", ")
            );
        }
    }

    if !unresolved.is_empty() {
        if args.ignore_missing_info {
            for soname in &unresolved {
                eprintln!("shlibdeps: warning: no dependency information found for {soname}");
            }
        } else {
            bail!(
                "no dependency information found for: {}\n\
                 (pass --ignore-missing-info to warn instead of failing)",
                unresolved.join(", ")
            );
        }
    }

    let line = format!("shlibs:Depends={}", relations.join(", "));
    if let Some(path) = &args.substvars {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| anyhow::anyhow!("failed to open {}: {e}", path.display()))?;
        writeln!(f, "{line}")?;
    }
    if args.print || args.substvars.is_none() {
        println!("{line}");
    }
    Ok(())
}

/// Collect every ELF file reachable from `path` (a file, or a directory
/// scanned recursively) into `out`.
fn collect_elfs(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect_elfs(&entry?.path(), out)?;
        }
    } else if crate::filemeta::is_elf(path).unwrap_or(false) {
        out.push(path.to_path_buf());
    }
    Ok(())
}
