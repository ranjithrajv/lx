// SPDX-License-Identifier: GPL-3.0-or-later

//! `coverage-gaps` — list functions that are not 100% line-covered.
//!
//! Cross-references syn-extracted function spans (`covscan` JSON) with
//! `cargo llvm-cov --json` output. A function is reported as a gap if any
//! line in its `[start, end]` span has zero coverage; test-only functions
//! (name starts with `tests::`) are ignored.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};

type Functions = BTreeMap<String, Vec<(String, usize, usize)>>;
type FileCounts = BTreeMap<String, BTreeMap<usize, i64>>;

pub fn report(fns_path: &Path, cov_path: &Path) -> Result<()> {
    let fns: Functions = serde_json::from_slice(
        &std::fs::read(fns_path).with_context(|| format!("reading {}", fns_path.display()))?,
    )
    .with_context(|| format!("parsing {}", fns_path.display()))?;
    let cov: serde_json::Value = serde_json::from_slice(
        &std::fs::read(cov_path).with_context(|| format!("reading {}", cov_path.display()))?,
    )
    .with_context(|| format!("parsing {}", cov_path.display()))?;

    let per_file = line_counts(&cov);

    for (module, funcs) in &fns {
        for (name, start, end) in funcs {
            if name.starts_with("tests::") {
                continue;
            }
            let counts = per_file
                .iter()
                .find(|(file, _)| file.ends_with(&format!("/{module}")))
                .map(|(_, counts)| counts)
                .cloned()
                .unwrap_or_default();
            let uncovered: Vec<usize> = (*start..=*end)
                .filter(|line| counts.get(line) == Some(&0))
                .collect();
            if uncovered.is_empty() {
                println!("{module}::{name} [{start}-{end}] OK");
            } else {
                let lines = uncovered
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("{module}::{name} [{start}-{end}] lines {lines}");
            }
        }
    }
    Ok(())
}

/// Map each llvm-cov file to `line -> max count over segments on that line`.
fn line_counts(cov: &serde_json::Value) -> FileCounts {
    let mut per_file = FileCounts::new();
    let Some(files) = cov.pointer("/data/0/files").and_then(|v| v.as_array()) else {
        return per_file;
    };
    for file in files {
        let Some(name) = file.get("filename").and_then(|v| v.as_str()) else {
            continue;
        };
        let mut counts: BTreeMap<usize, i64> = BTreeMap::new();
        if let Some(segments) = file.get("segments").and_then(|v| v.as_array()) {
            for segment in segments {
                let Some(segment) = segment.as_array() else {
                    continue;
                };
                let has_count = segment.get(3).and_then(|v| v.as_bool()).unwrap_or(false);
                if !has_count {
                    continue;
                }
                let line = segment.first().and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let count = segment.get(2).and_then(|v| v.as_i64()).unwrap_or(0);
                let entry = counts.entry(line).or_insert(0);
                if count > *entry {
                    *entry = count;
                }
            }
        }
        per_file.insert(name.to_string(), counts);
    }
    per_file
}
