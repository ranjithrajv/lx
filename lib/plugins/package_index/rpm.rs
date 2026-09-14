// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{IndexOptions, PackageIndex, WriteIndex};
use crate::plugins::plugin::plugin_identity;

/// RPM (dnf/yum/zypper) repository: a `repodata/repomd.xml` plus a gzipped
/// `primary.xml`. Package metadata is read with `rpm -qp` (the same host-tool
/// fallback `convert.rs` uses for RPM input).
pub struct RpmIndexer;

plugin_identity!(
    RpmIndexer,
    "rpm",
    "RPM repository (repodata/repomd.xml + primary.xml.gz)"
);

impl PackageIndex for RpmIndexer {}

impl WriteIndex for RpmIndexer {
    fn file_extension(&self) -> Option<&'static str> {
        Some("rpm")
    }

    fn build_index(&self, dir: &Path, artifacts: &[PathBuf], _opts: &IndexOptions) -> Result<()> {
        if artifacts.is_empty() {
            bail!("no .rpm files in '{}'", dir.display());
        }
        let mut primary = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <metadata xmlns=\"http://linux.duke.edu/metadata/common\" \
             xmlns:rpm=\"http://linux.duke.edu/metadata/rpm\" packages=\"",
        );
        primary.push_str(&artifacts.len().to_string());
        primary.push_str("\">\n");
        for rpm in artifacts {
            let meta = query_rpm(rpm)?;
            let fname = rpm.file_name().unwrap().to_string_lossy().to_string();
            let sha256 = lx_lib::checksum::sha256_file(rpm)?;
            let archive_size = std::fs::metadata(rpm)?.len();
            primary.push_str(&format!(
                "<package type=\"rpm\">\
                 <name>{}</name>\
                 <arch>{}</arch>\
                 <version epoch=\"0\" ver=\"{}\" rel=\"{}\"/>\
                 <checksum type=\"sha256\" pkgid=\"YES\">{sha256}</checksum>\
                 <summary>{}</summary>\
                 <description>{}</description>\
                 <size package=\"{archive_size}\" installed=\"{}\" archive=\"0\"/>\
                 <location href=\"{}\"/>\
                 </package>\n",
                xml_escape(&meta.name),
                xml_escape(&meta.arch),
                xml_escape(&meta.version),
                xml_escape(&meta.release),
                xml_escape(&meta.summary),
                xml_escape(&meta.summary),
                meta.size,
                xml_escape(&fname),
            ));
        }
        primary.push_str("</metadata>\n");

        let repodata = dir.join("repodata");
        std::fs::create_dir_all(&repodata)?;
        let gz = lx_lib::debarchive::deterministic_gzip_bytes(primary.as_bytes(), 0, 9)?;
        std::fs::write(repodata.join("primary.xml.gz"), &gz)?;
        let open_sha = sha256_bytes(primary.as_bytes());
        let gz_sha = sha256_bytes(&gz);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let repomd = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <repomd xmlns=\"http://linux.duke.edu/metadata/repo\" \
             xmlns:rpm=\"http://linux.duke.edu/metadata/rpm\">\
             <revision>{timestamp}</revision>\
             <data type=\"primary\">\
             <checksum type=\"sha256\">{gz_sha}</checksum>\
             <open-checksum type=\"sha256\">{open_sha}</open-checksum>\
             <location href=\"repodata/primary.xml.gz\"/>\
             <timestamp>{timestamp}</timestamp>\
             <size>{}</size>\
             <open-size>{}</open-size>\
             </data>\
             </repomd>\n",
            gz.len(),
            primary.len(),
        );
        std::fs::write(repodata.join("repomd.xml"), repomd)?;
        println!(
            "wrote repodata/repomd.xml, repodata/primary.xml.gz ({} packages)",
            artifacts.len()
        );
        Ok(())
    }

    fn sign_index(&self, dir: &Path, opts: &IndexOptions) -> Result<()> {
        let Some(key) = opts.sign_key else {
            return Ok(());
        };
        // dnf/yum fetch an armored detached signature at
        // `repodata/repomd.xml.asc`.
        let repomd = std::fs::read(dir.join("repodata/repomd.xml"))?;
        let req = lx_lib::sign::SignRequest {
            key_file: key,
            key_id: opts.sign_key_id,
            passphrase: None,
        };
        let asc = lx_lib::sign::clearsign(&repomd, &req)?;
        std::fs::write(dir.join("repodata/repomd.xml.asc"), asc)?;
        println!("wrote repodata/repomd.xml.asc (armored detached)");
        Ok(())
    }
}

struct RpmMeta {
    name: String,
    version: String,
    release: String,
    arch: String,
    summary: String,
    size: u64,
}

fn query_rpm(path: &Path) -> Result<RpmMeta> {
    let out = Command::new("rpm")
        .args([
            "-qp",
            "--queryformat",
            "%{NAME}|%{VERSION}|%{RELEASE}|%{ARCH}|%{SUMMARY}|%{SIZE}|%{LICENSE}",
        ])
        .arg(path)
        .output()
        .context("failed to run `rpm` (install rpm to index .rpm repositories)")?;
    if !out.status.success() {
        bail!(
            "`rpm -qp` failed for '{}': {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let fields: Vec<&str> = text.trim().split('|').collect();
    if fields.len() < 6 {
        bail!(
            "unexpected `rpm -qp` output for '{}': {text}",
            path.display()
        );
    }
    Ok(RpmMeta {
        name: fields[0].to_string(),
        version: fields[1].to_string(),
        release: fields[2].to_string(),
        arch: fields[3].to_string(),
        summary: fields[4].to_string(),
        size: fields[5].parse().unwrap_or(0),
    })
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}
