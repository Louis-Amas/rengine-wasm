#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   ./logs.sh add <venue> <id> <wasm_file.cwasm>
#   ./logs.sh remove <venue> <id>
#
# Example:
#   ./logs.sh add anvil test_logs ./evm_logs_wasm/test_logs.cwasm
#   ./logs.sh remove anvil test_logs

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
  LOG_ID="$2"
  WASM_FILE="$3"

  if [ ! -f "$WASM_FILE" ]; then
    echo "Error: File '$WASM_FILE' not found."
    exit 1
  fi

  echo "Adding log reader '$LOG_ID' for venue '$VENUE'..."
  RESPONSE=$(
    base64 -w0 "$WASM_FILE" |
      jq -Rs '{ wasm: . }' |
      curl -s -X POST "$API_BASE/$VENUE/logs/$LOG_ID" \
        -H "Content-Type: application/json" \
        -d @-
  )
  echo "Response: $RESPONSE"
  ;;

remove)
  if [ $# -lt 2 ]; then
    echo "Usage: $0 remove <venue> <id>"
    exit 1
  fi
  VENUE="$1"
  LOG_ID="$2"

  echo "Removing log reader '$LOG_ID' from venue '$VENUE'..."
  curl -s -X DELETE "$API_BASE/$VENUE/logs/$LOG_ID"
  echo ""
  ;;

*)
  echo "Unknown command: $CMD"
  exit 1
  ;;
esac
