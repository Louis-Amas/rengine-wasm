#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   ./strategy.sh add <id> <wasm_file.cwasm>
#   ./strategy.sh execute <wasm_file.cwasm>
#   ./strategy.sh toggle <id> <true|false>
#
# Example:
#   ./strategy.sh add farb ./farb.cwasm
#   ./strategy.sh execute ./dummy.cwasm
#   ./strategy.sh toggle farb false

# API_BASE="http://10.0.0.2:3000/strategies"
API_BASE="http://localhost:3000/strategies"

if [ $# -lt 1 ]; then
  echo "Usage:"
  echo "  $0 add <id> <wasm_file.cwasm>"
  echo "  $0 execute <wasm_file.cwasm>"
  echo "  $0 toggle <id> <true|false>"
  exit 1
fi

CMD="$1"
shift

case "$CMD" in
add)
  if [ $# -lt 2 ]; then
    echo "Usage: $0 add <id> <wasm_file.cwasm>"
    exit 1
  fi
  STRATEGY_ID="$1"
  WASM_FILE="$2"

  if [ ! -f "$WASM_FILE" ]; then
    echo "Error: File '$WASM_FILE' not found."
    exit 1
  fi

  echo "Adding strategy '$STRATEGY_ID'..."
  RESPONSE=$(
    base64 -i "$WASM_FILE" |
      tr -d '\n' |
      jq -Rs '{ wasm: . }' |
      curl -s -X POST "$API_BASE/$STRATEGY_ID" \
        -H "Content-Type: application/json" \
        -d @-
  )
  ;;

execute)
  if [ $# -lt 1 ]; then
    echo "Usage: $0 execute <wasm_file.cwasm>"
    exit 1
  fi
  WASM_FILE="$1"

  if [ ! -f "$WASM_FILE" ]; then
    echo "Error: File '$WASM_FILE' not found."
    exit 1
  fi

  echo "Executing strategy..."
  RESPONSE=$(
    base64 -i "$WASM_FILE" |
      tr -d '\n' |
      jq -Rs '{ wasm: . }' |
      curl -s -X POST "$API_BASE/execute" \
        -H "Content-Type: application/json" \
        -d @-
  )
  ;;

toggle)
  if [ $# -lt 2 ]; then
    echo "Usage: $0 toggle <id> <true|false>"
    exit 1
  fi
  STRATEGY_ID="$1"
  ENABLED="$2"

  echo "Toggling strategy '$STRATEGY_ID' to enabled=$ENABLED..."
  RESPONSE=$(
    jq -n --argjson enabled "$ENABLED" '{ enabled: $enabled }' |
      curl -s -X PUT "$API_BASE/$STRATEGY_ID/toggle" \
        -H "Content-Type: application/json" \
        -d @-
  )
  ;;

*)
  echo "Unknown command: $CMD"
  echo "Commands: add | execute | toggle"
  exit 1
  ;;
esac

# Print JSON response (fallback to raw if malformed)
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
