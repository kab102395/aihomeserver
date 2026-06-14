#!/bin/sh
set -eu

ROLE="${ROLE:-coordinator}"

case "$ROLE" in
  coordinator)
    exec /app/aihomeserver
    ;;
  worker)
    exec /app/worker
    ;;
  *)
    echo "Unknown ROLE: $ROLE" >&2
    exit 1
    ;;
esac
