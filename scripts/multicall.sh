#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   ./multicall.sh add <venue> <id> <wasm_file.cwasm>
#   ./multicall.sh remove <venue> <id>
#
# Example:
#   ./multicall.sh add hyperliquid eth_balances ./eth_balances.cwasm
#   ./multicall.sh remove hyperliquid eth_balances

# API_BASE="http://10.0.0.2:3000/evm"
API_BASE="http://localhost:3000/evm"

if [ $# -lt 1 ]; then
  echo "Usage:"
  echo "  $0 add <venue> <id> <wasm_file.cwasm>"
  echo "  $0 remove <venue> <id>"
  exit 1
fi

CMD="$1"
shift

case "$CMD" in
add)
  if [ $# -lt 3 ]; then
    echo "Usage: $0 add <venue> <id> <wasm_file.cwasm>"
    exit 1
  fi

  VENUE="$1"
  ID="$2"
  WASM_FILE="$3"

  if [ ! -f "$WASM_FILE" ]; then
    echo "Error: File '$WASM_FILE' not found."
    exit 1
  fi

  echo "Adding multicall '$ID' for venue '$VENUE'..."
  RESPONSE=$(
    base64 -w0 "$WASM_FILE" |
      jq -Rs '{ wasm: . }' |
      curl -s -X POST "$API_BASE/$VENUE/multicall/$ID" \
        -H "Content-Type: application/json" \
        -d @-
  )
  ;;

remove)
  if [ $# -lt 2 ]; then
    echo "Usage: $0 remove <venue> <id>"
    exit 1
  fi

  VENUE="$1"
  ID="$2"

  echo "Removing multicall '$ID' from venue '$VENUE'..."
  RESPONSE=$(
    curl -s -X DELETE "$API_BASE/$VENUE/multicall/$ID"
  )
  ;;

*)
  echo "Unknown command: $CMD"
  echo "Commands: add | remove"
  exit 1
  ;;
esac

# Print JSON response (fallback to raw if malformed)
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
