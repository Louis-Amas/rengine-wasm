use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::build::{BuildRequest, spawn_killable, run_command};
use crate::cargo_gen;

/// A single diagnostic suitable for mapping to a Monaco editor marker.
#[derive(Debug, Serialize, Clone)]
pub struct Diagnostic {
    /// "error" | "warning" | "info" | "hint"
    pub severity: String,
    pub message: String,
    /// File path relative to the project (e.g. "src/lib.rs")
    pub file: Option<String>,
    pub start_line: Option<u32>,
    pub start_column: Option<u32>,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
    /// The rustc error code, e.g. "E0308"
    pub code: Option<String>,
}

#[derive(Serialize)]
pub struct CheckResult {
    pub success: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Raw stderr for debugging
    pub raw_stderr: String,
}

/// Partial shape of `cargo check --message-format=json` output lines.
#[derive(Deserialize)]
struct CargoMessage {
    reason: Option<String>,
    message: Option<CompilerMessage>,
}

#[derive(Deserialize)]
struct CompilerMessage {
    level: Option<String>,
    message: Option<String>,
    code: Option<DiagnosticCode>,
    spans: Option<Vec<DiagnosticSpan>>,
}

#[derive(Deserialize)]
struct DiagnosticCode {
    code: Option<String>,
}

#[derive(Deserialize)]
struct DiagnosticSpan {
    file_name: Option<String>,
    line_start: Option<u32>,
    line_end: Option<u32>,
    column_start: Option<u32>,
    column_end: Option<u32>,
    is_primary: Option<bool>,
}

/// Map rustc level strings to Monaco-friendly severity.
fn map_severity(level: &str) -> &'static str {
    match level {
        "error" | "error: internal compiler error" => "error",
        "warning" => "warning",
        "note" => "info",
        "help" => "hint",
        _ => "info",
    }
}

/// Run `cargo component check --message-format=json` and parse diagnostics.
pub async fn execute_check(
    deps_dir: &str,
    target_dir: &str,
    req: &BuildRequest,
    cancel: &CancellationToken,
) -> CheckResult {
    let work_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                success: false,
                diagnostics: vec![],
                raw_stderr: format!("failed to create temp dir: {e}"),
            };
        }
    };

    let project_dir = work_dir.path();

    // 1. Generate Cargo.toml
    let cargo_toml = match cargo_gen::generate_cargo_toml(
        deps_dir,
        &req.name,
        &req.component_type,
        &req.dependencies,
    ) {
        Ok(t) => t,
        Err(e) => {
            return CheckResult {
                success: false,
                diagnostics: vec![],
                raw_stderr: format!("cargo.toml generation error: {e}"),
            };
        }
    };

    if let Err(e) = tokio::fs::write(project_dir.join("Cargo.toml"), &cargo_toml).await {
        return CheckResult {
            success: false,
            diagnostics: vec![],
            raw_stderr: format!("failed to write Cargo.toml: {e}"),
        };
    }

    // 2. Write user source files
    for (path, content) in &req.files {
        let file_path = project_dir.join(path);
        if let Some(parent) = file_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return CheckResult {
                    success: false,
                    diagnostics: vec![],
                    raw_stderr: format!("failed to create dir {}: {e}", parent.display()),
                };
            }
        }
        if let Err(e) = tokio::fs::write(&file_path, content).await {
            return CheckResult {
                success: false,
                diagnostics: vec![],
                raw_stderr: format!("failed to write {path}: {e}"),
            };
        }
    }

    // 3. Run cargo check with JSON output targeting wasm32-wasip2
    info!(name = %req.name, "starting cargo check --target wasm32-wasip2");
    let child = spawn_killable(
        Command::new("cargo")
            .arg("check")
            .arg("--target")
            .arg("wasm32-wasip2")
            .arg("--message-format=json")
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", target_dir),
    );

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return CheckResult {
                success: false,
                diagnostics: vec![],
                raw_stderr: format!("cargo check failed to spawn: {e}"),
            };
        }
    };

    let output = match run_command(child, cancel).await {
        Ok(o) => o,
        Err(e) => {
            return CheckResult {
                success: false,
                diagnostics: vec![],
                raw_stderr: format!("cargo check interrupted: {e}"),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // 4. Parse JSON lines from stdout
    let user_files: HashMap<&str, ()> = req.files.keys().map(|k| (k.as_str(), ())).collect();
    let mut diagnostics = Vec::new();

    for line in stdout.lines() {
        let msg: CargoMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if msg.reason.as_deref() != Some("compiler-message") {
            continue;
        }

        let compiler_msg = match msg.message {
            Some(m) => m,
            None => continue,
        };

        let level = compiler_msg.level.as_deref().unwrap_or("error");
        let message = compiler_msg.message.unwrap_or_default();
        let code = compiler_msg.code.and_then(|c| c.code);

        let spans = compiler_msg.spans.unwrap_or_default();
        // Find the primary span, or use the first span
        let primary_span = spans
            .iter()
            .find(|s| s.is_primary == Some(true))
            .or(spans.first());

        let (file, start_line, start_col, end_line, end_col) = match primary_span {
            Some(span) => {
                let file = span.file_name.as_deref().unwrap_or("");
                // Only include diagnostics for user files
                if !user_files.contains_key(file) && !file.is_empty() {
                    // Still include errors without a file, or with "src/lib.rs" etc.
                    if !file.starts_with("src/") {
                        continue;
                    }
                }
                (
                    Some(file.to_string()),
                    span.line_start,
                    span.column_start,
                    span.line_end,
                    span.column_end,
                )
            }
            None => (None, None, None, None, None),
        };

        diagnostics.push(Diagnostic {
            severity: map_severity(level).to_string(),
            message,
            file,
            start_line,
            start_column: start_col,
            end_line,
            end_column: end_col,
            code,
        });
    }

    CheckResult {
        success: output.status.success(),
        diagnostics,
        raw_stderr: stderr.into_owned(),
    }
}
