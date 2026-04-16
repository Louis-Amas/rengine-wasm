#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   ./transformers.sh add <id> <wasm_file.cwasm>
#   ./transformers.sh execute <wasm_file.cwasm>
#   ./transformers.sh toggle <id> <true|false>
#
# Example:
#   ./transformers.sh add ema_price ./ema_price.cwasm
#   ./transformers.sh execute ./dummy.cwasm
#   ./transformers.sh toggle ema_price false

# API_BASE="http://10.0.0.2:3000/transformers"
API_BASE="http://localhost:3000/transformers"

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
  TRANSFORMER_ID="$1"
  WASM_FILE="$2"

  if [ ! -f "$WASM_FILE" ]; then
    echo "Error: File '$WASM_FILE' not found."
    exit 1
  fi

  echo "Adding transformer '$TRANSFORMER_ID'..."
  RESPONSE=$(
    base64 -i "$WASM_FILE" |
      tr -d '\n' |
      jq -Rs '{ wasm: . }' |
      curl -s -X POST "$API_BASE/$TRANSFORMER_ID" \
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

  echo "Executing transformer..."
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
  TRANSFORMER_ID="$1"
  ENABLED="$2"

  echo "Toggling transformer '$TRANSFORMER_ID' to enabled=$ENABLED..."
  RESPONSE=$(
    jq -n --argjson enabled "$ENABLED" '{ enabled: $enabled }' |
      curl -s -X PUT "$API_BASE/$TRANSFORMER_ID/toggle" \
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
