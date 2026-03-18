# Rengine

A high-performance algorithmic trading engine written in Rust, built around a WebAssembly plugin architecture for safe, sandboxed strategy execution.

## What It Does

Rengine runs trading strategies as isolated WASM components. You write your strategy in Rust, compile it to WASM, and the engine handles everything else: market data ingestion, state management, order execution, and blockchain interactions.

The engine connects to centralized exchanges (Hyperliquid, Binance Spot & Perps) and EVM-compatible blockchains, streaming real-time data into a shared state that your strategies react to. When a strategy decides to trade, the engine routes execution requests to the appropriate venue.

## Architecture

```
Market Data (CEX / EVM)
        |
        v
   +---------+
   |  State   |  <-- balances, orderbooks, positions, indicators
   +---------+
        |
        v
+-------------------+
| WASM Components   |  <-- sandboxed, fuel-metered
| - Strategies      |  trading logic & decisions
| - Transformers    |  data enrichment & feature computation
| - Multicalls      |  batch EVM reads (Multicall3)
| - EVM Logs        |  on-chain event processing
+-------------------+
        |
        v
  Execution Layer
  (orders, EVM txs, indicator updates)
```

### WASM Plugin System

Four component types, each with its own WIT interface:

- **Strategies** - Core trading logic. Subscribe to state changes, timers, or UTC intervals. Emit execution requests (orders, EVM transactions, indicator updates).
- **Transformers** - Data processors that enrich the shared state with computed features (e.g., EMAs, market microstructure signals).
- **Multicalls** - Batch EVM contract reads via Multicall3. Efficiently query on-chain state (balances, pool reserves, prices).
- **EVM Logs** - Process blockchain event logs in real-time.

Components are compiled with `cargo component build` and ahead-of-time compiled with `wasmtime compile` for instant startup with no JIT overhead.

### Writing a Strategy

Implement the `Plugin` or `UnsafePlugin` trait:

```rust
use strategy_api::*;

struct MyStrategy;

impl Plugin for MyStrategy {
    type State = MyState;

    fn init() -> StrategyConfiguration {
        // Define triggers: state changes, timers, cooldowns
    }

    fn execute(state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        // Read market data, compute signals, emit orders
    }
}

impl_guest_from_plugin!(MyStrategy);
```

The `UnsafePlugin` variant uses zero-copy `Pod` types for latency-sensitive strategies.

### Execution Flow

1. Exchange readers and EVM readers stream market data
2. State updates trigger subscribed strategies/transformers
3. WASM components execute in sandboxed runtimes with fuel metering
4. Execution requests route to exchange executors or EVM executors
5. Results feed back into state, triggering the next cycle

## Supported Venues

| Venue | Type | Read | Execute |
|-------|------|------|---------|
| Hyperliquid | CEX (Spot & Perp) | Yes | Yes |
| Binance Spot | CEX | Yes | Yes |
| Binance Perpetuals | CEX | Yes | Yes |
| EVM Chains | Blockchain | Yes (Multicall3 + Logs) | Yes (Tx submission) |

## Project Structure

```
crates/
  core/           Engine orchestration & state management
  wasm-runtime/   Wasmtime sandbox with fuel metering
  strategy_api/   Plugin API for strategy authors
  evm_multicall_api/  Plugin API for multicall authors
  evm_logs_api/   Plugin API for log processor authors
  evm/            EVM reader/executor integration
  api/            HTTP API for runtime management
  db/             Persistence (DuckDB / ClickHouse)
  hyperliquid/    Hyperliquid exchange connector
  binance-*/      Binance exchange connectors
  types/          Core data types (Decimal, Account, Venue, etc.)
  config/         Configuration parsing
  user/           Multi-tenant user management
  gateway/        API gateway with auth
  git_store/      Git-based strategy versioning

strategies/       Example strategy components
transformers/     Example transformer components
evm_multicalls/   Example multicall components
evm_logs/         Example log processor components
bin/trader/       Main binary
services/
  wasm-builder/   HTTP service for compiling strategies to WASM
```

## Building

```bash
# Build the engine
cargo build

# Build a WASM strategy component
cd strategies/my_strategy
cargo component build

# AOT compile for your platform
wasmtime compile target/wasm32-wasip1/debug/my_strategy.wasm
```

## Multi-Tenant Platform

Rengine includes components for running as a multi-user platform:

- **WASM Build Service** - HTTP service that compiles user-submitted Rust code to WASM with a dependency whitelist
- **Git Store** - Per-user versioned code repositories with rollback support
- **User & Auth** - JWT + API key authentication with subscription tiers and quota tracking
- **Gateway** - Authenticated API that orchestrates builds, storage, and quota enforcement

## Why It's Good

- **Sandboxed execution** - Strategies run in isolated WASM runtimes. A buggy strategy can't crash the engine or access other strategies' state.
- **Fuel metering** - Resource limits prevent runaway computations from blocking the system.
- **Zero-copy fast path** - The `UnsafePlugin` trait with `Pod` types eliminates serialization overhead for latency-sensitive strategies.
- **AOT compilation** - Pre-compiled `.cwasm` files load instantly with no JIT warmup.
- **Hot reloading** - Load, enable, or disable strategies at runtime via the HTTP API.
- **Multi-venue** - Unified interface across CEX and DeFi venues with a single shared state model.
- **Production-grade persistence** - DuckDB for local analytics, optional ClickHouse for distributed time-series storage.
- **Strict code quality** - Workspace-wide clippy nursery lints and Rust 2018 idioms enforced.

## License

Proprietary
