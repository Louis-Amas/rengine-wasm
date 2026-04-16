# Rengine MCP Server

Rengine exposes an [MCP (Model Context Protocol)](https://modelcontextprotocol.io/) server that lets AI assistants and other clients read engine state, place/cancel orders, manage strategies, and control EVM readers — all over HTTP.

## Configuration

Add `mcp_port` to your config TOML to enable the server:

```toml
mcp_port = 8423
```

The server starts on `0.0.0.0:<mcp_port>` and serves the MCP endpoint at `/mcp`.

## Transport

The server uses **Streamable HTTP** transport (not SSE or stdio). Requests must include:

```
Content-Type: application/json
Accept: application/json, text/event-stream
```

After initializing a session, include the `Mcp-Session-Id` header in all subsequent requests.

## Quick Start

### 1. Initialize a session

```bash
curl -s -X POST http://localhost:8423/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": { "name": "my-client", "version": "0.1" }
    }
  }'
```

The response includes a `Mcp-Session-Id` header — use it for all subsequent requests.

### 2. List available tools

```bash
curl -s -X POST http://localhost:8423/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Mcp-Session-Id: <SESSION_ID>' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
```

### 3. Call a tool

```bash
curl -s -X POST http://localhost:8423/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'Mcp-Session-Id: <SESSION_ID>' \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": { "name": "get_balances", "arguments": {} }
  }'
```

## Tools Reference

### State Reading

#### `get_balances`

Get account balances across all venues.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | no | Filter by venue (e.g. `"hyperliquid"`) |
| `symbol` | string | no | Filter by symbol (e.g. `"usdc"`) |

Example response:

```json
{
  "hyperliquid|perp|default|usdc": "125.50"
}
```

#### `get_positions`

Get open positions.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | no | Filter by venue |
| `instrument` | string | no | Filter by instrument (e.g. `"eth/usd-perp"`) |

#### `get_order_book`

Get top-of-book (best bid/ask) for instruments.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | no | Filter by venue |
| `instrument` | string | no | Filter by instrument |

Example response:

```json
[
  {
    "key": "hyperliquid|eth/usd-perp",
    "mid": "2340.75",
    "top_ask": { "price": "2340.8", "size": "135.8756" },
    "top_bid": { "price": "2340.7", "size": "151.3635" }
  }
]
```

#### `get_open_orders`

Get open orders.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | no | Filter by venue |
| `instrument` | string | no | Filter by instrument |

#### `get_indicators`

Get indicator values (funding rates, custom indicators, etc).

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `prefix` | string | no | Filter keys by prefix (e.g. `"hyperliquid-funding"`) |

Example response:

```json
{
  "hyperliquid-funding-eth": "-0.0000286371"
}
```

#### `get_market_specs`

Get market specifications (decimals, increments, leverage limits).

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | no | Filter by venue |
| `instrument` | string | no | Filter by instrument |

Example response:

```json
[
  {
    "key": "hyperliquid|eth/usd-perp",
    "symbol": "eth/usd-perp",
    "marketType": "perp",
    "priceDecimals": 2,
    "priceIncrement": "0.01",
    "sizeDecimals": 4,
    "sizeIncrement": "0.0001",
    "minSize": "0.0001",
    "minNotional": "10",
    "minPrice": "0.01",
    "contractSize": "1",
    "maxLeverage": 25
  }
]
```

### Order Management

#### `place_orders`

Place one or more orders on an exchange.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | yes | Exchange venue (e.g. `"hyperliquid"`) |
| `market_type` | string | yes | `"spot"` or `"perp"` |
| `account_id` | string | yes | Account identifier (e.g. `"default"`) |
| `orders_json` | string | yes | JSON array string of order objects (see below) |
| `execution_type` | string | no | `"Managed"` or `"Unmanaged"` (default: `"Unmanaged"`) |

Each order object in `orders_json`:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `instrument` | string | yes | e.g. `"eth/usd-perp"` |
| `side` | string | yes | `"Ask"` or `"Bid"` |
| `price` | string | yes | Decimal price |
| `size` | string | yes | Decimal size |
| `time_in_force` | string | no | `"PostOnly"`, `"GoodUntilCancelled"` (default), `"ImmediateOrCancel"` |
| `order_type` | string | no | `"Limit"` (default), `"Market"`, `"Pegged"` |
| `client_order_id` | string | no | Custom order ID |

Example:

```json
{
  "venue": "hyperliquid",
  "market_type": "perp",
  "account_id": "default",
  "orders_json": "[{\"instrument\":\"eth/usd-perp\",\"side\":\"Bid\",\"price\":\"2300.00\",\"size\":\"0.01\",\"time_in_force\":\"PostOnly\"}]"
}
```

#### `cancel_orders`

Cancel one or more orders.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | yes | Exchange venue |
| `market_type` | string | yes | `"spot"` or `"perp"` |
| `account_id` | string | yes | Account identifier |
| `cancellations_json` | string | yes | JSON array string of cancellation objects (see below) |

Each cancellation object:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `instrument` | string | yes | e.g. `"eth/usd-perp"` |
| `order_id` | string | yes | Order ID to cancel |
| `ref_type` | string | no | `"external"` (default) or `"client"` |

### Strategy Management

#### `add_strategy`

Add and enable a WASM strategy.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Strategy identifier |
| `wasm_base64` | string | yes | Base64-encoded compiled WASM binary |

#### `toggle_strategy`

Enable or disable an existing strategy.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | yes | Strategy identifier |
| `enabled` | boolean | yes | `true` to enable, `false` to disable |

#### `execute_strategy`

Execute a one-off WASM strategy without persisting it. Returns the execution result with emitted requests and logs.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `wasm_base64` | string | yes | Base64-encoded compiled WASM binary |

### EVM Reader Management

#### `add_multicall_reader`

Add an EVM multicall reader to a venue.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | yes | EVM venue/chain name (e.g. `"arbitrum"`) |
| `id` | string | yes | Reader identifier |
| `wasm_base64` | string | yes | Base64-encoded compiled WASM binary |

#### `remove_multicall_reader`

Remove an EVM multicall reader from a venue.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | yes | EVM venue/chain name |
| `id` | string | yes | Reader identifier |

#### `add_log_processor`

Add an EVM log processor to a venue.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | yes | EVM venue/chain name |
| `id` | string | yes | Processor identifier |
| `wasm_base64` | string | yes | Base64-encoded compiled WASM binary |

#### `remove_log_processor`

Remove an EVM log processor from a venue.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `venue` | string | yes | EVM venue/chain name |
| `id` | string | yes | Processor identifier |

## Using with Claude Code

You can connect this MCP server to Claude Code by adding it to your settings:

```json
{
  "mcpServers": {
    "rengine": {
      "type": "streamable-http",
      "url": "http://localhost:8423/mcp"
    }
  }
}
```

This gives Claude direct access to your engine's state, order management, and strategy deployment.
