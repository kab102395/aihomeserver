#!/usr/bin/env sh
set -eu

MODE="${1:-quick}"
BASE_URL="${BASE_URL:-http://localhost:3000}"

if [ "$MODE" = "full" ]; then
  TIMEOUT=30
else
  TIMEOUT=15
fi

echo "POST $BASE_URL/eval/run ($MODE)"
curl -sS -X POST "$BASE_URL/eval/run" \
  -H 'content-type: application/json' \
  -d "{\"mode\":\"$MODE\",\"timeout_secs\":$TIMEOUT}" | jq .

