use std::collections::HashMap;
use std::path::Path;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::cargo_gen;

pub struct BuildResult {
    pub success: bool,
    pub wasm_bytes: Option<Vec<u8>>,
    pub build_log: String,
}

pub struct BuildRequest {
    pub component_type: String,
    pub name: String,
    pub files: HashMap<String, String>,
    pub dependencies: Vec<String>,
}

/// Spawn a command that is automatically killed on drop (i.e. on cancellation).
pub fn spawn_killable(cmd: &mut Command) -> std::io::Result<tokio::process::Child> {
    cmd.kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
}

/// Run a child process, killing it if the cancellation token fires.
pub async fn run_command(child: tokio::process::Child, cancel: &CancellationToken) -> std::io::Result<std::process::Output> {
    // kill_on_drop(true) ensures the child is killed when the future is dropped
    // by tokio::select! on cancellation.
    tokio::select! {
        result = child.wait_with_output() => result,
        () = cancel.cancelled() => {
            // child is dropped here, kill_on_drop sends SIGKILL
            Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "build cancelled"))
        }
    }
}

/// Execute a full build: generate Cargo.toml, write files, cargo build --target wasm32-wasip2, wasmtime compile.
///
/// Uses a shared `target_dir` so dependency artifacts are cached across builds.
/// Only the user's crate (a single `lib.rs`) gets recompiled each time.
/// Cargo's built-in file locking serializes concurrent builds sharing the same target.
///
/// The `cancel` token allows graceful shutdown — in-flight builds are killed.
pub async fn execute_build(deps_dir: &str, target_dir: &str, req: &BuildRequest, cancel: &CancellationToken) -> BuildResult {
    let work_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: format!("failed to create temp dir: {e}"),
            };
        }
    };

    let project_dir = work_dir.path();
    let mut log = String::new();

    // 1. Generate and write Cargo.toml
    let cargo_toml = match cargo_gen::generate_cargo_toml(
        deps_dir,
        &req.name,
        &req.component_type,
        &req.dependencies,
    ) {
        Ok(t) => t,
        Err(e) => {
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: format!("cargo.toml generation error: {e}"),
            };
        }
    };

    if let Err(e) = tokio::fs::write(project_dir.join("Cargo.toml"), &cargo_toml).await {
        return BuildResult {
            success: false,
            wasm_bytes: None,
            build_log: format!("failed to write Cargo.toml: {e}"),
        };
    }

    log.push_str(&format!("Generated Cargo.toml:\n{cargo_toml}\n---\n"));

    // 2. Write user source files
    for (path, content) in &req.files {
        let file_path = project_dir.join(path);
        if let Some(parent) = file_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return BuildResult {
                    success: false,
                    wasm_bytes: None,
                    build_log: format!("failed to create dir {}: {e}", parent.display()),
                };
            }
        }
        if let Err(e) = tokio::fs::write(&file_path, content).await {
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: format!("failed to write {path}: {e}"),
            };
        }
    }

    // 3. Run cargo build targeting wasm32-wasip2
    info!(name = %req.name, "starting cargo build --target wasm32-wasip2");
    let child = spawn_killable(
        Command::new("cargo")
            .arg("build")
            .arg("--target")
            .arg("wasm32-wasip2")
            .arg("--release")
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", target_dir),
    );

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: format!("{log}cargo build failed to spawn: {e}"),
            };
        }
    };

    let cargo_output = match run_command(child, cancel).await {
        Ok(o) => o,
        Err(e) => {
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: format!("{log}cargo build interrupted: {e}"),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&cargo_output.stdout);
    let stderr = String::from_utf8_lossy(&cargo_output.stderr);
    log.push_str(&format!("cargo build stdout:\n{stdout}\n"));
    log.push_str(&format!("cargo build stderr:\n{stderr}\n"));

    if !cargo_output.status.success() {
        return BuildResult {
            success: false,
            wasm_bytes: None,
            build_log: log,
        };
    }

    // 4. Find the .wasm output (in the shared target dir)
    let wasm_name = req.name.replace('-', "_");
    let target = std::path::Path::new(target_dir);
    let wasm_path = target
        .join("wasm32-wasip2")
        .join("release")
        .join(format!("{wasm_name}.wasm"));

    if !wasm_path.exists() {
        // Try debug path as fallback
        let debug_path = target
            .join("wasm32-wasip2")
            .join("debug")
            .join(format!("{wasm_name}.wasm"));

        if !debug_path.exists() {
            log.push_str(&format!(
                "WASM file not found at {} or {}\n",
                wasm_path.display(),
                debug_path.display()
            ));
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: log,
            };
        }
        // Use debug path
        return compile_wasm(&debug_path, &log, cancel).await;
    }

    compile_wasm(&wasm_path, &log, cancel).await
}

/// Run wasmtime compile on the .wasm file to produce .cwasm bytes.
async fn compile_wasm(wasm_path: &Path, existing_log: &str, cancel: &CancellationToken) -> BuildResult {
    let mut log = existing_log.to_string();

    info!(wasm = %wasm_path.display(), "running wasmtime compile");

    let cwasm_path = wasm_path.with_extension("cwasm");

    let child = spawn_killable(
        Command::new("wasmtime")
            .arg("compile")
            .arg("--output")
            .arg(&cwasm_path)
            .arg(wasm_path),
    );

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: format!("{log}wasmtime compile failed to spawn: {e}"),
            };
        }
    };

    let compile_output = match run_command(child, cancel).await {
        Ok(o) => o,
        Err(e) => {
            return BuildResult {
                success: false,
                wasm_bytes: None,
                build_log: format!("{log}wasmtime compile interrupted: {e}"),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&compile_output.stdout);
    let stderr = String::from_utf8_lossy(&compile_output.stderr);
    log.push_str(&format!("wasmtime compile stdout:\n{stdout}\n"));
    log.push_str(&format!("wasmtime compile stderr:\n{stderr}\n"));

    if !compile_output.status.success() {
        // Even if wasmtime compile fails, we can still return the .wasm bytes
        warn!(stderr = %stderr.trim(), "wasmtime compile failed, returning raw .wasm instead");
        match tokio::fs::read(wasm_path).await {
            Ok(bytes) => {
                log.push_str("wasmtime compile failed, returning raw .wasm\n");
                return BuildResult {
                    success: true,
                    wasm_bytes: Some(bytes),
                    build_log: log,
                };
            }
            Err(e) => {
                return BuildResult {
                    success: false,
                    wasm_bytes: None,
                    build_log: format!("{log}failed to read .wasm file: {e}"),
                };
            }
        }
    }

    // Read the compiled .cwasm
    match tokio::fs::read(&cwasm_path).await {
        Ok(bytes) => {
            log.push_str("build successful\n");
            BuildResult {
                success: true,
                wasm_bytes: Some(bytes),
                build_log: log,
            }
        }
        Err(e) => BuildResult {
            success: false,
            wasm_bytes: None,
            build_log: format!("{log}failed to read .cwasm file: {e}"),
        },
    }
}
