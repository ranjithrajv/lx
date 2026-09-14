// SPDX-License-Identifier: GPL-3.0-or-later

//! Real-client and live-endpoint verification for outputs that are otherwise
//! only unit-tested (spec + fixtures).
//!
//! These are opt-in / environment-gated: each test skips cleanly when the
//! relevant client or network opt-in is unavailable, so the normal suite
//! stays hermetic.
//!
//! * `pacman_accepts_generated_db` — runs whenever `pacman` is on `PATH`.
//! * `opkg_accepts_generated_index` — set `LX_OPKG=<opkg>`.
//! * `dnf_accepts_generated_repodata` — set `LX_DNF=<dnf>`.
//! * `gitee_live_*` / `sourceforge_live_*` — set `LX_NETWORK_TESTS=1`.

use lx_lib::plugins::package_index::{get_index_backend, IndexOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

fn opts(suite: &str) -> IndexOptions<'_> {
    IndexOptions {
        suite,
        origin: "test",
        components: "main",
        sign_key: None,
        sign_key_id: "",
    }
}

/// Whether we can obtain a (fake) root euid — already root, or an
/// unprivileged user namespace maps one. `pacman` refuses `-Sy` otherwise.
fn can_root() -> bool {
    if unsafe { libc::geteuid() } == 0 {
        return true;
    }
    which("unshare")
        .map(|u| {
            Command::new(u)
                .args(["-r", "true"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Run `program` as (fake) root via an unprivileged user namespace when the
/// caller isn't already root.
fn rootify(program: &Path) -> Command {
    if unsafe { libc::geteuid() } == 0 {
        return Command::new(program);
    }
    if let Some(unshare) = which("unshare") {
        let mut c = Command::new(unshare);
        c.arg("-r").arg(program);
        c
    } else {
        Command::new(program)
    }
}

fn which(tool: &str) -> Option<PathBuf> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        None
    } else {
        Some(p.into())
    }
}

fn stage(root: &Path) {
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"x").unwrap();
}

fn tiny_arch_pkg(dir: &Path) -> PathBuf {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("stage");
    stage(&root);
    let pkg = dir.join("hello-1.0-1-x86_64.pkg.tar.zst");
    let meta = lx_lib::archarchive::PackageMeta {
        name: "hello",
        version: "1.0",
        release: "1",
        description: "test",
        url: "https://example.com",
        license: "MIT",
    };
    lx_lib::archarchive::build(&root, &meta, "x86_64", 1_735_689_600, &pkg, None).unwrap();
    pkg
}

/// Real `pacman -Sy` must accept our generated `<repo>.db.tar.gz` and expose
/// the package.
#[test]
fn pacman_accepts_generated_db() {
    let Some(pacman) = which("pacman") else {
        eprintln!("skipping: pacman not on PATH");
        return;
    };
    if !can_root() {
        eprintln!("skipping: pacman -Sy needs root (or an unprivileged userns)");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo/x86_64");
    std::fs::create_dir_all(&repo).unwrap();
    let pkg = tiny_arch_pkg(&repo);
    get_index_backend("arch")
        .unwrap()
        .make_writer("arch")
        .unwrap()
        .build_index(&repo, &[pkg], &opts("core"))
        .unwrap();
    // pacman fetches `<repo>.db`; expose the tar.gz under that name.
    std::fs::copy(repo.join("core.db.tar.gz"), repo.join("core.db")).unwrap();

    let root = tmp.path().join("root");
    std::fs::create_dir_all(root.join("db")).unwrap();
    std::fs::create_dir_all(root.join("cache")).unwrap();
    std::fs::create_dir_all(root.join("gnupg")).unwrap();
    let conf = tmp.path().join("pacman.conf");
    std::fs::write(
        &conf,
        format!(
            "[options]\n\
             RootDir = {r}\n\
             DBPath = {r}/db\n\
             CacheDir = {r}/cache\n\
             LogFile = {r}/pacman.log\n\
             GPGDir = {r}/gnupg\n\
             SigLevel = Never\n\
             \n[core]\nServer = file://{srv}\n",
            r = root.display(),
            srv = repo.display(),
        ),
    )
    .unwrap();

    let sync = rootify(&pacman)
        .args(["--sync", "--refresh", "--noconfirm", "--config"])
        .arg(&conf)
        .output()
        .expect("run pacman -Sy");
    assert!(
        sync.status.success(),
        "pacman -Sy failed:\n{}{}",
        String::from_utf8_lossy(&sync.stdout),
        String::from_utf8_lossy(&sync.stderr)
    );

    let info = rootify(&pacman)
        .args(["--sync", "--info", "hello", "--config"])
        .arg(&conf)
        .output()
        .expect("run pacman -Si hello");
    assert!(
        info.status.success(),
        "pacman -Si hello failed:\n{}{}",
        String::from_utf8_lossy(&info.stdout),
        String::from_utf8_lossy(&info.stderr)
    );
    assert!(String::from_utf8_lossy(&info.stdout).contains("hello"));
}

/// Real `opkg` (set `LX_OPKG=<opkg>`) must consume our `Packages.gz`.
#[test]
fn opkg_accepts_generated_index() {
    let Some(opkg) = std::env::var_os("LX_OPKG") else {
        eprintln!("skipping: set LX_OPKG=<opkg> to run");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    // A tiny ipk fixture.
    let root = tmp.path().join("stage");
    stage(&root);
    let ipk = tmp.path().join("hello_1.0-1_x86_64.ipk");
    let control = "Package: hello\nVersion: 1.0-1\nArchitecture: x86_64\nMaintainer: t <t@e>\nDepends: libc\nDescription: test\nSection: utils\n";
    lx_lib::ipkarchive::build(&root, control.as_bytes(), 1_735_689_600, &ipk).unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::copy(&ipk, repo.join("hello_1.0-1_x86_64.ipk")).unwrap();
    get_index_backend("ipk")
        .unwrap()
        .make_writer("ipk")
        .unwrap()
        .build_index(
            &repo,
            &[repo.join("hello_1.0-1_x86_64.ipk")],
            &opts("openwrt"),
        )
        .unwrap();

    let rootfs = tmp.path().join("rootfs");
    std::fs::create_dir_all(rootfs.join("var/lib/opkg/lists")).unwrap();
    let conf = tmp.path().join("opkg.conf");
    std::fs::write(
        &conf,
        format!(
            "dest root {}\nlists_dir ext {}/var/lib/opkg/lists\nsrc/gz test file://{}\n",
            rootfs.display(),
            rootfs.display(),
            repo.display()
        ),
    )
    .unwrap();

    let out = Command::new(&opkg)
        .args(["--conf"])
        .arg(&conf)
        .arg("update")
        .output()
        .expect("run opkg update");
    assert!(
        out.status.success(),
        "opkg update failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn tiny_rpm(dir: &Path) -> Option<PathBuf> {
    use lx_lib::plugins::Packager;
    let tmp = tempfile::tempdir().ok()?;
    let bin = tmp.path().join("binary");
    std::fs::create_dir_all(&bin).ok()?;
    let mut elf = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    elf.extend(vec![0u8; 100]);
    std::fs::write(bin.join("hello"), &elf).ok()?;
    let cfg = lx_lib::config::PackageConfig {
        package_name: "hello".to_string(),
        github_repo: "owner/hello".to_string(),
        ..Default::default()
    };
    let job = lx_lib::build::ResolvedJob {
        dist: "fedora".to_string(),
        arch: "amd64".to_string(),
        asset: lx_lib::github::Asset {
            name: "hello.tar.gz".to_string(),
            size: None,
            browser_download_url: String::new(),
            checksums: Default::default(),
        },
        tag: "v1.0.0".to_string(),
        published_at: Some(1_735_689_600),
    };
    let staging = tmp.path().join("staging");
    std::fs::create_dir_all(&staging).ok()?;
    let ctx = lx_lib::plugins::BuildContext {
        cfg: &cfg,
        job: &job,
        binary_dir: &bin,
        staging_root: &staging,
        license: None,
        debian_version: "1.0.0",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
        detected_deps: Vec::new(),
    };
    let built = lx_lib::plugins::rpm::RpmPackager.build(&ctx).ok()?;
    let dest = dir.join(built.file_name()?);
    std::fs::copy(&built, &dest).ok()?;
    Some(dest)
}

/// Real `dnf` (set `LX_DNF=<dnf>`) must consume our `repodata/`.
#[test]
fn dnf_accepts_generated_repodata() {
    let Some(dnf) = std::env::var_os("LX_DNF") else {
        eprintln!("skipping: set LX_DNF=<dnf> to run");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let Some(rpm) = tiny_rpm(&repo) else {
        eprintln!("skipping: could not build an rpm fixture");
        return;
    };
    get_index_backend("rpm")
        .unwrap()
        .make_writer("rpm")
        .unwrap()
        .build_index(&repo, &[rpm], &opts("test"))
        .unwrap();

    let out = Command::new(&dnf)
        .args([
            "--disablerepo=*",
            "--setopt=reposdir=/dev/null",
            &format!("--repofrompath=test,{}", repo.display()),
            "--repo=test",
            "--nogpgcheck",
            "makecache",
        ])
        .output()
        .expect("run dnf makecache");
    assert!(
        out.status.success(),
        "dnf makecache failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Live Gitee API: `latest_release` parses a real release.
#[test]
fn gitee_live_release() {
    if std::env::var("LX_NETWORK_TESTS").is_err() {
        eprintln!("skipping: set LX_NETWORK_TESTS=1 to run");
        return;
    }
    let client = lx_lib::gitee::GiteeClient::new(None).unwrap();
    let rel = client.latest_release("mindspore", "mindspore").unwrap();
    assert!(!rel.tag_name.is_empty(), "empty tag");
    assert!(rel.html_url.contains("gitee.com"));

    // `release_by_tag` (tags endpoint, with list-scan fallback) resolves the
    // same tag.
    let by_tag = client
        .release_by_tag("mindspore", "mindspore", &rel.tag_name)
        .unwrap();
    assert_eq!(by_tag.tag_name, rel.tag_name);
}

/// Live SourceForge feed: real files parse, with an inline md5.
#[test]
fn sourceforge_live_files() {
    if std::env::var("LX_NETWORK_TESTS").is_err() {
        eprintln!("skipping: set LX_NETWORK_TESTS=1 to run");
        return;
    }
    let client = lx_lib::sourceforge::SourceForgeClient::new(None).unwrap();
    let rel = client.latest_release("sevenzip").unwrap();
    assert!(!rel.assets.is_empty(), "no files parsed");
    assert!(
        rel.assets.iter().any(|a| a.checksums.contains_key("md5")),
        "no inline md5 parsed from the feed"
    );
}
