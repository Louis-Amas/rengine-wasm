# ============================================================
# Stage 1: Build the wasm-builder HTTP service
# ============================================================
FROM rust:1.91-slim AS builder

RUN apt-get update && apt-get install -y \
    build-essential pkg-config libssl-dev curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY services/wasm-builder/ .
RUN cargo build --release

# ============================================================
# Stage 2: Prepare deps — the API crates that user code imports
# ============================================================
FROM rust:1.91-slim AS deps-prep

RUN apt-get update && apt-get install -y \
    build-essential pkg-config libssl-dev curl \
    && rm -rf /var/lib/apt/lists/*

# Install cargo-component (arch-aware)
RUN ARCH=$(uname -m) && \
    curl -LsSf "https://github.com/bytecodealliance/cargo-component/releases/latest/download/cargo-component-${ARCH}-unknown-linux-gnu.tar.gz" \
    | tar xzf - -C /usr/local/bin/ 2>/dev/null || \
    cargo install cargo-component

# Install wasmtime CLI (pinned to match wasm-runtime crate, arch-aware)
ENV WASMTIME_VERSION=v39.0.1
RUN ARCH=$(uname -m) && \
    curl -LsSf "https://github.com/bytecodealliance/wasmtime/releases/download/${WASMTIME_VERSION}/wasmtime-${WASMTIME_VERSION}-${ARCH}-linux.tar.xz" \
    | tar xJf - --strip-components=1 -C /usr/local/bin/ "wasmtime-${WASMTIME_VERSION}-${ARCH}-linux/wasmtime"

# Copy only the crates needed as deps for user WASM components
WORKDIR /deps

# strategy-api and its transitive deps
COPY crates/strategy_api/ /deps/strategy-api/
COPY crates/types/ /deps/rengine-types/
COPY crates/macros/ /deps/rengine-macros/

# evm-multicall-api and its transitive deps
COPY crates/evm_multicall_api/ /deps/evm-multicall-api/
COPY crates/evm_types/ /deps/evm-types/

# evm-logs-api
COPY crates/evm_logs_api/ /deps/evm-logs-api/

# WIT files (needed by cargo-component)
COPY crates/strategy_api/strategy.wit /deps/strategy-api/strategy.wit
COPY crates/evm_multicall_api/multicall.wit /deps/evm-multicall-api/multicall.wit
COPY crates/evm_logs_api/evm_logs.wit /deps/evm-logs-api/evm_logs.wit

# Rewrite workspace dependencies to use local paths + explicit versions.
# The API crates use `workspace = true` which won't work outside the workspace.
# We generate standalone Cargo.toml files for each dep crate.

# strategy-api Cargo.toml
RUN cat > /deps/strategy-api/Cargo.toml <<'TOML'
[package]
name = "strategy-api"
version = "0.1.0"
edition = "2021"

[dependencies]
rengine-types = { path = "../rengine-types" }
rengine-macros = { path = "../rengine-macros" }
anyhow = "1"
borsh = { version = "1.5", features = ["derive"] }
parking_lot = "0.12"
rust_decimal = { version = "1.36", features = ["borsh", "serde-str", "maths", "c-repr"] }
rust_decimal_macros = "1.36"
wit-bindgen = "0.42"
TOML

# rengine-types Cargo.toml
RUN cat > /deps/rengine-types/Cargo.toml <<'TOML'
[package]
name = "rengine-types"
version = "0.0.1"
edition = "2021"

[dependencies]
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
borsh = { version = "1.5", features = ["derive"] }
rust_decimal = { version = "1.36", features = ["borsh", "serde-str", "maths", "c-repr"] }
rust_decimal_macros = "1.36"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
smol_str = { version = "0.3", features = ["borsh", "serde"] }
hex = "0.4"
TOML

# rengine-macros Cargo.toml
RUN cat > /deps/rengine-macros/Cargo.toml <<'TOML'
[package]
name = "rengine-macros"
version = "0.0.1"
edition = "2021"

[lib]
proc-macro = true

[dependencies]
quote = "1"
syn = { version = "2", features = ["full", "visit"] }
proc-macro2 = "1"
TOML

# evm-multicall-api Cargo.toml
RUN cat > /deps/evm-multicall-api/Cargo.toml <<'TOML'
[package]
name = "evm-multicall-api"
version = "0.1.0"
edition = "2021"

[dependencies]
rengine-types = { path = "../rengine-types" }
rengine-macros = { path = "../rengine-macros" }
evm-types = { path = "../evm-types", features = ["providers"] }
anyhow = "1"
alloy = { version = "1.0", features = ["eip712"], default-features = false }
borsh = { version = "1.5", features = ["derive"] }
parking_lot = "0.12"
rust_decimal = { version = "1.36", features = ["borsh", "serde-str", "maths", "c-repr"] }
rust_decimal_macros = "1.36"
wit-bindgen = "0.42"
TOML

# evm-types Cargo.toml
RUN cat > /deps/evm-types/Cargo.toml <<'TOML'
[package]
name = "evm-types"
version = "0.0.1"
edition = "2021"

[dependencies]
alloy = { version = "1.0", default-features = false, features = ["sol-types", "json"] }
borsh = { version = "1.5", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
rengine-types = { path = "../rengine-types" }
rust_decimal = { version = "1.36", features = ["borsh", "serde-str", "maths", "c-repr"] }

[features]
providers = ["alloy/providers", "alloy/contract"]
TOML

# evm-logs-api Cargo.toml
RUN cat > /deps/evm-logs-api/Cargo.toml <<'TOML'
[package]
name = "evm-logs-api"
version = "0.0.1"
edition = "2021"

[dependencies]
wit-bindgen = "0.42"
borsh = { version = "1.5", features = ["derive"] }
evm-types = { path = "../evm-types" }
rengine-types = { path = "../rengine-types" }
anyhow = "1"
serde_json = "1"
alloy = { version = "1.0", default-features = false, features = ["rpc-types"] }
TOML

# Pre-build deps to cache them (build a dummy project that imports all API crates)
RUN mkdir -p /tmp/warmup/src && \
    echo 'fn main() {}' > /tmp/warmup/src/main.rs && \
    cat > /tmp/warmup/Cargo.toml <<'TOML'
[package]
name = "warmup"
version = "0.1.0"
edition = "2021"

[dependencies]
strategy-api = { path = "/deps/strategy-api" }
rengine-types = { path = "/deps/rengine-types" }
TOML
# Warm up the cargo registry cache (best-effort, don't fail the build)
RUN cd /tmp/warmup && cargo fetch 2>/dev/null || true && rm -rf /tmp/warmup

# ============================================================
# Stage 3: Final runtime image
# ============================================================
FROM rust:1.91-slim

RUN apt-get update && apt-get install -y \
    build-essential pkg-config libssl-dev curl \
    && rm -rf /var/lib/apt/lists/*

# Install cargo-component (arch-aware)
RUN ARCH=$(uname -m) && \
    (curl -LsSf "https://github.com/bytecodealliance/cargo-component/releases/latest/download/cargo-component-${ARCH}-unknown-linux-gnu.tar.gz" \
    | tar xzf - -C /usr/local/bin/ 2>/dev/null || \
    cargo install cargo-component)

# Install wasmtime CLI (pinned to match wasm-runtime crate, arch-aware)
ENV WASMTIME_VERSION=v39.0.1
RUN ARCH=$(uname -m) && \
    curl -LsSf "https://github.com/bytecodealliance/wasmtime/releases/download/${WASMTIME_VERSION}/wasmtime-${WASMTIME_VERSION}-${ARCH}-linux.tar.xz" \
    | tar xJf - --strip-components=1 -C /usr/local/bin/ "wasmtime-${WASMTIME_VERSION}-${ARCH}-linux/wasmtime"

# Add wasm32-wasip1 target
RUN rustup target add wasm32-wasip1

# Copy API crates (deps for user builds)
COPY --from=deps-prep /deps /deps

# Copy the wasm-builder binary
COPY --from=builder /build/target/release/wasm-builder /usr/local/bin/wasm-builder

ENV WASM_BUILDER_DEPS_DIR=/deps
ENV WASM_BUILDER_TARGET_DIR=/cache/target
ENV WASM_BUILDER_ADDR=0.0.0.0:3001
ENV RUST_LOG=wasm_builder=info

EXPOSE 3001

ENTRYPOINT ["/usr/local/bin/wasm-builder"]
