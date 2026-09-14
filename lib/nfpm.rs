// SPDX-License-Identifier: GPL-3.0-or-later

//! `nfpm.yaml` → `package.yaml` conversion, backing `lx init --from-nfpm`.
//!
//! nfpm and `lx` share the same conceptual model — files go in, a native
//! package comes out — but disagree on the surface: nfpm uses `name`/`arch`/
//! `release` and a `contents:` DSL, `lx` uses `package_name`/`architectures`/
//! `build_version` and a `package.yaml`. This module maps the parts that line
//! up one-to-one, converts the contents DSL (including `file_info`,
//! `expand`, and `disown_subtree`), and reports everything it could not map
//! instead of dropping it silently.
//!
//! The result is deliberately a *starter*: keys nfpm has and `lx` does not
//! (e.g. `changelog`, `mtime`) are called out in a generated footer, and the
//! required `github_repo` gets a placeholder the user must fix.

use anyhow::{Context, Result};
use serde_yaml::{Mapping, Value};
use std::path::Path;

/// Read `path` and convert it to `package.yaml` text.
pub fn convert_file(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read nfpm config '{}'", path.display()))?;
    convert_str(&text).with_context(|| format!("converting '{}'", path.display()))
}

/// Convert an in-memory `nfpm.yaml` document to `package.yaml` text.
pub fn convert_str(text: &str) -> Result<String> {
    let raw: Value = serde_yaml::from_str(text).context("failed to parse nfpm.yaml")?;
    let src = raw
        .as_mapping()
        .context("nfpm.yaml must be a YAML mapping at the top level")?;

    let mut out = Mapping::new();
    let mut warnings: Vec<String> = Vec::new();

    let name = get_str(src, "name").unwrap_or_default();
    if name.is_empty() {
        anyhow::bail!("nfpm.yaml has no 'name' field");
    }
    insert_string(&mut out, "package_name", &name);
    // nfpm has no source repo concept; lx needs one for forge builds and for
    // the config to parse at all. Force a placeholder the user must review.
    insert_string(&mut out, "github_repo", &format!("OWNER/{name}"));
    warnings.push(format!(
        "github_repo is required by lx but nfpm has no equivalent — set it to the real 'owner/repo' (currently 'OWNER/{name}')."
    ));

    // --- identity / versioning -------------------------------------------------
    copy_string(src, &mut out, "version", "version");
    copy_string(src, &mut out, "version_schema", "version_schema");
    copy_string(src, &mut out, "release", "build_version");
    copy_string(src, &mut out, "epoch", "epoch");
    // nfpm splits semver into version/prerelease/version_metadata; lx keeps a
    // single version string, so fold the parts back in when present.
    let prerelease = get_str(src, "prerelease");
    let metadata = get_str(src, "version_metadata");
    if (prerelease.as_deref().is_some_and(|p| !p.is_empty()))
        || (metadata.as_deref().is_some_and(|m| !m.is_empty()))
    {
        if let Some(v) = get_str(src, "version") {
            let mut folded = v;
            if let Some(p) = prerelease.as_deref().filter(|p| !p.is_empty()) {
                folded.push('-');
                folded.push_str(p);
            }
            if let Some(m) = metadata.as_deref().filter(|m| !m.is_empty()) {
                folded.push('+');
                folded.push_str(m);
            }
            insert_string(&mut out, "version", &folded);
            warnings.push(
                "nfpm `prerelease`/`version_metadata` were folded into `version` (lx keeps one version string)."
                    .to_string(),
            );
        }
    }
    // nfpm `mtime:` (RFC 3339) maps to lx's reproducible-build timestamp.
    if let Some(m) = get_str(src, "mtime") {
        insert_string(&mut out, "mtime", &m);
    }

    if let Some(arch) = get_str(src, "arch") {
        match map_arch(&arch) {
            Some(debian_arch) => {
                let seq = vec![Value::String(debian_arch.to_string())];
                out.insert(Value::String("architectures".into()), Value::Sequence(seq));
            }
            None if arch.eq_ignore_ascii_case("all") => {
                warnings.push(
                    "nfpm arch 'all' has no direct lx equivalent; left the architecture matrix unset (lx auto-discovers it)."
                        .to_string(),
                );
            }
            None => {
                warnings.push(format!(
                    "nfpm arch '{arch}' has no known lx/Debian spelling; set `architectures:` manually."
                ));
            }
        }
    }

    // --- metadata --------------------------------------------------------------
    copy_string(src, &mut out, "maintainer", "maintainer");
    copy_string(src, &mut out, "description", "description");
    copy_string(src, &mut out, "vendor", "vendor");
    if let Some(homepage) = get_str(src, "homepage") {
        warnings.push(format!(
            "nfpm `homepage: {homepage}` has no direct lx field; lx derives the homepage from github_repo. Point github_repo at the project if you want this URL."
        ));
    }
    copy_string(src, &mut out, "license", "license_spdx");
    copy_string(src, &mut out, "section", "section");
    copy_string(src, &mut out, "priority", "priority");
    copy_bool(src, &mut out, "disable_globbing", "disable_globbing");

    if let Some(umask) = src.get(Value::String("umask".into())) {
        match umask {
            Value::Number(n) => {
                if let Some(m) = n.as_u64() {
                    insert_string(&mut out, "umask", &format!("0o{m:o}"));
                }
            }
            Value::String(s) => insert_string(&mut out, "umask", s),
            _ => {}
        }
    }

    // --- dependency relations (nfpm lists -> lx comma strings) ------------------
    for key in [
        "depends",
        "recommends",
        "suggests",
        "conflicts",
        "replaces",
        "provides",
    ] {
        if let Some(joined) = join_seq(src, key) {
            insert_string(&mut out, key, &joined);
        }
    }

    // --- scripts ---------------------------------------------------------------
    if let Some(scripts) = get_map(src, "scripts") {
        let mut dst = Mapping::new();
        for key in ["preinstall", "postinstall", "preremove", "postremove"] {
            copy_string(scripts, &mut dst, key, key);
        }
        if !dst.is_empty() {
            out.insert(Value::String("scripts".into()), Value::Mapping(dst));
        }
    }

    // --- per-format blocks -----------------------------------------------------
    convert_rpm(src, &mut out, &mut warnings);
    convert_deb(src, &mut out, &mut warnings);
    convert_arch(src, &mut out, &mut warnings);
    convert_apk(src, &mut out, &mut warnings);
    convert_ipk(src, &mut out, &mut warnings);
    convert_msix(src, &mut out, &mut warnings);
    convert_signature(src, &mut out, &mut warnings);
    convert_overrides(src, &mut out, &mut warnings);

    // --- contents --------------------------------------------------------------
    if let Some(contents) = src.get(Value::String("contents".into())) {
        let converted = convert_contents(contents, &mut warnings)?;
        if !converted.is_empty() {
            out.insert(Value::String("contents".into()), Value::Sequence(converted));
        }
    }

    // --- keys nfpm has that lx cannot express --------------------------------
    const HANDLED: &[&str] = &[
        "name",
        "version",
        "version_schema",
        "release",
        "prerelease",
        "version_metadata",
        "epoch",
        "arch",
        "maintainer",
        "description",
        "vendor",
        "homepage",
        "license",
        "section",
        "priority",
        "disable_globbing",
        "umask",
        "depends",
        "recommends",
        "suggests",
        "conflicts",
        "replaces",
        "provides",
        "scripts",
        "rpm",
        "deb",
        "archlinux",
        "apk",
        "ipk",
        "msix",
        "overrides",
        "contents",
        "mtime",
        "changelog",
        "platform",
    ];
    for key in src.keys() {
        let Some(key) = key.as_str() else { continue };
        if !HANDLED.contains(&key) {
            warnings.push(format!(
                "nfpm `{key}:` was not mapped; review and add it manually."
            ));
        }
    }
    if src.contains_key(Value::String("changelog".into())) {
        warnings.push(
            "nfpm `changelog:` (chglog YAML) has no lx equivalent; lx auto-generates the Debian changelog from the release."
                .to_string(),
        );
    }
    if src.contains_key(Value::String("platform".into())) {
        warnings.push(
            "nfpm `platform:` is not mapped; lx targets Linux for Linux formats and Windows for `msix`."
                .to_string(),
        );
    }

    render(&out, &warnings)
}

fn render(out: &Mapping, warnings: &[String]) -> Result<String> {
    let mut body =
        String::from("# Converted from nfpm.yaml by `lx init --from-nfpm` — REVIEW ME.\n");
    body.push_str(&serde_yaml::to_string(out).context("serializing converted config")?);
    if !warnings.is_empty() {
        body.push_str("\n# --- review notes (not valid config) ---\n");
        for w in warnings {
            body.push_str(&format!("# - {w}\n"));
        }
    }
    Ok(body)
}

// ---------------------------------------------------------------------------
// Per-format blocks
// ---------------------------------------------------------------------------

fn convert_rpm(src: &Mapping, out: &mut Mapping, warnings: &mut Vec<String>) {
    let Some(rpm) = get_map(src, "rpm") else {
        return;
    };
    if let Some(compression) = get_str(rpm, "compression") {
        merge_nested(out, "rpm", {
            let mut m = Mapping::new();
            insert_string(&mut m, "compression", &compression);
            m
        });
    }
    if let Some(scripts) = get_map(rpm, "scripts") {
        let mut s = Mapping::new();
        for key in ["pretrans", "posttrans", "verify"] {
            copy_string(scripts, &mut s, key, key);
        }
        if !s.is_empty() {
            merge_nested(out, "scripts", s);
        }
    }
    if let Some(packager) = get_str(rpm, "packager") {
        insert_string(out, "packager", &packager);
    }
    // rpm.group / rpm.buildhost / rpm.prefixes.
    let mut block = Mapping::new();
    copy_string(rpm, &mut block, "group", "group");
    copy_string(rpm, &mut block, "buildhost", "buildhost");
    if let Some(prefixes) = rpm.get(Value::String("prefixes".into())) {
        if !prefixes.is_null() {
            block.insert(Value::String("prefixes".into()), prefixes.clone());
        }
    }
    // rpm.requires.post -> rpm.requires_post.
    if let Some(requires) = get_map(rpm, "requires") {
        if let Some(post) = requires.get(Value::String("post".into())) {
            if !post.is_null() {
                block.insert(Value::String("requires_post".into()), post.clone());
            }
        }
    }
    if !block.is_empty() {
        merge_nested(out, "rpm", block);
    }
    if rpm.contains_key(Value::String("summary".into())) {
        warnings.push(
            "nfpm `rpm.summary:` is not mapped; lx uses the top-level `description` as the RPM summary."
                .to_string(),
        );
    }
}

fn convert_deb(src: &Mapping, out: &mut Mapping, _warnings: &mut Vec<String>) {
    let Some(deb) = get_map(src, "deb") else {
        return;
    };
    copy_string(deb, out, "arch_variant", "arch_variant");
    if let Some(compression) = get_str(deb, "compression") {
        insert_string(out, "compression", &compression);
    }
    if let Some(joined) = join_seq(deb, "breaks") {
        insert_string(out, "breaks", &joined);
    }
    if let Some(joined) = join_seq(deb, "predepends") {
        insert_string(out, "predepends", &joined);
    }
    if let Some(fields) = deb.get(Value::String("fields".into())) {
        out.insert(Value::String("fields".into()), fields.clone());
    }

    let mut block = Mapping::new();
    if let Some(scripts) = get_map(deb, "scripts") {
        for key in ["rules", "templates", "config"] {
            copy_string(scripts, &mut block, key, key);
        }
    }
    if let Some(triggers) = get_map(deb, "triggers") {
        let trigger_map = [
            ("interest", "triggers_interest"),
            ("interest_await", "triggers_interest_await"),
            ("interest_noawait", "triggers_interest_noawait"),
            ("activate", "triggers_activate"),
            ("activate_await", "triggers_activate_await"),
            ("activate_noawait", "triggers_activate_noawait"),
        ];
        for (src_key, dst_key) in trigger_map {
            if let Some(seq) = triggers.get(Value::String(src_key.into())) {
                block.insert(Value::String(dst_key.into()), seq.clone());
            }
        }
    }
    if !block.is_empty() {
        merge_nested(out, "deb", block);
    }
}

fn convert_arch(src: &Mapping, out: &mut Mapping, warnings: &mut Vec<String>) {
    let Some(arch) = get_map(src, "archlinux") else {
        return;
    };
    if let Some(scripts) = get_map(arch, "scripts") {
        let mut s = Mapping::new();
        copy_string(scripts, &mut s, "preupgrade", "preupgrade_script");
        copy_string(scripts, &mut s, "postupgrade", "postupgrade_script");
        if !s.is_empty() {
            merge_nested(out, "scripts", s);
        }
    }
    if let Some(packager) = get_str(arch, "packager") {
        insert_string(out, "packager", &packager);
    }
    for key in ["pkgbase", "arch"] {
        if arch.contains_key(Value::String(key.into())) {
            warnings.push(format!("nfpm `archlinux.{key}:` is not mapped."));
        }
    }
}

fn convert_apk(src: &Mapping, out: &mut Mapping, _warnings: &mut [String]) {
    let Some(apk) = get_map(src, "apk") else {
        return;
    };
    if let Some(scripts) = get_map(apk, "scripts") {
        let mut s = Mapping::new();
        copy_string(scripts, &mut s, "preupgrade", "preupgrade_script");
        copy_string(scripts, &mut s, "postupgrade", "postupgrade_script");
        if !s.is_empty() {
            merge_nested(out, "scripts", s);
        }
    }
}

fn convert_ipk(src: &Mapping, out: &mut Mapping, _warnings: &mut Vec<String>) {
    let Some(ipk) = get_map(src, "ipk") else {
        return;
    };
    if let Some(joined) = join_seq(ipk, "predepends") {
        insert_string(out, "predepends", &joined);
    }
    if let Some(fields) = ipk.get(Value::String("fields".into())) {
        out.insert(Value::String("fields".into()), fields.clone());
    }
    // nfpm `ipk:` fields map onto lx's `ipk:` block.
    let mut block = Mapping::new();
    for key in [
        "alternatives",
        "tags",
        "abi_version",
        "auto_installed",
        "essential",
    ] {
        if let Some(v) = ipk.get(Value::String(key.into())) {
            if !v.is_null() {
                block.insert(Value::String(key.into()), v.clone());
            }
        }
    }
    if !block.is_empty() {
        out.insert(Value::String("ipk".into()), Value::Mapping(block));
    }
}

fn convert_msix(src: &Mapping, out: &mut Mapping, warnings: &mut Vec<String>) {
    let Some(msix) = get_map(src, "msix") else {
        return;
    };
    let mut block = Mapping::new();
    for key in [
        "arch",
        "publisher",
        "identity",
        "properties",
        "applications",
        "dependencies",
        "capabilities",
    ] {
        if let Some(v) = msix.get(Value::String(key.into())) {
            if !v.is_null() {
                block.insert(Value::String(key.into()), v.clone());
            }
        }
    }
    if let Some(sig) = get_map(msix, "signature") {
        if get_str(sig, "pfx_file").is_some_and(|p| !p.is_empty()) {
            warnings.push(
                "nfpm `msix.signature.pfx_file` is not supported; set `signature.key_file` (PKCS#8 PEM) and `signature.cert_file` (PEM) for native MSIX signing."
                    .to_string(),
            );
        }
    }
    if !block.is_empty() {
        out.insert(Value::String("msix".into()), Value::Mapping(block));
    }
}

fn convert_signature(src: &Mapping, out: &mut Mapping, warnings: &mut Vec<String>) {
    let mut sig = Mapping::new();
    // key_file / key_id are shared across formats; first non-empty wins.
    for block in ["deb", "rpm", "apk"] {
        let Some(sig_block) = get_map(src, block).and_then(|m| get_map(m, "signature")) else {
            continue;
        };
        if get_str(&sig, "key_file").is_none() {
            if let Some(v) = get_str(sig_block, "key_file") {
                insert_string(&mut sig, "key_file", &v);
            }
        }
        if get_str(&sig, "key_id").is_none() {
            if let Some(v) = get_str(sig_block, "key_id") {
                insert_string(&mut sig, "key_id", &v);
            }
        }
    }
    if let Some(deb) = get_map(src, "deb") {
        if let Some(s) = get_map(deb, "signature") {
            if let Some(method) = get_str(s, "method") {
                match method.as_str() {
                    "debsign" => insert_string(&mut sig, "method", "debsign"),
                    other => warnings.push(format!(
                        "nfpm deb signature method '{other}' is not supported by lx (only debsign/detach); left unset."
                    )),
                }
            }
            copy_string(s, &mut sig, "type", "type");
        }
    }
    if !sig.is_empty() {
        out.insert(Value::String("signature".into()), Value::Mapping(sig));
    }
}

fn convert_overrides(src: &Mapping, out: &mut Mapping, warnings: &mut Vec<String>) {
    let Some(overrides) = get_map(src, "overrides") else {
        return;
    };
    let mut dst = Mapping::new();
    for (key, value) in overrides {
        let Some(name) = key.as_str() else { continue };
        let lx_name = match name {
            "archlinux" => "arch",
            "deb" | "rpm" | "apk" | "ipk" | "msix" => name,
            other => {
                warnings.push(format!(
                    "nfpm overrides.{other}: has no lx packager; dropped."
                ));
                continue;
            }
        };
        let Some(vmap) = value.as_mapping() else {
            continue;
        };
        let mut entry = Mapping::new();
        for relation in [
            "depends",
            "recommends",
            "suggests",
            "conflicts",
            "replaces",
            "provides",
            "breaks",
        ] {
            if let Some(joined) = join_seq(vmap, relation) {
                insert_string(&mut entry, relation, &joined);
            }
        }
        if !entry.is_empty() {
            dst.insert(Value::String(lx_name.into()), Value::Mapping(entry));
        }
    }
    if !dst.is_empty() {
        out.insert(Value::String("overrides".into()), Value::Mapping(dst));
    }
}

fn convert_contents(contents: &Value, warnings: &mut Vec<String>) -> Result<Vec<Value>> {
    let Some(seq) = contents.as_sequence() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in seq {
        let Some(map) = entry.as_mapping() else {
            continue;
        };
        let mut dst = Mapping::new();
        for key in ["src", "dst", "type", "packager", "expand"] {
            if let Some(v) = map.get(Value::String(key.into())) {
                if !v.is_null() {
                    dst.insert(Value::String(key.into()), v.clone());
                }
            }
        }
        if let Some(file_info) = get_map(map, "file_info") {
            let mut fi = Mapping::new();
            copy_string(file_info, &mut fi, "owner", "owner");
            copy_string(file_info, &mut fi, "group", "group");
            copy_string(file_info, &mut fi, "lang", "lang");
            if let Some(mode) = file_info.get(Value::String("mode".into())) {
                if let Some(octal) = mode_as_octal(mode) {
                    insert_string(&mut fi, "mode", &octal);
                } else {
                    warnings.push(format!(
                        "contents file_info.mode '{mode:?}' is not an integer/string; dropped."
                    ));
                }
            }
            if let Some(mtime) = file_info.get(Value::String("mtime".into())) {
                if let Some(s) = value_to_string(mtime) {
                    insert_string(&mut fi, "mtime", &s);
                }
            }
            if !fi.is_empty() {
                dst.insert(Value::String("file_info".into()), Value::Mapping(fi));
            }
        }
        if let Some(disown) = map.get(Value::String("disown_subtree".into())) {
            if !disown.is_null() {
                dst.insert(Value::String("disown_subtree".into()), disown.clone());
            }
        }
        if dst.is_empty() {
            continue;
        }
        if !dst.contains_key(Value::String("dst".into())) {
            warnings.push("a contents entry had no `dst`; dropped.".to_string());
            continue;
        }
        out.push(Value::Mapping(dst));
    }
    Ok(out)
}

/// nfpm writes `mode` as a YAML integer (its `os.FileMode` value) or an octal
/// string. Normalize to lx's explicit `"0oNNN"` string form.
fn mode_as_octal(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => n.as_u64().map(|m| format!("0o{m:o}")),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Small helpers over serde_yaml values
// ---------------------------------------------------------------------------

fn get<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(Value::String(key.to_string()))
}

fn get_map<'a>(map: &'a Mapping, key: &str) -> Option<&'a Mapping> {
    get(map, key).and_then(Value::as_mapping)
}

fn get_str(map: &Mapping, key: &str) -> Option<String> {
    get(map, key).and_then(value_to_string)
}

fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn join_seq(map: &Mapping, key: &str) -> Option<String> {
    let seq = get(map, key)?.as_sequence()?;
    let parts: Vec<String> = seq.iter().filter_map(value_to_string).collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn insert_string(map: &mut Mapping, key: &str, value: &str) {
    map.insert(
        Value::String(key.to_string()),
        Value::String(value.to_string()),
    );
}

fn copy_string(src: &Mapping, dst: &mut Mapping, src_key: &str, dst_key: &str) {
    if let Some(v) = get_str(src, src_key) {
        insert_string(dst, dst_key, &v);
    }
}

fn copy_bool(src: &Mapping, dst: &mut Mapping, src_key: &str, dst_key: &str) {
    if let Some(Value::Bool(b)) = get(src, src_key) {
        dst.insert(Value::String(dst_key.to_string()), Value::Bool(*b));
    }
}

/// Insert/merge `block` under `key` in `out`, preserving a sibling block's keys.
fn merge_nested(out: &mut Mapping, key: &str, block: Mapping) {
    let existing = out.get_mut(Value::String(key.to_string()));
    if let Some(Value::Mapping(existing)) = existing {
        for (k, v) in block {
            existing.insert(k, v);
        }
        return;
    }
    out.insert(Value::String(key.to_string()), Value::Mapping(block));
}

/// Map an nfpm/Go architecture name to its Debian spelling, when one exists.
fn map_arch(arch: &str) -> Option<&'static str> {
    match arch.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" => Some("amd64"),
        "arm64" | "aarch64" => Some("arm64"),
        "386" | "i386" | "i686" => Some("i386"),
        "arm" | "arm7" | "armhf" => Some("armhf"),
        "armel" => Some("armel"),
        "ppc64le" | "ppc64el" => Some("ppc64el"),
        "s390x" => Some("s390x"),
        "riscv64" => Some("riscv64"),
        "loong64" | "loongarch64" => Some("loong64"),
        _ => None,
    }
}
