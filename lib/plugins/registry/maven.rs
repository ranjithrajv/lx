// SPDX-License-Identifier: GPL-3.0-or-later

//! Maven Central (Java/Kotlin/Scala) registry source plugin.
//!
//! Fetches a Java artifact from Maven Central and produces a local
//! directory of files ready for packaging. Uses `mvn dependency:copy`
//! to download the jar/pom to a staging directory.
//!
//! Maven coordinates: groupId:artifactId:version (e.g. org.apache.commons:commons-lang3:3.12.0)

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

use crate::config::PackageConfig;
use crate::plugins::plugin::plugin_identity;
use crate::plugins::registry::{staging, RegistryPayload, RegistrySource};

pub struct MavenRegistrySource;

plugin_identity!(
    MavenRegistrySource,
    "maven",
    "Maven Central (Java/Kotlin/Scala) — mvn dependency:copy + extract"
);

impl RegistrySource for MavenRegistrySource {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["mvn"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        // package is expected as "groupId:artifactId" or "groupId:artifactId:version"
        let (group_id, artifact_id, resolved_version) = parse_maven_coordinates(package, version)?;

        let coord = format!("{}:{}:{}", group_id, artifact_id, resolved_version);
        println!("maven: fetching {coord}");

        let workdir = tempfile::tempdir().context("failed to create maven workdir")?;
        let download_dir = workdir.path().join("download");
        std::fs::create_dir_all(&download_dir)?;

        // Create a minimal pom.xml to use mvn dependency:copy.
        let pom = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
    <modelVersion>4.0.0</modelVersion>
    <groupId>lx.temp</groupId>
    <artifactId>lx-temp</artifactId>
    <version>1.0</version>
    <dependencies>
        <dependency>
            <groupId>{}</groupId>
            <artifactId>{}</artifactId>
            <version>{}</version>
        </dependency>
    </dependencies>
</project>
"#,
            group_id, artifact_id, resolved_version
        );

        let pom_path = workdir.path().join("pom.xml");
        std::fs::write(&pom_path, pom).context("failed to write temporary pom.xml")?;

        // mvn dependency:copy-dependencies downloads all deps to the target dir.
        let out_dir = download_dir.to_string_lossy().to_string();
        staging::run_tool(
            "mvn",
            &[
                "dependency:copy-dependencies",
                "-DoutputDirectory",
                &out_dir,
                "-DincludeScope",
                "runtime",
                "--batch-mode",
                "--quiet",
            ],
            Some(workdir.path()),
            "mvn dependency:copy-dependencies",
        )?;

        // Find the downloaded jar (and optionally sources jar).
        let jar_files: Vec<PathBuf> = std::fs::read_dir(&download_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e == "jar")
                    .unwrap_or(false)
            })
            .collect();

        if jar_files.is_empty() {
            bail!("mvn downloaded no jar files for {coord}");
        }

        // Stage the main jar (not sources/javadoc) into usr/share/java/<artifact>/.
        let files_dir = workdir.path().join("package");
        let java_dir = files_dir
            .join("usr")
            .join("share")
            .join("java")
            .join(artifact_id);
        std::fs::create_dir_all(&java_dir)?;

        for jar in &jar_files {
            let name = jar.file_name().unwrap_or_default().to_string_lossy();
            // Skip sources and javadoc jars for the main artifact.
            if !name.contains("-sources") && !name.contains("-javadoc") {
                let dest = java_dir.join(name.as_ref());
                std::fs::copy(jar, dest)?;
            }
        }

        println!("maven: staged {coord}");

        let description = cfg.description.clone();

        Ok(RegistryPayload {
            files_dir,
            resolved_version,
            description,
        })
    }
}

/// Parse Maven coordinates from the package string.
/// Accepts "groupId:artifactId" or "groupId:artifactId:version".
fn parse_maven_coordinates(package: &str, version: &str) -> Result<(String, String, String)> {
    let parts: Vec<&str> = package.split(':').collect();

    match parts.as_slice() {
        [group, artifact] => {
            let v = if version.trim().is_empty() {
                "latest.release"
            } else {
                version.trim()
            };
            Ok((group.to_string(), artifact.to_string(), v.to_string()))
        }
        [group, artifact, ver] => Ok((group.to_string(), artifact.to_string(), ver.to_string())),
        _ => bail!(
            "maven: expected 'groupId:artifactId' or 'groupId:artifactId:version', got '{package}'"
        ),
    }
}
