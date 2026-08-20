use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Content-addressed download cache mirroring the action's
/// `download_with_cache`: cache key = sha256(`url[:checksum]`), 24h TTL,
/// `.meta` sidecar records the source URL. Concurrency-safe via per-entry
/// lock files (no external deps).
pub struct DownloadCache {
    dir: PathBuf,
}

const CACHE_TTL_SECS: u64 = 86_400; // 24h, matching the action

impl DownloadCache {
    /// Create (or reuse) the cache directory.
    pub fn new(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create cache dir '{}'", dir.display()))?;
        Ok(Self { dir })
    }

    /// Fetch `url` into `output`, populating the cache on first use.
    /// `expected_sha256` (when present) is verified against both cached and
    /// freshly-downloaded files (strict, matching the action's hard-fail on
    /// checksum mismatch).
    pub fn fetch(
        &self,
        url: &str,
        output: &Path,
        expected_sha256: Option<&str>,
        download: &(dyn Fn(&str, &Path) -> Result<()> + Sync),
    ) -> Result<()> {
        let key = self.key(url, expected_sha256);
        let cache_file = self.dir.join(&key);
        let meta_file = self.dir.join(format!("{key}.meta"));

        // Fast path: valid, non-stale cache entry.
        if let Some(entry) = self.try_reuse(&cache_file, &meta_file, url, expected_sha256)? {
            std::fs::copy(entry, output)?;
            eprintln!("  (cache) using cached download");
            return Ok(());
        }

        // Slow path under a per-entry lock to avoid duplicate parallel
        // downloads (mirrors the action's flock).
        let lock_path = self.dir.join(format!("{key}.lock"));
        let lock = std::fs::File::create(&lock_path)
            .with_context(|| format!("failed to create lock '{}'", lock_path.display()))?;
        let guard = lock_file::FileLock::try_lock(lock)?;
        let _guard = guard;

        // Re-check under the lock (another worker may have cached it).
        if let Some(entry) = self.try_reuse(&cache_file, &meta_file, url, expected_sha256)? {
            std::fs::copy(entry, output)?;
            eprintln!("  (cache) using cached download");
            return Ok(());
        }

        download(url, output)?;
        if let Some(h) = expected_sha256 {
            if !self.checksum_matches(output, h)? {
                return Err(anyhow::anyhow!(
                    "checksum verification failed for '{}': expected {h}",
                    output.display()
                ));
            }
        }
        std::fs::copy(output, &cache_file)?;
        std::fs::write(&meta_file, url)
            .with_context(|| format!("failed to write meta '{}'", meta_file.display()))?;
        Ok(())
    }

    /// Returns the cache file to reuse if the entry is valid and (when a
    /// checksum is pinned) its content verifies; otherwise `None`. Invalid
    /// checksummed entries are evicted so the next fetch re-downloads.
    fn try_reuse<'a>(
        &self,
        cache_file: &'a Path,
        meta_file: &Path,
        url: &str,
        expected_sha256: Option<&str>,
    ) -> Result<Option<&'a Path>> {
        if !self.valid(cache_file, meta_file, url) {
            return Ok(None);
        }
        match expected_sha256 {
            Some(h) => {
                if self.checksum_matches(cache_file, h)? {
                    Ok(Some(cache_file))
                } else {
                    // Checksum mismatch in cache: re-download.
                    let _ = std::fs::remove_file(cache_file);
                    let _ = std::fs::remove_file(meta_file);
                    Ok(None)
                }
            }
            None => Ok(Some(cache_file)),
        }
    }

    /// Content-based cache key: sha256 of `url[:checksum]`.
    fn key(&self, url: &str, checksum: Option<&str>) -> String {
        let material = match checksum {
            Some(c) => format!("{url}:{c}"),
            None => url.to_string(),
        };
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        hasher.update(material.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Cache entry is present, fresh (<24h), and matches the source URL.
    fn valid(&self, cache_file: &Path, meta_file: &Path, url: &str) -> bool {
        valid_entry(
            cache_file,
            meta_file,
            url,
            SystemTime::now(),
            Some(std::fs::metadata(cache_file).and_then(|m| m.modified())),
        )
    }

    fn checksum_matches(&self, file: &Path, expected: &str) -> Result<bool> {
        let actual = crate::checksum::sha256_file(file)?;
        Ok(actual.eq_ignore_ascii_case(expected))
    }
}

/// Pure decision kernel for [`DownloadCache::valid`], factored out so every
/// branch (stale entry, future mtime, unreadable metadata) is deterministically
/// testable. `modified` is the result of `metadata().modified()`.
fn valid_entry(
    cache_file: &Path,
    meta_file: &Path,
    url: &str,
    now: SystemTime,
    modified: Option<std::io::Result<SystemTime>>,
) -> bool {
    if !cache_file.is_file() || !meta_file.is_file() {
        return false;
    }
    if std::fs::read_to_string(meta_file).ok().as_deref() != Some(url) {
        return false;
    }
    match modified {
        Some(Ok(mtime)) => match now.duration_since(mtime) {
            Ok(age) => age.as_secs() < CACHE_TTL_SECS,
            Err(_) => false, // mtime in the future
        },
        // Unreadable metadata or modified() failure.
        Some(Err(_)) | None => false,
    }
}

mod lock_file {
    use anyhow::Result;
    use std::fs::File;
    use std::os::unix::io::AsRawFd;

    /// Blocking advisory lock on a file descriptor. Releases on drop.
    pub struct FileLock(File);

    impl FileLock {
        pub fn try_lock(file: File) -> Result<Self> {
            let fd = file.as_raw_fd();
            let rc = unsafe { libc::flock(fd, libc::LOCK_EX) };
            if rc != 0 {
                return Err(anyhow::anyhow!("failed to lock cache entry"));
            }
            Ok(Self(file))
        }
    }

    impl Drop for FileLock {
        fn drop(&mut self) {
            let fd = self.0.as_raw_fd();
            unsafe { libc::flock(fd, libc::LOCK_UN) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let sum = crate::checksum::sha256_file(&{
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
        let sum = crate::checksum::sha256_file(&{
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
        let sum = crate::checksum::sha256_file(&{
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
            crate::checksum::sha256_file(&p).unwrap()
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
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86_400);
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
        assert!(lock_file::FileLock::try_lock(f).is_err());
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
}
