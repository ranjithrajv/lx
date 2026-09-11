use lx_lib::optimize::*;

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
