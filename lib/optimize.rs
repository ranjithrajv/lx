//! Resource-aware build optimization, mirroring the action's
//! `ci-optimization.sh` (dynamic parallelism) and `resource-pool.sh`
//! (graceful degradation). On CI runners the action caps parallelism from
//! known runner specs; locally it falls back to system detection and does
//! not enforce limits. We keep the same spirit: auto-tune `max_parallel`
//! from detected resources, warn when the requested level exceeds capacity.

/// Detected system resources (fall back to conservative defaults when the
/// host exposes nothing).
#[derive(Debug, Clone, Copy)]
pub struct SystemResources {
    pub cores: usize,
    pub memory_mb: usize,
    pub disk_gb: usize,
}

impl SystemResources {
    pub fn detect() -> Self {
        Self {
            cores: detect_cores(),
            memory_mb: detect_memory_mb(),
            disk_gb: detect_disk_gb(),
        }
    }

    /// Mirror `is_arch_supported_for_dist`-style conservative defaults.
    pub fn is_ci_environment() -> bool {
        std::env::var_os("GITHUB_ACTIONS").is_some_and(|v| v == "true")
            || std::env::var_os("GITLAB_CI").is_some_and(|v| v == "true")
            || std::env::var_os("TF_BUILD").is_some()
            || std::env::var_os("JENKINS_URL").is_some()
            || std::env::var_os("CIRCLECI").is_some()
            || std::env::var_os("TRAVIS").is_some()
            || std::env::var_os("BITBUCKET_BUILD_NUMBER").is_some()
    }
}

/// Optimal concurrent jobs from resources, mirroring
/// `calculate_optimal_parallel_jobs`: 2 GB RAM, 1 core, 5 GB disk per job,
/// take the most restrictive, cap at 8, reserve one job for CI overhead.
pub fn optimal_parallel_jobs(res: &SystemResources) -> usize {
    let mem_limit = res.memory_mb / 2048;
    let cpu_limit = res.cores;
    let disk_limit = res.disk_gb / 5;

    let mut limit = mem_limit.min(cpu_limit).min(disk_limit);
    if limit < 1 {
        limit = 1;
    }
    if SystemResources::is_ci_environment() {
        limit = limit.saturating_sub(1).max(1);
    }
    limit.min(8)
}

/// Choose the effective parallelism for a given resource set, mirroring
/// `apply_ci_optimizations` + `apply_graceful_degradation`: prefer the
/// user's value when lower than the resource optimum, else use the optimum.
/// Pure so tests are deterministic.
pub fn resolve_max_parallel(requested: usize, res: &SystemResources) -> usize {
    let optimum = optimal_parallel_jobs(res);
    if requested == 0 || requested > optimum {
        optimum
    } else {
        requested
    }
}

/// Detect system resources and pick the effective parallelism. Warns when
/// the requested level exceeds capacity (graceful degradation).
pub fn effective_max_parallel(requested: usize) -> usize {
    let res = SystemResources::detect();
    let optimum = optimal_parallel_jobs(&res);
    let effective = resolve_max_parallel(requested, &res);

    if requested > optimum {
        eprintln!(
            "  ⚠️  resource constraints: reducing parallel builds from {requested} to {optimum} \
             ({} cores, {} MB RAM, {} GB disk)",
            res.cores, res.memory_mb, res.disk_gb
        );
    }

    if effective == 1 {
        eprintln!(
            "  ℹ️  resource-based optimization: {effective} concurrent build job (sequential)"
        );
    } else {
        eprintln!("  ℹ️  resource-based optimization: {effective} concurrent build jobs");
    }
    effective
}

fn detect_cores() -> usize {
    if let Ok(out) = std::process::Command::new("nproc").output() {
        if let Ok(s) = String::from_utf8(out.stdout) {
            if let Ok(n) = s.trim().parse::<usize>() {
                if n > 0 {
                    return n;
                }
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") {
        let count = text.lines().filter(|l| l.starts_with("processor")).count();
        if count > 0 {
            return count;
        }
    }
    2
}

fn detect_memory_mb() -> usize {
    if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                if let Ok(kb) = rest.trim().trim_end_matches("kB").trim().parse::<usize>() {
                    return kb / 1024;
                }
            }
        }
    }
    if let Ok(out) = std::process::Command::new("free").output() {
        if let Ok(s) = String::from_utf8(out.stdout) {
            for line in s.lines() {
                let mut parts = line.split_whitespace();
                if parts.next() == Some("Mem:") {
                    if let Some(kb) = parts.next().and_then(|v| v.parse::<usize>().ok()) {
                        return kb / 1024;
                    }
                }
            }
        }
    }
    4096
}

fn detect_disk_gb() -> usize {
    use std::ffi::CString;
    let cwd = std::env::current_dir().ok();
    let path = cwd
        .as_ref()
        .and_then(|p| p.to_str())
        .and_then(|s| CString::new(s).ok());
    if let Some(path) = path {
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: statvfs writes to the uninitialized buffer on success.
        let rc = unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) };
        if rc == 0 {
            // SAFETY: rc == 0 guarantees the buffer is initialized.
            let stat = unsafe { stat.assume_init() };
            let free_bytes = stat.f_bavail * stat.f_frsize;
            if free_bytes > 0 {
                return (free_bytes / (1024 * 1024 * 1024)) as usize;
            }
        }
    }
    if let Ok(out) = std::process::Command::new("df").args(["-BG", "."]).output() {
        if let Ok(s) = String::from_utf8(out.stdout) {
            for line in s.lines().skip(1) {
                let mut parts = line.split_whitespace();
                parts.next(); // filesystem
                parts.next(); // 1G-blocks
                parts.next(); // Used
                if let Some(avail) = parts
                    .next()
                    .and_then(|v| v.trim_end_matches('G').parse::<usize>().ok())
                {
                    return avail;
                }
            }
        }
    }
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimum_respects_most_restrictive() {
        let res = SystemResources {
            cores: 8,
            memory_mb: 2048,
            disk_gb: 40,
        };
        assert_eq!(optimal_parallel_jobs(&res), 1); // 2GB -> 1 job
    }

    #[test]
    fn optimum_capped_at_8() {
        let res = SystemResources {
            cores: 64,
            memory_mb: 262144,
            disk_gb: 1000,
        };
        assert_eq!(optimal_parallel_jobs(&res), 8);
    }

    #[test]
    fn effective_prefers_user_when_lower() {
        // Requested 1 always wins over a higher optimum.
        let res = SystemResources {
            cores: 16,
            memory_mb: 65536,
            disk_gb: 200,
        };
        let optimum = optimal_parallel_jobs(&res);
        assert!(optimum > 1);
        assert_eq!(resolve_max_parallel(1, &res), 1);
    }

    #[test]
    fn effective_auto_uses_optimum() {
        let res = SystemResources {
            cores: 16,
            memory_mb: 65536,
            disk_gb: 200,
        };
        let optimum = optimal_parallel_jobs(&res);
        assert_eq!(resolve_max_parallel(0, &res), optimum);
    }

    #[test]
    fn effective_caps_overrequested() {
        let res = SystemResources {
            cores: 4,
            memory_mb: 4096,
            disk_gb: 20,
        };
        let optimum = optimal_parallel_jobs(&res);
        assert_eq!(resolve_max_parallel(99, &res), optimum);
    }
}
