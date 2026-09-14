// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests freezing `lx repo` index output for the merged
//! `PackageIndex` plugin dimension (`lib/plugins/package_index/`).
//!
//! Each test builds a throwaway artifact, runs the real indexer, masks the
//! volatile/digest fields (timestamps, sizes, checksums), and compares a
//! SHA-256 of the normalized index to a stored constant. The golden captures
//! the index *shape* (fields, ordering, layout), not the artifact bytes, so
//! an unrelated change to an archive builder doesn't trip it — but a
//! dispatch/registry change does. Regenerate intentional changes with:
//!
//! ```text
//! UPDATE_GOLDENS=1 cargo test --test index_merge_golden -- --nocapture
//! ```
//!
//! The registry tests at the bottom assert the union registry covers every
//! backend (5 write + 3 read) with the right capabilities and id/alias
//! resolution.

use lx_lib::index::InstallOpts;
use lx_lib::plugins::package_index::{
    all_index_backends, artifacts_with_ext, get_index_backend, Capabilities, IndexOptions,
    PackageIndex, BACKEND_IDS,
};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Suite the arch/union snapshots use (the pacman db filename embeds it).
const GOLDEN_SUITE: &str = "golden";

/// Instantiate the write backend for a `--format`-style value.
fn indexer(fmt: &str) -> Box<dyn PackageIndex> {
    get_index_backend(fmt)
        .unwrap_or_else(|| panic!("no backend for '{fmt}'"))
        .make(fmt)
}

fn opts(suite: &str) -> IndexOptions<'_> {
    IndexOptions {
        suite,
        origin: "golden",
        components: "main",
        sign_key: None,
        sign_key_id: "",
    }
}

fn stage(root: &Path) {
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"golden-payload").unwrap();
}

fn build_deb(dir: &Path) -> PathBuf {
    let root = dir.join("root");
    stage(&root);
    let control = "Package: hello\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nDepends: libc6\nDescription: golden test\nSection: utils\n";
    let deb = dir.join("hello_1.0-1+bookworm_amd64.deb");
    lx_lib::debarchive::build(&root, control.as_bytes(), 1_735_689_600, &deb).unwrap();
    deb
}

fn build_ipk(dir: &Path) -> PathBuf {
    let root = dir.join("root");
    stage(&root);
    let control = "Package: hello\nVersion: 1.0-1\nArchitecture: x86_64\nMaintainer: T <t@e.c>\nDepends: libc\nProvides: hello-full\nSection: utils\nDescription: golden test\n";
    let ipk = dir.join("hello_1.0-1_x86_64.ipk");
    lx_lib::ipkarchive::build(&root, control.as_bytes(), 1_735_689_600, &ipk).unwrap();
    ipk
}

fn build_pkg(dir: &Path) -> PathBuf {
    let root = dir.join("root");
    stage(&root);
    let meta = lx_lib::archarchive::PackageMeta {
        name: "hello",
        version: "1.0",
        release: "1",
        description: "golden test",
        url: "https://example.com",
        license: "MIT",
    };
    let pkg = dir.join("hello-1.0-1-x86_64.pkg.tar.zst");
    lx_lib::archarchive::build(&root, &meta, "x86_64", 1_735_689_600, &pkg, None).unwrap();
    pkg
}

fn build_apk(dir: &Path) -> PathBuf {
    let root = dir.join("root");
    stage(&root);
    let depends = ["libc".to_string()];
    let meta = lx_lib::apkarchive::PackageMeta {
        name: "hello",
        version: "1.0.0-r1",
        description: "golden test",
        url: "https://example.com",
        license: "MIT",
        depends: &depends,
        provides: &[],
        replaces: &[],
    };
    let apk = dir.join("hello-1.0.0-r1.apk");
    lx_lib::apkarchive::build(&root, &meta, "x86_64", 1_735_689_600, &apk).unwrap();
    apk
}

fn build_rpm(dir: &Path) -> PathBuf {
    let root = dir.join("root");
    stage(&root);
    let meta = lx_lib::rpmarchive::PackageMeta {
        name: "hello",
        version: "1.0",
        release: "1",
        summary: "golden test",
        description: "golden test",
        license: "MIT",
        vendor: None,
        packager: None,
    };
    let rpm = dir.join("hello-1.0-1.x86_64.rpm");
    lx_lib::rpmarchive::build(&root, &meta, "x86_64", 1_735_689_600, &rpm).unwrap();
    rpm
}

fn hash(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Compare a normalized index snapshot to a stored digest. Set
/// `UPDATE_GOLDENS=1` to print the digest instead (for intentional updates).
fn assert_golden(label: &str, normalized: &str, expected: &str) {
    let actual = hash(normalized);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        eprintln!("GOLDEN {label} = {actual}");
        return;
    }
    assert_eq!(
        actual, expected,
        "golden mismatch for {label}\n\
         (re-run with UPDATE_GOLDENS=1 if this change is intentional)\n\
         --- normalized snapshot ---\n{normalized}\n---------------------------"
    );
}

fn apply(text: &str, rules: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (pattern, replacement) in rules {
        out = Regex::new(pattern)
            .unwrap()
            .replace_all(&out, *replacement)
            .to_string();
    }
    out
}

/// Mask checksums/sizes that derive from artifact bytes, so the golden
/// captures index structure rather than the artifact.
fn normalize_common(text: &str) -> String {
    apply(
        text,
        &[
            (r"(?m)^Date: .*$", "Date: <date>"),
            (r"\b[0-9a-f]{64}\b", "<sha256>"),
            (r"\b[0-9a-f]{40}\b", "<sha1>"),
            (r"\b[0-9a-f]{32}\b", "<md5>"),
            (r" \d+ (Packages|Packages\.gz)\b", " <n> $1"),
            (r"(?m)^Size: \d+$", "Size: <n>"),
            (r"(?m)^Installed-Size: \d+$", "Installed-Size: <n>"),
        ],
    )
}

fn normalize_pacman(text: &str) -> String {
    let text = normalize_common(text);
    apply(
        &text,
        &[
            (r"%CSIZE%\n\d+", "%CSIZE%\n<n>"),
            (r"%ISIZE%\n\d+", "%ISIZE%\n<n>"),
        ],
    )
}

fn normalize_apk(text: &str) -> String {
    let text = normalize_common(text);
    apply(
        &text,
        &[
            (r"(?m)^C:Q1.*$", "C:<sum>"),
            (r"(?m)^S:\d+$", "S:<n>"),
            (r"(?m)^I:\d+$", "I:<n>"),
        ],
    )
}

fn normalize_rpm(text: &str) -> String {
    let text = normalize_common(text);
    apply(
        &text,
        &[
            (r"<revision>\d+</revision>", "<revision><n></revision>"),
            (r"<timestamp>\d+</timestamp>", "<timestamp><n></timestamp>"),
            (
                r#"size package="\d+" installed="\d+" archive="\d+""#,
                r#"size package="<n>" installed="<n>" archive="<n>""#,
            ),
            (r"<size>\d+</size>", "<size><n></size>"),
            (r"<open-size>\d+</open-size>", "<open-size><n></open-size>"),
        ],
    )
}

/// Decompress a gzip stream to UTF-8 text.
fn gunzip_to_string(bytes: &[u8]) -> String {
    let mut s = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut s)
        .unwrap();
    s
}

/// Unpack a gzip'd tar into sorted `(path, text)` members.
fn tar_members(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        let mut text = String::new();
        entry.read_to_string(&mut text).unwrap();
        out.push((path, text));
    }
    out.sort();
    out
}

fn has_tool(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Normalized `Packages` + `Release` for an apt index in `dir`.
fn snapshot_deb(dir: &Path) -> String {
    format!(
        "==Packages==\n{}\n==Release==\n{}",
        normalize_common(&std::fs::read_to_string(dir.join("Packages")).unwrap()),
        normalize_common(&std::fs::read_to_string(dir.join("Release")).unwrap()),
    )
}

/// Normalized `Packages` for an opkg index in `dir`.
fn snapshot_ipk(dir: &Path) -> String {
    normalize_common(&std::fs::read_to_string(dir.join("Packages")).unwrap())
}

/// Normalized pacman db members for `GOLDEN_SUITE` in `dir`.
fn snapshot_arch(dir: &Path) -> String {
    let db = std::fs::read(dir.join(format!("{GOLDEN_SUITE}.db.tar.gz"))).unwrap();
    let mut out = String::new();
    for (path, text) in tar_members(&db) {
        out.push_str(&format!("=={path}==\n{}\n", normalize_pacman(&text)));
    }
    out
}

/// Normalized Alpine `APKINDEX.tar.gz` members in `dir`.
fn snapshot_apk(dir: &Path) -> String {
    let index = std::fs::read(dir.join("APKINDEX.tar.gz")).unwrap();
    let mut out = String::new();
    for (path, text) in tar_members(&index) {
        out.push_str(&format!("=={path}==\n{}\n", normalize_apk(&text)));
    }
    out
}

#[test]
fn golden_deb_index() {
    let tmp = tempfile::tempdir().unwrap();
    build_deb(tmp.path());
    let arts = artifacts_with_ext(tmp.path(), "deb").unwrap();
    indexer("deb")
        .build_index(tmp.path(), &arts, &opts("bookworm"))
        .unwrap();

    assert!(tmp.path().join("Packages.gz").is_file());
    assert_golden(
        "deb",
        &snapshot_deb(tmp.path()),
        "ef6b6ceae4c04ae7b2dffdfd7125f04ea7745adb11a2e83f228f0a5ce89a4d19",
    );
}

#[test]
fn golden_ipk_index() {
    let tmp = tempfile::tempdir().unwrap();
    build_ipk(tmp.path());
    let arts = artifacts_with_ext(tmp.path(), "ipk").unwrap();
    indexer("ipk")
        .build_index(tmp.path(), &arts, &opts("openwrt"))
        .unwrap();

    assert!(tmp.path().join("Packages.gz").is_file());
    assert_golden(
        "ipk",
        &snapshot_ipk(tmp.path()),
        "9de6a2b2647a1befe871957bba4683c7d5b980ffb722f0175135d149be6833df",
    );
}

#[test]
fn golden_arch_index() {
    let tmp = tempfile::tempdir().unwrap();
    build_pkg(tmp.path());
    let arts = artifacts_with_ext(tmp.path(), "zst").unwrap();
    indexer("arch")
        .build_index(tmp.path(), &arts, &opts("golden"))
        .unwrap();

    assert_golden(
        "arch",
        &snapshot_arch(tmp.path()),
        "1b71a251f263b49a0d78e008007c8091703a189bcd96d5fb309cd11ca27a0cdf",
    );
}

#[test]
fn golden_apk_index() {
    let tmp = tempfile::tempdir().unwrap();
    build_apk(tmp.path());
    let arts = artifacts_with_ext(tmp.path(), "apk").unwrap();
    indexer("apk")
        .build_index(tmp.path(), &arts, &opts("alpine"))
        .unwrap();

    assert_golden(
        "apk",
        &snapshot_apk(tmp.path()),
        "8665eb2aee79f08be6940a89b13b4a1c39fd888997801b58313ffd107b5ddaf3",
    );
}

#[test]
fn golden_rpm_index() {
    // The rpm indexer reads metadata with the host `rpm` CLI.
    if !has_tool("rpm") {
        eprintln!("skipping: rpm CLI unavailable");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    build_rpm(tmp.path());
    let arts = artifacts_with_ext(tmp.path(), "rpm").unwrap();
    indexer("rpm")
        .build_index(tmp.path(), &arts, &opts("stable"))
        .unwrap();

    // No digest golden here: rpm index output requires the host `rpm` CLI,
    // so it cannot be regenerated (or verified) everywhere. Assert the
    // dispatch and metadata shape structurally instead.
    let repomd =
        normalize_rpm(&std::fs::read_to_string(tmp.path().join("repodata/repomd.xml")).unwrap());
    let primary = normalize_rpm(&gunzip_to_string(
        &std::fs::read(tmp.path().join("repodata/primary.xml.gz")).unwrap(),
    ));
    assert!(primary.contains("<name>hello</name>"), "{primary}");
    assert!(primary.contains("<arch>x86_64</arch>"), "{primary}");
    assert!(
        primary.contains("<version epoch=\"0\" ver=\"1.0\" rel=\"1\"/>"),
        "{primary}"
    );
    assert!(
        primary.contains("<checksum type=\"sha256\" pkgid=\"YES\"><sha256></checksum>"),
        "{primary}"
    );
    assert!(repomd.contains("<data type=\"primary\">"), "{repomd}");
    assert!(
        repomd.contains("<location href=\"repodata/primary.xml.gz\"/>"),
        "{repomd}"
    );
}

#[test]
fn golden_deb_multi_suite() {
    let tmp = tempfile::tempdir().unwrap();
    let bookworm = tmp.path().join("dists/bookworm");
    let trixie = tmp.path().join("dists/trixie");
    std::fs::create_dir_all(&bookworm).unwrap();
    std::fs::create_dir_all(&trixie).unwrap();
    build_deb(&bookworm);
    // Same package name, different suite so both suites have one entry.
    std::fs::rename(
        bookworm.join("hello_1.0-1+bookworm_amd64.deb"),
        bookworm.join("hello_1.0-1_amd64.deb"),
    )
    .unwrap();
    build_deb(&trixie);
    std::fs::rename(
        trixie.join("hello_1.0-1+bookworm_amd64.deb"),
        trixie.join("hello_1.0-1_amd64.deb"),
    )
    .unwrap();

    lx_lib::repo::run(lx_lib::repo::RepoArgs {
        dir: tmp.path().to_path_buf(),
        format: Some("deb".to_string()),
        suite: "stable".to_string(),
        multi_suite: true,
        components: "main".to_string(),
        origin: "golden".to_string(),
        sign_key: None,
        sign_key_id: None,
    })
    .unwrap();

    let mut normalized = String::new();
    for rel in [
        "Release",
        "dists/bookworm/Packages",
        "dists/bookworm/Release",
        "dists/trixie/Packages",
        "dists/trixie/Release",
    ] {
        let text = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        normalized.push_str(&format!("=={rel}==\n{}\n", normalize_common(&text)));
    }
    assert_golden(
        "deb-multi-suite",
        &normalized,
        "90b01222df766087da6f03c432cbc0e34bc13dc5c1cb0373d07f6fe059c40b3d",
    );
}

// ---------------------------------------------------------------------------
// PackageIndex union registry (merge step 2)
// ---------------------------------------------------------------------------

#[test]
fn union_registry_covers_all_backends() {
    let ids: Vec<&str> = all_index_backends().iter().map(|b| b.id).collect();
    assert_eq!(ids, BACKEND_IDS, "registry order drifted from BACKEND_IDS");
    assert_eq!(ids.len(), 9, "expected 5 write + 4 read backends");
    assert_eq!(
        get_index_backend("deb").unwrap().capabilities,
        Capabilities::WRITE
    );
    assert_eq!(
        get_index_backend("repology").unwrap().capabilities,
        Capabilities::READ
    );
    let write = all_index_backends()
        .iter()
        .filter(|b| b.capabilities.can_write())
        .count();
    let read = all_index_backends()
        .iter()
        .filter(|b| b.capabilities.can_read())
        .count();
    assert_eq!((write, read), (5, 4), "capability split changed");
}

#[test]
fn union_registry_reports_capabilities() {
    for backend in all_index_backends() {
        let (want_read, want_write) = match backend.id {
            "apt" | "opkg" | "pacman" | "apk" | "rpm" => (false, true),
            "lx-community" | "aur" | "repology" | "custom" => (true, false),
            other => panic!("unexpected backend '{other}'"),
        };
        assert_eq!(
            backend.capabilities.can_read(),
            want_read,
            "read capability wrong for '{}'",
            backend.id
        );
        assert_eq!(
            backend.capabilities.can_write(),
            want_write,
            "write capability wrong for '{}'",
            backend.id
        );
    }
}

#[test]
fn union_lookup_accepts_ids_and_format_aliases() {
    assert_eq!(get_index_backend("deb").unwrap().id, "apt");
    assert_eq!(get_index_backend("arch").unwrap().id, "pacman");
    assert_eq!(get_index_backend("ipk").unwrap().id, "opkg");
    assert_eq!(
        get_index_backend("lx-community").unwrap().id,
        "lx-community"
    );
    assert!(get_index_backend("nope").is_none());

    // Read backends carry the configured instance name for `IndexHit::source`
    // and the `--repo` filter, while `id()` stays canonical.
    let aur = get_index_backend("aur").unwrap().make("my-aur");
    assert_eq!(aur.id(), "aur");
    assert_eq!(aur.instance_name(), "my-aur");

    // A read backend reports the read role; write-role methods fail with an
    // actionable error rather than panicking, and Repology stays
    // metadata-only.
    let repology = get_index_backend("repology").unwrap().make("repology");
    assert!(repology.capabilities().can_read());
    assert!(repology
        .build_index(Path::new("."), &[], &opts("stable"))
        .is_err());
    assert!(repology.install("curl", InstallOpts::default()).is_err());
}
