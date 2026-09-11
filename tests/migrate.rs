// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

fn test_dirs(tag: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("lx-migrate-test-{}-{}", std::process::id(), tag));
    let data = base.join("data");
    let cache = base.join("cache");
    std::fs::create_dir_all(data.join("lpt")).unwrap();
    std::fs::create_dir_all(cache.join("lpt").join("downloads")).unwrap();
    (data, cache)
}

#[test]
fn migrate_moves_manifest_and_cache_idempotently() {
    let (data, cache) = test_dirs("state");
    std::fs::write(
        data.join("lpt").join("installed.json"),
        r#"{"packages":{}}"#,
    )
    .unwrap();
    std::fs::write(cache.join("lpt").join("downloads").join("f.deb"), b"cached").unwrap();

    // Point dirs:: at the sandbox via env (dirs honors XDG on Linux).
    std::env::set_var("XDG_DATA_HOME", &data);
    std::env::set_var("XDG_CACHE_HOME", &cache);

    lx_lib::migrate::run(lx_lib::migrate::MigrateArgs { repo: None }).unwrap();
    assert!(data.join("lx").join("installed.json").exists());
    assert!(cache.join("lx").join("downloads").join("f.deb").exists());

    // Second run is a no-op (new locations exist / old ones gone).
    lx_lib::migrate::run(lx_lib::migrate::MigrateArgs { repo: None }).unwrap();
    assert!(data.join("lx").join("installed.json").exists());

    std::env::remove_var("XDG_DATA_HOME");
    std::env::remove_var("XDG_CACHE_HOME");
    std::fs::remove_dir_all(data.parent().unwrap()).ok();
}

#[test]
fn migrate_rewrites_workflows() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".github").join("workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(
        wf.join("build.yml"),
        "- uses: ranjithrajv/lpt@v1\n- run: lpt build package.yaml\n  key: lpt-cache-x\n",
    )
    .unwrap();
    std::fs::write(wf.join("other.txt"), "lpt stays here\n").unwrap();

    lx_lib::migrate::run(lx_lib::migrate::MigrateArgs {
        repo: Some(dir.path().to_path_buf()),
    })
    .unwrap();
    let out = std::fs::read_to_string(wf.join("build.yml")).unwrap();
    assert!(out.contains("ranjithrajv/lx@v1"));
    assert!(out.contains("lx build package.yaml"));
    assert!(out.contains("lx-cache-x"));
    assert!(!out.contains("lpt"));
    // Non-yml files untouched.
    assert_eq!(
        std::fs::read_to_string(wf.join("other.txt")).unwrap(),
        "lpt stays here\n"
    );
}

#[test]
fn migrate_rejects_repo_without_workflows() {
    let dir = tempfile::tempdir().unwrap();
    assert!(lx_lib::migrate::run(lx_lib::migrate::MigrateArgs {
        repo: Some(dir.path().to_path_buf()),
    })
    .is_err());
}
