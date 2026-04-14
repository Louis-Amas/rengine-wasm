/// Whitelisted dependencies that user code is allowed to import.
const ALLOWED_DEPS: &[&str] = &[
    "borsh",
    "rust_decimal",
    "rust_decimal_macros",
    "serde",
    "serde_json",
    "anyhow",
    "smol_str",
    "alloy",
];

/// Component types and their corresponding API crate.
#[derive(Debug, Clone, Copy)]
pub struct ComponentMeta {
    pub api_crate: &'static str,
    /// Extra required deps beyond the API crate (e.g. evm-types for multicall/evm_logs).
    pub extra_deps: &'static [&'static str],
}

pub fn component_meta(component_type: &str) -> Option<ComponentMeta> {
    match component_type {
        "transformer" | "strategy" => Some(ComponentMeta {
            api_crate: "strategy-api",
            extra_deps: &[],
        }),
        "multicall" => Some(ComponentMeta {
            api_crate: "evm-multicall-api",
            extra_deps: &["evm-types"],
        }),
        "evm_logs" => Some(ComponentMeta {
            api_crate: "evm-logs-api",
            extra_deps: &["evm-types"],
        }),
        _ => None,
    }
}

/// Versions for whitelisted deps — pinned to match the workspace.
fn dep_version(name: &str) -> &'static str {
    match name {
        "borsh" => "1.5",
        "rust_decimal" => "1.36",
        "rust_decimal_macros" => "1.36",
        "serde" => "1",
        "serde_json" => "1",
        "anyhow" => "1",
        "smol_str" => "0.3",
        "alloy" => "1.0",
        _ => "0",
    }
}

/// Features to enable for specific deps.
fn dep_features(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "borsh" => Some(&["derive"]),
        "rust_decimal" => Some(&["borsh", "serde-str", "maths", "c-repr"]),
        "serde" => Some(&["derive"]),
        "smol_str" => Some(&["borsh", "serde"]),
        "alloy" => Some(&["sol-types"]),
        _ => None,
    }
}

/// Generate a Cargo.toml string for a user component.
///
/// `deps_dir` is the path where API crates are available (e.g. `/deps`).
/// `name` is the component name (e.g. `my_ema`).
/// `requested_deps` are optional deps from the whitelist the user wants.
pub fn generate_cargo_toml(
    deps_dir: &str,
    name: &str,
    component_type: &str,
    requested_deps: &[String],
) -> Result<String, String> {
    let meta = component_meta(component_type)
        .ok_or_else(|| format!("unknown component type: {component_type}"))?;

    // Validate all requested deps are whitelisted
    for dep in requested_deps {
        if !ALLOWED_DEPS.contains(&dep.as_str()) {
            return Err(format!("dependency '{dep}' is not in the whitelist. Allowed: {ALLOWED_DEPS:?}"));
        }
    }

    let mut toml_parts = Vec::new();

    // [package]
    toml_parts.push(format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]
"#
    ));

    // [dependencies]
    let mut deps_section = String::from("[dependencies]\n");

    // API crate as path dep
    deps_section.push_str(&format!(
        "{} = {{ path = \"{}/{}\" }}\n",
        meta.api_crate, deps_dir, meta.api_crate
    ));

    // rengine-types always included
    deps_section.push_str(&format!(
        "rengine-types = {{ path = \"{}/rengine-types\" }}\n",
        deps_dir
    ));

    // Extra deps (e.g. evm-types)
    for extra in meta.extra_deps {
        deps_section.push_str(&format!(
            "{extra} = {{ path = \"{deps_dir}/{extra}\" }}\n",
        ));
    }

    // Whitelisted user-requested deps
    for dep in requested_deps {
        let version = dep_version(dep);
        if let Some(features) = dep_features(dep) {
            let features_str: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
            deps_section.push_str(&format!(
                "{dep} = {{ version = \"{version}\", features = [{}] }}\n",
                features_str.join(", ")
            ));
        } else {
            deps_section.push_str(&format!("{dep} = \"{version}\"\n"));
        }
    }

    // Always include borsh (needed for serialization)
    if !requested_deps.iter().any(|d| d == "borsh") {
        if let Some(features) = dep_features("borsh") {
            let features_str: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
            deps_section.push_str(&format!(
                "borsh = {{ version = \"{}\", features = [{}] }}\n",
                dep_version("borsh"),
                features_str.join(", ")
            ));
        }
    }

    // Always include anyhow
    if !requested_deps.iter().any(|d| d == "anyhow") {
        deps_section.push_str(&format!("anyhow = \"{}\"\n", dep_version("anyhow")));
    }

    // Always include rust_decimal for strategy/transformer
    if matches!(component_type, "transformer" | "strategy")
        && !requested_deps.iter().any(|d| d == "rust_decimal")
    {
        if let Some(features) = dep_features("rust_decimal") {
            let features_str: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
            deps_section.push_str(&format!(
                "rust_decimal = {{ version = \"{}\", features = [{}] }}\n",
                dep_version("rust_decimal"),
                features_str.join(", ")
            ));
        }
    }
    if matches!(component_type, "transformer" | "strategy")
        && !requested_deps.iter().any(|d| d == "rust_decimal_macros")
    {
        deps_section.push_str(&format!(
            "rust_decimal_macros = \"{}\"\n",
            dep_version("rust_decimal_macros")
        ));
    }

    toml_parts.push(deps_section);

    Ok(toml_parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_transformer_cargo_toml() {
        let toml = generate_cargo_toml("/deps", "my_ema", "transformer", &[]).unwrap();
        assert!(toml.contains("strategy-api"));
        assert!(toml.contains("rengine-types"));
        assert!(!toml.contains("[package.metadata.component]"));
        assert!(toml.contains("borsh"));
    }

    #[test]
    fn test_generate_multicall_cargo_toml() {
        let toml = generate_cargo_toml("/deps", "my_multicall", "multicall", &["alloy".into()])
            .unwrap();
        assert!(toml.contains("evm-multicall-api"));
        assert!(toml.contains("evm-types"));
        assert!(!toml.contains("[package.metadata.component]"));
        assert!(toml.contains("alloy"));
    }

    #[test]
    fn test_reject_unknown_dep() {
        let result = generate_cargo_toml("/deps", "test", "transformer", &["reqwest".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_unknown_component_type() {
        let result = generate_cargo_toml("/deps", "test", "unknown", &[]);
        assert!(result.is_err());
    }
}
