// SPDX-License-Identifier: GPL-3.0-or-later

//! ERB-like template engine for maintainer scripts.
//!
//! When `template_scripts: true` is set in package.yaml, script files
//! (preinstall, postinstall, etc.) are processed through this engine before
//! being staged into the package. Template expressions use `<%= key %>`
//! syntax and are replaced with actual package values at build time.
//!
//! Available template variables:
//! - `<%= name %>` — package name
//! - `<%= version %>` — package version
//! - `<%= maintainer %>` — maintainer string
//! - `<%= description %>` — package description
//! - `<%= homepage %>` — homepage URL
//! - `<%= license %>` — SPDX license
//! - `<%= arch %>` — target architecture
//! - `<%= dist %>` — target distribution
//! - `<%= iteration %>` — build version/revision
//! - `<%= epoch %>` — epoch
//! - `<%= vendor %>` — vendor
//! - `<%= packager %>` — packager
//! - `<%= prefix %>` — install prefix

use std::collections::HashMap;

/// Render a template string, replacing `<%= key %>` expressions with values
/// from the given context. Unknown keys are left as-is (with a warning
/// printed to stderr).
pub fn render_template(template: &str, context: &HashMap<String, String>) -> String {
    let re = regex::Regex::new(r"<%\s*=\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*%>").unwrap();
    let mut result = String::with_capacity(template.len());
    let mut last_end = 0;
    let mut warnings = Vec::new();

    for cap in re.captures_iter(template) {
        let mat = cap.get(0).unwrap();
        let key = cap.get(1).unwrap().as_str();
        result.push_str(&template[last_end..mat.start()]);
        match context.get(key) {
            Some(value) => result.push_str(value),
            None => {
                warnings.push(key.to_string());
                result.push_str(mat.as_str());
            }
        }
        last_end = mat.end();
    }
    result.push_str(&template[last_end..]);

    if !warnings.is_empty() {
        eprintln!(
            "warning: unknown template variable(s) in script: {}",
            warnings.join(", ")
        );
    }

    result
}

/// Build the default template context from a PackageConfig and resolved job.
pub fn build_context(
    cfg: &crate::config::PackageConfig,
    job: &crate::build::ResolvedJob,
) -> HashMap<String, String> {
    let mut ctx = HashMap::new();
    ctx.insert("name".to_string(), cfg.package_name.clone());
    ctx.insert("version".to_string(), cfg.version.clone());
    ctx.insert("maintainer".to_string(), cfg.effective_maintainer());
    ctx.insert("description".to_string(), cfg.effective_description());
    ctx.insert("homepage".to_string(), homepage_for(cfg));
    ctx.insert("license".to_string(), cfg.license_spdx.clone());
    ctx.insert("arch".to_string(), job.arch.clone());
    ctx.insert("dist".to_string(), job.dist.clone());
    ctx.insert("iteration".to_string(), cfg.build_version.clone());
    ctx.insert("epoch".to_string(), cfg.epoch.clone());
    ctx.insert("vendor".to_string(), vendor_for(cfg));
    ctx.insert("packager".to_string(), cfg.effective_packager());
    ctx.insert("prefix".to_string(), cfg.prefix.clone());
    ctx
}

/// Resolve the homepage URL for the package based on its source provider.
fn homepage_for(cfg: &crate::config::PackageConfig) -> String {
    if cfg.effective_source() == "gitlab" {
        lx_lib::constants::homepage_for_gitlab(
            &cfg.github_repo,
            cfg.gitlab_host.as_deref().unwrap_or(""),
        )
    } else {
        lx_lib::constants::homepage_for_github(&cfg.github_repo)
    }
}

/// Resolve the vendor string from the config's extra fields.
fn vendor_for(cfg: &crate::config::PackageConfig) -> String {
    cfg.fields.get("Vendor").cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_simple() {
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), "myapp".to_string());
        ctx.insert("version".to_string(), "1.0.0".to_string());

        let template = "Package: <%= name %>\nVersion: <%= version %>";
        let result = render_template(template, &ctx);
        assert_eq!(result, "Package: myapp\nVersion: 1.0.0");
    }

    #[test]
    fn test_render_unknown_key_left_as_is() {
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), "myapp".to_string());

        let template = "Package: <%= name %>\nUnknown: <%= unknown_key %>";
        let result = render_template(template, &ctx);
        assert_eq!(result, "Package: myapp\nUnknown: <%= unknown_key %>");
    }

    #[test]
    fn test_render_no_expressions() {
        let ctx = HashMap::new();
        let template = "No templates here";
        let result = render_template(template, &ctx);
        assert_eq!(result, "No templates here");
    }

    #[test]
    fn test_render_multiple_same_key() {
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), "myapp".to_string());

        let template = "<%= name %> and <%= name %> again";
        let result = render_template(template, &ctx);
        assert_eq!(result, "myapp and myapp again");
    }

    #[test]
    fn test_render_whitespace_variants() {
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), "myapp".to_string());

        // Test various whitespace patterns
        assert_eq!(render_template("<%=name%>", &ctx), "myapp");
        assert_eq!(render_template("<%= name %>", &ctx), "myapp");
        assert_eq!(render_template("<%=  name  %>", &ctx), "myapp");
        assert_eq!(render_template("<% = name %>", &ctx), "myapp");
    }
}
