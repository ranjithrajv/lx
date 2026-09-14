// SPDX-License-Identifier: GPL-3.0-or-later

//! Build an MSIX (Windows) package, mirroring nfpm's `msix` packager.
//!
//! An MSIX is an OPC (zip) container holding `AppxManifest.xml`,
//! `[Content_Types].xml`, `AppxBlockMap.xml`, and the payload. This writer
//! produces the container and metadata **unsigned**; `.pfx` signing is not
//! implemented yet (requesting it in `msix.signature.pfx_file` is reported
//! as unsupported rather than silently ignored).
//!
//! The payload is the staged install tree, placed at its destination paths
//! (leading `/` stripped), matching nfpm's path normalization.

use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::path::Path;

use base64::Engine as _;
use sha2::{Digest, Sha256};

/// One `<Application>` entry in the manifest.
#[derive(Debug, Clone, Default)]
pub struct MsixApplication {
    pub id: String,
    pub executable: String,
    pub entry_point: String,
    pub display_name: String,
    pub description: String,
    pub background_color: String,
    pub square150x150_logo: String,
    pub square44x44_logo: String,
}

/// One `<TargetDeviceFamily>`.
#[derive(Debug, Clone)]
pub struct TargetDeviceFamily {
    pub name: String,
    pub min_version: String,
    pub max_version_tested: String,
}

/// Everything the manifest needs, already resolved from `package.yaml`.
#[derive(Debug, Clone, Default)]
pub struct MsixManifest {
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub arch: String,
    pub resource_id: String,
    pub display_name: String,
    pub publisher_display_name: String,
    pub description: String,
    pub logo: String,
    pub applications: Vec<MsixApplication>,
    pub device_families: Vec<TargetDeviceFamily>,
    pub capabilities: Vec<String>,
    pub device_capabilities: Vec<String>,
    pub restricted: Vec<String>,
}

/// Build the `.msix` at `out` from the staged `root`. When `signer` is
/// given, the archive is signed natively: a `AppxSignature.p7x` member is
/// appended (PKCS#7 over the package digests) per Microsoft's Appx scheme,
/// using the `msix` crate for the ASN.1/PKCS#7 layer.
pub fn build(
    root: &Path,
    manifest: &MsixManifest,
    out: &Path,
    signer: Option<&xcommon::Signer>,
) -> Result<()> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let content_types = content_types_xml();
    let manifest_xml = render_manifest(manifest);
    let block_map = render_block_map(&files);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in [
        ("[Content_Types].xml", content_types.as_bytes()),
        ("AppxManifest.xml", manifest_xml.as_bytes()),
        ("AppxBlockMap.xml", block_map.as_bytes()),
    ] {
        zip.start_file(name, options)?;
        zip.write_all(body)?;
    }
    for (name, body) in &files {
        zip.start_file(name.as_str(), options)?;
        zip.write_all(body)?;
    }
    let mut bytes = zip.finish()?.into_inner();

    // Native signing: the p7x digest is over the package *before* the
    // signature member is appended (mirroring `signtool`/the msix crate).
    if let Some(signer) = signer {
        let digests = compute_digests(&bytes, &content_types, &block_map)?;
        let signature = msix::p7x::p7x(signer, &digests);
        bytes = append_member(bytes, "AppxSignature.p7x", &signature)?;
    }

    std::fs::write(out, bytes).with_context(|| format!("failed to write '{}'", out.display()))?;
    Ok(())
}

/// The five digests an Appx signature covers. `axci` (content integrity) is
/// optional and emitted as zeroes.
fn compute_digests(
    zip_bytes: &[u8],
    content_types: &str,
    block_map: &str,
) -> Result<msix::p7x::Digests> {
    let cd_start = central_directory_offset(zip_bytes)?;
    Ok(msix::p7x::Digests {
        axpc: Sha256::digest(&zip_bytes[..cd_start]).into(),
        axcd: Sha256::digest(&zip_bytes[cd_start..]).into(),
        axct: Sha256::digest(content_types.as_bytes()).into(),
        axbm: Sha256::digest(block_map.as_bytes()).into(),
        axci: [0u8; 32],
    })
}

/// Offset of the zip central directory, read from the EOCD record.
fn central_directory_offset(zip_bytes: &[u8]) -> Result<usize> {
    let sig = b"PK\x05\x06";
    let eocd = zip_bytes
        .windows(4)
        .rposition(|w| w == sig)
        .ok_or_else(|| anyhow!("zip end-of-central-directory record not found"))?;
    if eocd + 20 > zip_bytes.len() {
        anyhow::bail!("truncated zip end-of-central-directory record");
    }
    Ok(u32::from_le_bytes(
        zip_bytes[eocd + 16..eocd + 20]
            .try_into()
            .expect("4-byte slice"),
    ) as usize)
}

/// Append one member to an existing in-memory zip (rewrites the central
/// directory). Used for the signature, which must not be part of the digests
/// it covers.
fn append_member(bytes: Vec<u8>, name: &str, data: &[u8]) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new_append(std::io::Cursor::new(bytes))?;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(name, options)?;
    zip.write_all(data)?;
    Ok(zip.finish()?.into_inner())
}

/// Collect regular files under `dir` as (package path, bytes), skipping the
/// MSIX metadata members a caller might have staged into the tree.
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_files(root, &path, out)?;
        } else if ty.is_file() {
            let rel = path
                .strip_prefix(root)
                .expect("walked path must be under root");
            let name = rel.to_string_lossy().trim_start_matches('/').to_string();
            if is_metadata_member(&name) {
                continue;
            }
            out.push((name, std::fs::read(&path)?));
        }
    }
    Ok(())
}

fn is_metadata_member(name: &str) -> bool {
    matches!(
        name,
        "[Content_Types].xml" | "AppxManifest.xml" | "AppxBlockMap.xml" | "AppxSignature.p7x"
    ) || name.starts_with("AppxMetadata/")
}

fn content_types_xml() -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n",
    );
    for (ext, ct) in [
        ("xml", "application/xml"),
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("exe", "application/octet-stream"),
        ("dll", "application/octet-stream"),
        ("txt", "text/plain"),
        ("json", "application/json"),
        ("html", "text/html"),
        ("css", "text/css"),
        ("js", "application/javascript"),
    ] {
        s.push_str(&format!(
            "  <Default Extension=\"{ext}\" ContentType=\"{ct}\"/>\n"
        ));
    }
    s.push_str(
        "  <Default Extension=\"*\" ContentType=\"application/octet-stream\"/>\n\
         </Types>\n",
    );
    s
}

fn render_manifest(m: &MsixManifest) -> String {
    let display_name = if m.display_name.is_empty() {
        &m.name
    } else {
        &m.display_name
    };
    let publisher_display_name = if m.publisher_display_name.is_empty() {
        &m.name
    } else {
        &m.publisher_display_name
    };
    let mut apps = String::new();
    for app in &m.applications {
        let entry_point = if app.entry_point.is_empty() {
            "Windows.FullTrustApplication"
        } else {
            &app.entry_point
        };
        let app_display = if app.display_name.is_empty() {
            display_name
        } else {
            &app.display_name
        };
        let app_desc = if app.description.is_empty() {
            &m.description
        } else {
            &app.description
        };
        let bg = if app.background_color.is_empty() {
            "transparent"
        } else {
            &app.background_color
        };
        apps.push_str(&format!(
            "    <Application Id=\"{}\" Executable=\"{}\" EntryPoint=\"{}\">\n\
             \x20     <uap:VisualElements DisplayName=\"{}\" Description=\"{}\" BackgroundColor=\"{}\" Square150x150Logo=\"{}\" Square44x44Logo=\"{}\"/>\n\
             \x20   </Application>\n",
            esc(&app.id),
            esc(&app.executable),
            esc(entry_point),
            esc(app_display),
            esc(app_desc),
            esc(bg),
            esc(&app.square150x150_logo),
            esc(&app.square44x44_logo),
        ));
    }

    let mut deps = String::new();
    for d in &m.device_families {
        deps.push_str(&format!(
            "    <TargetDeviceFamily Name=\"{}\" MinVersion=\"{}\" MaxVersionTested=\"{}\"/>\n",
            esc(&d.name),
            esc(&d.min_version),
            esc(&d.max_version_tested),
        ));
    }

    let mut caps = String::new();
    for c in &m.capabilities {
        caps.push_str(&format!("    <Capability Name=\"{}\"/>\n", esc(c)));
    }
    for c in &m.device_capabilities {
        caps.push_str(&format!("    <DeviceCapability Name=\"{}\"/>\n", esc(c)));
    }
    for c in &m.restricted {
        caps.push_str(&format!("    <rescap:Capability Name=\"{}\"/>\n", esc(c)));
    }

    let resource_id = if m.resource_id.is_empty() {
        String::new()
    } else {
        format!(" ResourceId=\"{}\"", esc(&m.resource_id))
    };

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n\
         <Package xmlns=\"http://schemas.microsoft.com/appx/manifest/foundation/windows10\" \
         xmlns:uap=\"http://schemas.microsoft.com/appx/manifest/uap/windows10\" \
         xmlns:rescap=\"http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities\">\n\
         \x20 <Identity Name=\"{name}\" Publisher=\"{publisher}\" Version=\"{version}\" ProcessorArchitecture=\"{arch}\"{resource_id}/>\n\
         \x20 <Properties>\n\
         \x20   <DisplayName>{display}</DisplayName>\n\
         \x20   <PublisherDisplayName>{pub_display}</PublisherDisplayName>\n\
         \x20   <Logo>{logo}</Logo>\n\
         \x20   <Description>{desc}</Description>\n\
         \x20 </Properties>\n\
         \x20 <Dependencies>\n{deps}\x20 </Dependencies>\n\
         \x20 <Resources>\n    <Resource Language=\"en-us\"/>\n  </Resources>\n\
         \x20 <Applications>\n{apps}\x20 </Applications>\n\
         \x20 <Capabilities>\n{caps}\x20 </Capabilities>\n\
         </Package>\n",
        name = esc(&m.name),
        publisher = esc(&m.publisher),
        version = esc(&m.version),
        arch = esc(&m.arch),
        resource_id = resource_id,
        display = esc(display_name),
        pub_display = esc(publisher_display_name),
        logo = esc(&m.logo),
        desc = esc(&m.description),
    )
}

fn render_block_map(files: &[(String, Vec<u8>)]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <BlockMap xmlns=\"http://schemas.microsoft.com/appx/2010/blockmap\" \
         HashMethod=\"http://www.w3.org/2001/04/xmlenc#sha256\">\n",
    );
    for (name, body) in files {
        // Native block maps separate path components with `\` and hash
        // uncompressed 64 KiB blocks (no `Size`: the reference implementation
        // omits it, and Windows accepts that).
        let win_name = name.replace('/', "\\");
        let lfh = 30 + win_name.len();
        s.push_str(&format!(
            "  <File Name=\"{}\" Size=\"{}\" LfhSize=\"{}\">\n",
            esc(&win_name),
            body.len(),
            lfh,
        ));
        for chunk in body.chunks(65_536) {
            let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(chunk));
            s.push_str(&format!("    <Block Hash=\"{hash}\"/>\n"));
        }
        s.push_str("  </File>\n");
    }
    s.push_str("</BlockMap>\n");
    s
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
