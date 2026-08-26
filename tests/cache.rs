use anyhow::Result;
use lpt_lib::cache::*;
use std::path::Path;
use std::time::SystemTime;

/// The download closure passed to the cache (used by the real callers too).
type DownloadFn = Box<dyn Fn(&str, &Path) -> Result<()> + Sync>;

fn fake_download(count: &'static std::sync::atomic::AtomicUsize) -> DownloadFn {
    Box::new(move |_: &str, out: &Path| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::fs::write(out, b"payload").unwrap();
        Ok(())
    })
}

#[test]
fn caches_second_fetch() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DownloadCache::new(dir.path().join("c")).unwrap();
    let count = Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
    let dl = fake_download(count);

    let out1 = dir.path().join("a.bin");
    let out2 = dir.path().join("b.bin");
    cache.fetch("https://x/a.bin", &out1, None, &dl).unwrap();
    cache.fetch("https://x/a.bin", &out2, None, &dl).unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(&out2).unwrap(), b"payload");
}

#[test]
fn different_urls_dont_collide() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DownloadCache::new(dir.path().join("c")).unwrap();
    let count = Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
    let dl = fake_download(count);
    cache
        .fetch("https://x/a", &dir.path().join("a"), None, &dl)
        .unwrap();
    cache
        .fetch("https://x/b", &dir.path().join("b"), None, &dl)
        .unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn verifies_checksum_on_fresh_download() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DownloadCache::new(dir.path().join("c")).unwrap();
    let sum = lpt_lib::checksum::sha256_file(&{
        let p = dir.path().join("tmp");
        std::fs::write(&p, b"payload").unwrap();
        p
    })
    .unwrap();
    let dl: DownloadFn = Box::new(|_: &str, out: &Path| -> Result<()> {
        std::fs::write(out, b"payload").unwrap();
        Ok(())
    });
    let out = dir.path().join("out");
    cache.fetch("https://x/a", &out, Some(&sum), &dl).unwrap();
    // Corrupt payload must fail against the same pin.
    let dl_bad: DownloadFn = Box::new(|_: &str, out: &Path| -> Result<()> {
        std::fs::write(out, b"tampered").unwrap();
        Ok(())
    });
    let out2 = dir.path().join("out2");
    assert!(cache
        .fetch("https://x/b", &out2, Some(&sum), &dl_bad)
        .is_err());
}

#[test]
fn reuses_checksummed_cache_fast_path() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DownloadCache::new(dir.path().join("c")).unwrap();
    let sum = lpt_lib::checksum::sha256_file(&{
        let p = dir.path().join("tmp");
        std::fs::write(&p, b"payload").unwrap();
        p
    })
    .unwrap();
    let count = Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
    let dl = fake_download(count);
    let out1 = dir.path().join("out1");
    let out2 = dir.path().join("out2");
    cache.fetch("https://x/a", &out1, Some(&sum), &dl).unwrap();
    cache.fetch("https://x/a", &out2, Some(&sum), &dl).unwrap();
    // Second fetch must come from cache: no re-download.
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(&out2).unwrap(), b"payload");
}

#[test]
fn redownloads_when_cached_checksum_mismatches() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DownloadCache::new(dir.path().join("c")).unwrap();
    let sum = lpt_lib::checksum::sha256_file(&{
        let p = dir.path().join("tmp");
        std::fs::write(&p, b"payload").unwrap();
        p
    })
    .unwrap();
    let count = Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
    let dl = fake_download(count);
    let out = dir.path().join("out");
    cache.fetch("https://x/a", &out, Some(&sum), &dl).unwrap();
    // Corrupt the cached copy so the fast-path checksum fails.
    let key = cache.key("https://x/a", Some(&sum));
    std::fs::write(dir.path().join("c").join(key), b"corrupted").unwrap();
    let out2 = dir.path().join("out2");
    cache.fetch("https://x/a", &out2, Some(&sum), &dl).unwrap();
    // Cache invalidated -> re-downloaded.
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&out2).unwrap(), b"payload");
}

#[test]
fn under_lock_recheck_reuses_cache_after_another_worker_downloads() {
    use std::sync::Barrier;
    let dir = tempfile::tempdir().unwrap();
    let cache = std::sync::Arc::new(DownloadCache::new(dir.path().join("c")).unwrap());
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count_check = count.clone();
    let barrier = std::sync::Arc::new(Barrier::new(2));

    // A single shared download closure (not two identical copies, which
    // the linker would ICF-merge and break per-line attribution). Slow so
    // the racing thread blocks on flock while this worker holds it, then
    // lands in the locked re-check (cache valid).
    let dl: DownloadFn = Box::new(move |_: &str, out: &Path| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(80));
        std::fs::write(out, b"payload").unwrap();
        Ok(())
    });

    std::thread::scope(|s| {
        let t1 = s.spawn(|| {
            barrier.wait();
            cache.fetch("https://x/a", &dir.path().join("out_a"), None, &dl)
        });
        let t2 = s.spawn(|| {
            barrier.wait();
            cache.fetch("https://x/a", &dir.path().join("out_b"), None, &dl)
        });
        t1.join().unwrap().unwrap();
        t2.join().unwrap().unwrap();
    });

    // Exactly one worker downloaded; the other found the cache under lock.
    assert_eq!(count_check.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(dir.path().join("out_a")).unwrap(), b"payload");
    assert_eq!(std::fs::read(dir.path().join("out_b")).unwrap(), b"payload");
}

#[test]
fn under_lock_recheck_reuses_checksummed_cache() {
    use std::sync::Barrier;
    let dir = tempfile::tempdir().unwrap();
    let cache = std::sync::Arc::new(DownloadCache::new(dir.path().join("c")).unwrap());
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count_check = count.clone();
    let barrier = std::sync::Arc::new(Barrier::new(2));
    // Expected checksum of "payload".
    let sum = {
        let p = dir.path().join("seed");
        std::fs::write(&p, b"payload").unwrap();
        lpt_lib::checksum::sha256_file(&p).unwrap()
    };

    let dl: DownloadFn = Box::new(move |_: &str, out: &Path| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(80));
        std::fs::write(out, b"payload").unwrap();
        Ok(())
    });

    std::thread::scope(|s| {
        let t1 = s.spawn(|| {
            barrier.wait();
            cache.fetch("https://x/c", &dir.path().join("out_a"), Some(&sum), &dl)
        });
        let t2 = s.spawn(|| {
            barrier.wait();
            cache.fetch("https://x/c", &dir.path().join("out_b"), Some(&sum), &dl)
        });
        t1.join().unwrap().unwrap();
        t2.join().unwrap().unwrap();
    });

    assert_eq!(count_check.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(dir.path().join("out_a")).unwrap(), b"payload");
    assert_eq!(std::fs::read(dir.path().join("out_b")).unwrap(), b"payload");
}

#[test]
fn stale_cache_entry_is_redownloaded() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DownloadCache::new(dir.path().join("c")).unwrap();
    let count = Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
    let dl = fake_download(count);
    let out = dir.path().join("out");
    cache.fetch("https://x/a", &out, None, &dl).unwrap();
    // Age the cache entry past the TTL by rewriting its mtime into the past.
    let key = cache.key("https://x/a", None);
    let cache_file = dir.path().join("c").join(&key);
    let old = std::time::SystemTime::now()
        - std::time::Duration::from_secs(2 * lpt_lib::constants::DOWNLOAD_CACHE_TTL_SECS);
    let f = std::fs::File::open(&cache_file).unwrap();
    f.set_modified(old).unwrap();
    let out2 = dir.path().join("out2");
    cache.fetch("https://x/a", &out2, None, &dl).unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn try_lock_reports_error() {
    // An O_PATH descriptor is a valid fd (so File::from_raw_fd won't
    // panic on -1) but flock() rejects it with EBADF.
    use std::os::unix::io::FromRawFd;
    let dir = tempfile::tempdir().unwrap();
    let path = std::ffi::CString::new(dir.path().as_os_str().as_encoded_bytes()).unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_PATH) };
    assert!(fd >= 0, "open(O_PATH) failed");
    let f = unsafe { std::fs::File::from_raw_fd(fd) };
    assert!(lpt_lib::cache::lock_file::FileLock::try_lock(f).is_err());
}

#[test]
fn valid_entry_branch_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let cache_file = dir.path().join("c").join("entry");
    let meta_file = dir.path().join("c").join("entry.meta");
    std::fs::create_dir_all(dir.path().join("c")).unwrap();
    std::fs::write(&cache_file, b"payload").unwrap();
    std::fs::write(&meta_file, "https://x/a").unwrap();
    let now = SystemTime::now();
    let fresh = || std::fs::metadata(&cache_file).and_then(|m| m.modified());

    // Fresh entry, matching URL.
    assert!(valid_entry(
        &cache_file,
        &meta_file,
        "https://x/a",
        now,
        Some(fresh())
    ));
    // Missing cache file.
    assert!(!valid_entry(
        &dir.path().join("nope"),
        &meta_file,
        "https://x/a",
        now,
        Some(fresh())
    ));
    // Missing meta file.
    assert!(!valid_entry(
        &cache_file,
        &dir.path().join("nope.meta"),
        "https://x/a",
        now,
        Some(fresh())
    ));
    // Meta URL mismatch.
    assert!(!valid_entry(
        &cache_file,
        &meta_file,
        "https://x/other",
        now,
        Some(fresh())
    ));
    // Stale entry (mtime older than TTL).
    let old_mtime = now - std::time::Duration::from_secs(2 * CACHE_TTL_SECS);
    assert!(!valid_entry(
        &cache_file,
        &meta_file,
        "https://x/a",
        now,
        Some(Ok(old_mtime))
    ));
    // mtime in the future (duration_since fails).
    let future = now + std::time::Duration::from_secs(3600);
    assert!(!valid_entry(
        &cache_file,
        &meta_file,
        "https://x/a",
        now,
        Some(Ok(future))
    ));
    // metadata() failure.
    assert!(!valid_entry(
        &cache_file,
        &meta_file,
        "https://x/a",
        now,
        Some(Err(std::io::Error::other("boom")))
    ));
    // modified() failure surfaces as Err too.
    assert!(!valid_entry(
        &cache_file,
        &meta_file,
        "https://x/a",
        now,
        Some(Err(std::io::Error::other("boom")))
    ));
}
