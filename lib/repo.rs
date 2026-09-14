// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx repo` — productize the prebuilt binary repository (Prebuilt-MPR
//! spirit): turn a directory of built `.deb`s into an apt-servable
//! repository with `Packages`, `Packages.gz`, `Release`, and a clearsigned
//! `InRelease` when a signing key is given. The `latest-debs` apt repo +
//! `lx install` against it is then a complete producer→distributor loop.

use anyhow::{bail, Context, Result};
use clap::Args;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Args)]
pub struct RepoArgs {
    /// Directory containing the built packages (also receives the index
    /// files).
    pub dir: PathBuf,

    /// Package format to index: `deb` (apt), `ipk` (opkg), `arch` (pacman),
    /// `apk` (Alpine), or `rpm` (repodata). Defaults to this host's native
    /// format (auto-detected), falling back to `deb`.
    #[arg(long)]
    pub format: Option<String>,

    /// Suite name recorded in `Release` (default: stable).
    /// Ignored when --multi-suite is used (suite names come from subdirectories).
    #[arg(long, default_value = "stable")]
    pub suite: String,

    /// Multi-suite mode: treat `dists/<suite>/` subdirectories as separate
    /// suites. Each suite gets its own Packages/Packages.gz/Release under
    /// `dists/<suite>/`, and a top-level Release is written listing all
    /// suites. This mirrors the layout of real Debian/Ubuntu archives.
    #[arg(long)]
    pub multi_suite: bool,

    /// Components to list in Release (comma-separated).
    /// Only used in --multi-suite mode. Default: main.
    #[arg(long, default_value = "main")]
    pub components: String,

    /// Origin label for the Release file.
    #[arg(long, default_value = "latest-debs")]
    pub origin: String,

    /// Path to an ASCII-armored secret key for clearsigning `InRelease`.
    /// Passphrase via $LX_SIGN_PASSPHRASE / $NFPM_PASSPHRASE. Without a
    /// key, only the unsigned `Release` is written.
    #[arg(long, value_name = "KEY_FILE")]
    pub sign_key: Option<PathBuf>,

    /// Signing key id / fingerprint (gpg --local-user).
    #[arg(long, value_name = "KEY_ID")]
    pub sign_key_id: Option<String>,
}

pub fn run(mut args: RepoArgs) -> Result<()> {
    if !args.dir.is_dir() {
        bail!("'{}' is not a directory", args.dir.display());
    }

    // Smart default: index as this host's native format unless told otherwise.
    if args.format.is_none() {
        let format = crate::info::native_plugin_format().unwrap_or("deb");
        println!("--format not given; indexing as '{format}' (this host's native format)");
        args.format = Some(format.to_string());
    }

    // Multi-suite mode is apt-specific; every other case routes through the
    // PackageIndex plugin registry (write role: build + sign).
    if args.multi_suite
        && args
            .format
            .as_deref()
            .unwrap_or("deb")
            .eq_ignore_ascii_case("deb")
    {
        return run_multi_suite(&args);
    }
    run_format(&args)
}

/// Route a `--format` through the [`package_index`](crate::plugins::package_index)
/// plugin registry's write role. Multi-suite apt is handled separately.
fn run_format(args: &RepoArgs) -> Result<()> {
    use crate::plugins::package_index::{self, FORMAT_ALIASES};

    let format = args.format.as_deref().unwrap_or("deb");
    let backend = package_index::get_index_backend(format).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported --format '{}' (expected one of: {})",
            format,
            FORMAT_ALIASES
                .iter()
                .map(|(alias, _)| *alias)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    if !backend.capabilities.can_write() {
        bail!(
            "--format '{}' resolves to '{}', which cannot publish a repository index",
            format,
            backend.id
        );
    }
    let indexer = backend
        .make_writer(format)
        .ok_or_else(|| anyhow::anyhow!("'{}' cannot publish a repository index", backend.id))?;
    let ext = indexer
        .file_extension()
        .ok_or_else(|| anyhow::anyhow!("'{}' has no artifact extension", backend.id))?;
    let artifacts = package_index::artifacts_with_ext(&args.dir, ext)?;
    if artifacts.is_empty() {
        bail!("no .{ext} files in '{}'", args.dir.display());
    }
    let opts = package_index::IndexOptions {
        suite: &args.suite,
        origin: &args.origin,
        components: &args.components,
        sign_key: args.sign_key.as_deref(),
        sign_key_id: args.sign_key_id.as_deref().unwrap_or(""),
    };
    indexer.build_index(&args.dir, &artifacts, &opts)?;
    indexer.sign_index(&args.dir, &opts)
}

/// Build an apt `Packages`/`Packages.gz`/`Release` index (unsigned). Public
/// so the `apt` repo-indexer plugin can reuse the original implementation;
/// the plugin's `sign_index` adds `InRelease`/`Release.gpg`.
pub fn build_apt_index(dir: &Path, debs: &[PathBuf], suite: &str, origin: &str) -> Result<()> {
    let (packages, archs) = build_packages_index(debs)?;
    std::fs::write(dir.join("Packages"), &packages)?;
    let gz = lx_lib::debarchive::deterministic_gzip_bytes(packages.as_bytes(), 0, 9)?;
    std::fs::write(dir.join("Packages.gz"), &gz)?;

    let date = jiff::Timestamp::now()
        .strftime("%a, %d %b %Y %H:%M:%S UTC")
        .to_string();
    let mut arch_list: Vec<String> = archs.keys().cloned().collect();
    arch_list.sort();
    let mut release = format!(
        "Origin: {origin}\nLabel: {origin}\nSuite: {suite}\nCodename: {suite}\nDate: {date}\nArchitectures: {}\nComponents: main\nDescription: {origin} prebuilt repository (generated by lx repo)\n",
        arch_list.join(" "),
    );
    for index in ["Packages", "Packages.gz"] {
        let bytes = std::fs::read(dir.join(index))?;
        release.push_str(&index_hashes(index, &bytes));
    }
    std::fs::write(dir.join("Release"), &release)?;
    println!(
        "wrote Packages, Packages.gz, Release ({} packages)",
        debs.len()
    );
    Ok(())
}

/// Multi-suite mode: discover suites under `dists/`, build per-suite indices,
/// then write a top-level Release referencing all suites.
fn run_multi_suite(args: &RepoArgs) -> Result<()> {
    let dists_dir = args.dir.join("dists");

    // Discover suites: either subdirectories of `dists/` or `.deb` files
    // directly in `dists/` (flat layout).
    let suites = discover_suites(&dists_dir, &args.dir)?;

    if suites.is_empty() {
        bail!(
            "no suites found — expected .deb files in dists/<suite>/ subdirectories or dists/ directly"
        );
    }

    // Single-suite fallback: if we only found .deb files in the root (no
    // dists/ layout), behave exactly like single-suite mode — write one
    // Release with Suite: <name>, no top-level multi-suite Release.
    if suites.len() == 1 && suites[0].1 == args.dir {
        let (suite_name, _) = &suites[0];
        let mut debs: Vec<PathBuf> = std::fs::read_dir(args.dir.clone())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|e| e == "deb").unwrap_or(false))
            .collect();
        debs.sort();
        let (packages, archs) = build_packages_index(&debs)?;
        std::fs::write(args.dir.join("Packages"), &packages)?;
        let gz = lx_lib::debarchive::deterministic_gzip_bytes(packages.as_bytes(), 0, 9)?;
        std::fs::write(args.dir.join("Packages.gz"), &gz)?;

        let date = jiff::Timestamp::now()
            .strftime("%a, %d %b %Y %H:%M:%S UTC")
            .to_string();
        let mut arch_list: Vec<String> = archs.keys().cloned().collect();
        arch_list.sort();
        let mut release = format!(
            "Origin: {}\nLabel: {}\nSuite: {}\nCodename: {}\nDate: {}\nArchitectures: {}\nComponents: main\nDescription: {} prebuilt repository (generated by lx repo)\n",
            args.origin,
            args.origin,
            suite_name,
            suite_name,
            date,
            arch_list.join(" "),
            args.origin,
        );
        for index in ["Packages", "Packages.gz"] {
            let bytes = std::fs::read(args.dir.join(index))?;
            release.push_str(&index_hashes(index, &bytes));
        }
        std::fs::write(args.dir.join("Release"), &release)?;
        println!(
            "wrote Packages, Packages.gz, Release ({} packages, single-suite)",
            debs.len()
        );
        sign_release(&args.dir, &release, args)?;
        return Ok(());
    }

    println!("multi-suite mode: {} suite(s) found", suites.len());

    let components: Vec<String> = args
        .components
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let date = jiff::Timestamp::now()
        .strftime("%a, %d %b %Y %H:%M:%S UTC")
        .to_string();

    // Build per-suite indices.
    let mut suite_summaries: Vec<(String, Vec<String>, usize)> = Vec::new(); // (suite, archs, count)
    for (suite_name, deb_dir) in &suites {
        let mut debs: Vec<PathBuf> = std::fs::read_dir(deb_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|e| e == "deb").unwrap_or(false))
            .collect();
        debs.sort();

        if debs.is_empty() {
            println!("  skipping '{}': no .deb files", suite_name);
            continue;
        }

        // Write suite-level Packages/Packages.gz under dists/<suite>/
        let (packages, archs) = build_packages_index_with_base(&debs, suite_name)?;

        // Determine the correct output path: for dists/<suite>/ layout, write directly there.
        let out_dir = if deb_dir != &args.dir {
            deb_dir.clone()
        } else {
            args.dir.clone()
        };

        std::fs::create_dir_all(&out_dir)?;
        std::fs::write(out_dir.join("Packages"), &packages)?;
        let gz = lx_lib::debarchive::deterministic_gzip_bytes(packages.as_bytes(), 0, 9)?;
        std::fs::write(out_dir.join("Packages.gz"), &gz)?;

        // Per-suite Release.
        let mut arch_list: Vec<String> = archs.keys().cloned().collect();
        arch_list.sort();
        let mut release = format!(
            "Origin: {}\nLabel: {}\nSuite: {}\nCodename: {}\nDate: {}\nArchitectures: {}\nComponents: {}\nDescription: {} suite {} (generated by lx repo)\n",
            args.origin,
            args.origin,
            suite_name,
            suite_name,
            date,
            arch_list.join(" "),
            components.join(" "),
            args.origin,
            suite_name,
        );
        for index in ["Packages", "Packages.gz"] {
            let bytes = std::fs::read(out_dir.join(index))?;
            release.push_str(&index_hashes(index, &bytes));
        }
        std::fs::write(out_dir.join("Release"), &release)?;

        println!(
            "  ✓ suite '{}': {} package(s), archs: {}",
            suite_name,
            debs.len(),
            arch_list.join(", ")
        );

        suite_summaries.push((suite_name.clone(), arch_list, debs.len()));
    }

    if suite_summaries.is_empty() {
        bail!("no packages found in any suite");
    }

    // Top-level Release listing all suites.
    let all_archs: BTreeMap<String, bool> = suite_summaries
        .iter()
        .flat_map(|(_, archs, _)| archs.iter().map(|a| (a.clone(), true)))
        .collect();
    let mut all_arch_list: Vec<String> = all_archs.keys().cloned().collect();
    all_arch_list.sort();

    let total_packages: usize = suite_summaries.iter().map(|(_, _, c)| c).sum();
    let suites_list: Vec<String> = suite_summaries.iter().map(|(s, _, _)| s.clone()).collect();

    let mut top_release = format!(
        "Origin: {}\nLabel: {}\nDate: {}\nArchitectures: {}\nComponents: {}\nSuites: {}\nDescription: {} multi-suite repository (generated by lx repo)\n",
        args.origin,
        args.origin,
        date,
        all_arch_list.join(" "),
        components.join(" "),
        suites_list.join(" "),
        args.origin,
    );

    // Hash all per-suite Packages/Packages.gz/Release files.
    for (suite_name, _, _) in &suite_summaries {
        let suite_dists_dir = dists_dir.join(suite_name);
        let suite_out_dir = if suite_dists_dir.exists() {
            &suite_dists_dir
        } else {
            &args.dir
        };
        for index in ["Packages", "Packages.gz", "Release"] {
            let path = suite_out_dir.join(index);
            if path.exists() {
                let bytes = std::fs::read(&path)?;
                let rel_path = format!("dists/{}/{}", suite_name, index);
                top_release.push_str(&index_hashes(&rel_path, &bytes));
            }
        }
    }

    std::fs::write(args.dir.join("Release"), &top_release)?;
    println!(
        "\nwrote multi-suite Release ({} suites, {} packages total)",
        suite_summaries.len(),
        total_packages
    );

    sign_release(&args.dir, &top_release, args)?;

    Ok(())
}

/// Discover suites in the repository directory.
/// Returns Vec of (suite_name, directory_containing_debs).
fn discover_suites(dists_dir: &Path, root_dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut suites = Vec::new();

    if dists_dir.is_dir() {
        // dists/<suite>/ layout.
        for entry in std::fs::read_dir(dists_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with('.') {
                    suites.push((name, path));
                }
            }
        }
    }

    if suites.is_empty() {
        // Fallback: treat root_dir as a single suite if it has .deb files.
        let has_debs = std::fs::read_dir(root_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .any(|p| p.extension().map(|e| e == "deb").unwrap_or(false));
        if has_debs {
            suites.push(("stable".to_string(), root_dir.to_path_buf()));
        }
    }

    suites.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(suites)
}

/// Build a Packages index from a list of .deb files.
/// Returns (packages_content, archs_map).
fn build_packages_index(debs: &[PathBuf]) -> Result<(String, BTreeMap<String, bool>)> {
    build_packages_index_with_base(debs, "")
}

fn build_packages_index_with_base(
    debs: &[PathBuf],
    suite: &str,
) -> Result<(String, BTreeMap<String, bool>)> {
    let mut stanzas = Vec::new();
    let mut archs = BTreeMap::new();
    for deb in debs {
        let ctrl = read_control(deb)?;
        let name = deb.file_name().unwrap().to_string_lossy().to_string();
        let get = |k: &str| ctrl.get(k).cloned().unwrap_or_default();
        archs.insert(get("Architecture"), true);
        let size = std::fs::metadata(deb)?.len();
        let sha256 = lx_lib::checksum::sha256_file(deb)?;
        // Filename path: include suite prefix for multi-suite layout so apt
        // can find the file relative to the repo root.
        let filename = if suite.is_empty() {
            name
        } else {
            format!("dists/{}/{}", suite, name)
        };
        let stanza = format!(
            "Package: {}\nVersion: {}\nArchitecture: {}\nMaintainer: {}\nDepends: {}\nDescription: {}\nFilename: {}\nSize: {}\nSHA256: {}\n",
            get("Package"),
            get("Version"),
            get("Architecture"),
            get("Maintainer"),
            get("Depends"),
            get("Description"),
            filename,
            size,
            sha256,
        );
        stanzas.push(stanza.trim_end_matches('\n').to_string());
    }
    let packages = stanzas.join("\n\n") + "\n";
    Ok((packages, archs))
}

/// Sign the Release file if a key is provided.
fn sign_release(dir: &Path, release: &str, args: &RepoArgs) -> Result<()> {
    if let Some(key) = &args.sign_key {
        let req = lx_lib::sign::SignRequest {
            key_file: key,
            key_id: args.sign_key_id.as_deref().unwrap_or_default(),
            passphrase: None,
        };
        // `InRelease` is inline-clearsigned; `Release.gpg` is armored detached.
        let inline = lx_lib::sign::clearsign_inline(release.as_bytes(), &req)?;
        std::fs::write(dir.join("InRelease"), inline)?;
        let detached = lx_lib::sign::clearsign(release.as_bytes(), &req)?;
        std::fs::write(dir.join("Release.gpg"), detached)?;
        println!("wrote InRelease, Release.gpg (clearsigned)");
    } else {
        println!("no --sign-key: skipping InRelease (unsigned Release only)");
    }
    Ok(())
}

fn index_hashes(name: &str, bytes: &[u8]) -> String {
    use sha2::Digest;
    let md5 = format!("{:x}", md5::compute(bytes));
    let mut h1 = sha1::Sha1::new();
    h1.update(bytes);
    let sha1hex = hex::encode(h1.finalize());
    let mut h2 = sha2::Sha256::new();
    h2.update(bytes);
    let sha256hex = hex::encode(h2.finalize());
    format!(
        "MD5Sum:\n {md5} {} {name}\nSHA1:\n {sha1hex} {} {name}\nSHA256:\n {sha256hex} {} {name}\n",
        bytes.len(),
        bytes.len(),
        bytes.len()
    )
}

/// Read the `control` file out of a `.deb`'s control.tar.* member.
pub fn read_control(deb: &Path) -> Result<BTreeMap<String, String>> {
    let file = std::fs::File::open(deb).with_context(|| format!("opening '{}'", deb.display()))?;
    let mut archive = ar::Archive::new(file);
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry?;
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if !name.starts_with("control.tar") {
            continue;
        }
        let mut data = Vec::new();
        std::io::copy(&mut entry, &mut data)?;
        let text: Vec<u8> = if name.ends_with(".gz") {
            let mut gz = flate2::read::GzDecoder::new(data.as_slice());
            let mut out = Vec::new();
            std::io::copy(&mut gz, &mut out)?;
            out
        } else if name.ends_with(".xz") {
            let mut out = Vec::new();
            let mut dec = lzma_rust2::XzReader::new(data.as_slice(), true);
            std::io::copy(&mut dec, &mut out)?;
            out
        } else if name.ends_with(".zst") || name.ends_with(".zstd") {
            zstd::bulk::decompress(&data, 16 * 1024 * 1024)
                .context("zstd decompress control.tar")?
        } else {
            data
        };
        let mut ar = tar::Archive::new(text.as_slice());
        for member in ar.entries()? {
            let mut member = member?;
            let path = member.path()?.to_string_lossy().to_string();
            if path.ends_with("control") && member.header().entry_type().is_file() {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut member, &mut s)?;
                return Ok(parse_control(&s));
            }
        }
        bail!(
            "control.tar member has no control file in '{}'",
            deb.display()
        );
    }
    bail!("'{}' has no control.tar.* member", deb.display())
}

/// Parse RFC-2822-style control fields (with continuation lines). Shared
/// with the `opkg` repo-indexer plugin (`.ipk` controls use the same syntax).
pub fn parse_control(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut key = String::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            if !key.is_empty() {
                map.entry(key.clone()).and_modify(|v: &mut String| {
                    v.push('\n');
                    v.push_str(line.trim());
                });
            }
        } else if let Some((k, v)) = line.split_once(':') {
            key = k.trim().to_string();
            map.insert(key.clone(), v.trim().to_string());
        }
    }
    map
}
