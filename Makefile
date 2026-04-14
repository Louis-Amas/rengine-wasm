# Detect host architecture for wasmtime compile --target
UNAME_S := $(shell uname -s)
UNAME_M := $(shell uname -m)

ifeq ($(UNAME_S),Linux)
  ifeq ($(UNAME_M),x86_64)
    WASMTIME_TARGET := x86_64-unknown-linux-gnu
  else ifeq ($(UNAME_M),aarch64)
    WASMTIME_TARGET := aarch64-unknown-linux-gnu
  endif
else ifeq ($(UNAME_S),Darwin)
  ifeq ($(UNAME_M),x86_64)
    WASMTIME_TARGET := x86_64-apple-darwin
  else ifeq ($(UNAME_M),arm64)
    WASMTIME_TARGET := aarch64-apple-darwin
  endif
endif

# Directories
STRATEGY_DIRS := $(wildcard strategies/*)
TRANSFORMER_DIRS := $(wildcard transformers/*)
MULTICALL_DIRS := $(wildcard evm_multicalls/*)
LOGS_DIRS := $(wildcard evm_logs/*)

TARGET_WASM_DIR := target/wasm32-wasip2/debug
WASM_OUT_DIR := strategies-wasm/
TRANSFORMER_WASM_OUT_DIR := transformers-wasm/
MULTICALL_WASM_OUT_DIR := evm_multicalls_wasm/
LOGS_WASM_OUT_DIR := evm_logs_wasm/

.PHONY: all build compile build-strategies build-transformers build-multicalls build-logs compile-strategies compile-transformers compile-multicalls clean test

# Default target: build and compile everything
all: build compile

test: compile
	cargo test

# =========================================================
# === Build phase (cargo build --target wasm32-wasip2)
# =========================================================

build: build-strategies build-transformers build-multicalls build-logs

build-strategies:
	@echo "Building all strategies..."
	@for dir in $(STRATEGY_DIRS); do \
		if [ -f $$dir/Cargo.toml ]; then \
			echo "  -> Building $$dir"; \
			cargo build --target wasm32-wasip2 --manifest-path $$dir/Cargo.toml; \
		fi; \
	done

build-transformers:
	@echo "Building all transformers..."
	@for dir in $(TRANSFORMER_DIRS); do \
		if [ -f $$dir/Cargo.toml ]; then \
			echo "  -> Building $$dir"; \
			cargo build --target wasm32-wasip2 --manifest-path $$dir/Cargo.toml; \
		fi; \
	done

build-multicalls:
	@echo "Building all multicalls..."
	@for dir in $(MULTICALL_DIRS); do \
		if [ -f $$dir/Cargo.toml ]; then \
			echo "  -> Building $$dir"; \
			cargo build --target wasm32-wasip2 --manifest-path $$dir/Cargo.toml; \
		fi; \
	done

build-logs:
	@echo "Building all logs..."
	@for dir in $(LOGS_DIRS); do \
		if [ -f $$dir/Cargo.toml ]; then \
			echo "  -> Building $$dir"; \
			cargo build --target wasm32-wasip2 --manifest-path $$dir/Cargo.toml; \
		fi; \
	done

# =========================================================
# === Compile phase (wasmtime compile)
# =========================================================

compile: compile-strategies compile-transformers compile-multicalls compile-logs

compile-strategies:
	@echo "Precompiling strategies with wasmtime..."
	@mkdir -p $(WASM_OUT_DIR)
	@for dir in $(STRATEGY_DIRS); do \
		name=$$(basename $$dir); \
		wasm_file="$(TARGET_WASM_DIR)/$$name.wasm"; \
		if [ -f $$wasm_file ]; then \
			echo "  -> Compiling $$wasm_file"; \
			wasmtime compile --target $(WASMTIME_TARGET) \
				--output $(WASM_OUT_DIR)/$$name.cwasm $$wasm_file; \
		else \
			echo "  !! Missing wasm for $$name"; \
		fi; \
	done

compile-transformers:
	@echo "Precompiling transformers with wasmtime..."
	@mkdir -p $(TRANSFORMER_WASM_OUT_DIR)
	@for dir in $(TRANSFORMER_DIRS); do \
		name=$$(basename $$dir); \
		wasm_file="$(TARGET_WASM_DIR)/$$name.wasm"; \
		if [ -f $$wasm_file ]; then \
			echo "  -> Compiling $$wasm_file"; \
			wasmtime compile --target $(WASMTIME_TARGET) \
				--output $(TRANSFORMER_WASM_OUT_DIR)/$$name.cwasm $$wasm_file; \
		else \
			echo "  !! Missing wasm for $$name"; \
		fi; \
	done

compile-multicalls:
	@echo "Precompiling multicalls with wasmtime..."
	@mkdir -p $(MULTICALL_WASM_OUT_DIR)
	@for dir in $(MULTICALL_DIRS); do \
		name=$$(basename $$dir); \
		wasm_file="$(TARGET_WASM_DIR)/$$name.wasm"; \
		if [ -f $$wasm_file ]; then \
			echo "  -> Compiling $$wasm_file"; \
			wasmtime compile --target $(WASMTIME_TARGET) \
				--output $(MULTICALL_WASM_OUT_DIR)/$$name.cwasm $$wasm_file; \
		else \
			echo "  !! Missing wasm for $$name"; \
		fi; \
	done

compile-logs:
	@echo "Precompiling logs with wasmtime..."
	@mkdir -p $(LOGS_WASM_OUT_DIR)
	@for dir in $(LOGS_DIRS); do \
		name=$$(basename $$dir); \
		wasm_file="$(TARGET_WASM_DIR)/$$name.wasm"; \
		if [ -f $$wasm_file ]; then \
			echo "  -> Compiling $$wasm_file"; \
			wasmtime compile --target $(WASMTIME_TARGET) \
				--output $(LOGS_WASM_OUT_DIR)/$$name.cwasm $$wasm_file; \
		else \
			echo "  !! Missing wasm for $$name"; \
		fi; \
	done

# =========================================================
# === Clean
# =========================================================

clean:
	@echo "Cleaning build artifacts..."
	@rm -rf $(WASM_OUT_DIR) $(TRANSFORMER_WASM_OUT_DIR) $(MULTICALL_WASM_OUT_DIR) $(LOGS_WASM_OUT_DIR)
	@cargo clean
