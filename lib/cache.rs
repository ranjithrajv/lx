// SPDX-License-Identifier: GPL-3.0-or-later

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

pub const CACHE_TTL_SECS: u64 = crate::constants::DOWNLOAD_CACHE_TTL_SECS;

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
    pub fn key(&self, url: &str, checksum: Option<&str>) -> String {
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

/// JSON API cache (5 min TTL) shared by `github` and `gitlab` clients.
/// Mirrors the action's `github_api_cache` semantics.
pub struct ApiCache {
    dir: Option<PathBuf>,
}

impl ApiCache {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let path = dir.join(format!("{key}.json"));
        let Ok(meta) = std::fs::metadata(&path) else {
            return Ok(None);
        };
        let Ok(modified) = meta.modified() else {
            return Ok(None);
        };
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default();
        if age > std::time::Duration::from_secs(crate::constants::API_CACHE_TTL_SECS) {
            return Ok(None);
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        match serde_json::from_str(&text) {
            Ok(v) => Ok(Some(v)),
            Err(_) => Ok(None),
        }
    }

    pub fn put<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let _ = std::fs::create_dir_all(dir);
        let json = serde_json::to_string(value)?;
        let path = dir.join(format!("{key}.json"));
        let tmp = dir.join(format!("{key}.json.tmp"));
        let _ = std::fs::write(&tmp, json);
        let _ = std::fs::rename(&tmp, &path);
        Ok(())
    }

    pub fn get_or_fetch<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        if let Some(hit) = self.get(key)? {
            return Ok(hit);
        }
        let value = fetch()?;
        self.put(key, &value)?;
        Ok(value)
    }
}

/// Pure decision kernel for [`DownloadCache::valid`], factored out so every
/// branch (stale entry, future mtime, unreadable metadata) is deterministically
/// testable. `modified` is the result of `metadata().modified()`.
pub fn valid_entry(
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

/// Input-keyed store for *built* package artifacts (not downloads). The key
/// is sha256 of recipe material (config + build-affecting flags + asset
/// digest + format + dist + arch); same inputs means the built bytes are
/// guaranteed identical, so a hit copies the cached file instead of
/// re-running the packaging pipeline. Foundation for a future remote
/// substituter (same key scheme, just fetched over HTTP instead of read
/// from disk).
pub struct ArtifactCache {
    dir: PathBuf,
}

impl ArtifactCache {
    pub fn new(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create artifact cache dir '{}'", dir.display()))?;
        Ok(Self { dir })
    }

    /// Content-based key over arbitrary recipe material (caller composes
    /// the string from whatever inputs affect the output bytes).
    pub fn key(material: &str) -> String {
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        hasher.update(material.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Path to the cached artifact for `key`, if present.
    pub fn get(&self, key: &str) -> Option<PathBuf> {
        let p = self.dir.join(key);
        if p.is_file() {
            Some(p)
        } else {
            None
        }
    }

    /// Cache `artifact`, remembering its original file name (needed to
    /// reconstruct the output path on a later hit -- the key itself carries
    /// no naming information).
    pub fn put(&self, key: &str, artifact: &Path, original_name: &str) -> Result<()> {
        std::fs::copy(artifact, self.dir.join(key))
            .with_context(|| format!("failed to cache artifact '{}'", artifact.display()))?;
        std::fs::write(self.dir.join(format!("{key}.name")), original_name)?;
        Ok(())
    }

    pub fn name_for(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(format!("{key}.name"))).ok()
    }
}

pub mod lock_file {
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

/// Forge clients that keep an optional on-disk JSON API cache.
///
/// The eight forge clients each hand-wrote the same `api_cache` wrapper around
/// [`ApiCache::get_or_fetch`]; this default method replaces them.
pub trait ApiCacheProvider {
    /// Directory for the JSON API cache, or `None` to fetch every time.
    fn api_cache_dir(&self) -> Option<PathBuf>;

    /// Fetch `key` through the cache, or directly when no cache dir is set.
    fn api_cache<T>(&self, key: &str, fetch: impl FnOnce() -> Result<T>) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone,
    {
        ApiCache::new(self.api_cache_dir()).get_or_fetch(key, fetch)
    }
}
