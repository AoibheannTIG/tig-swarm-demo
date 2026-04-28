#!/bin/sh
# Container entrypoint.
#
# - If LITESTREAM_BUCKET is set, attempt to restore swarm.db from the configured
#   replica on first boot (volume is empty), then run uvicorn under
#   `litestream replicate -exec` so every WAL frame is streamed to R2/S3 while
#   the server runs.
# - If LITESTREAM_BUCKET is unset, run uvicorn directly. Useful for local dev.
set -e

DB_DIR="${DATA_DIR:-/app}"
DB_PATH="${DB_DIR}/swarm.db"
PORT="${PORT:-8080}"
UVICORN_CMD="uvicorn server:app --host 0.0.0.0 --port ${PORT}"

if [ -n "$LITESTREAM_BUCKET" ]; then
    mkdir -p "$DB_DIR"
    if [ ! -f "$DB_PATH" ]; then
        echo "[entrypoint] No DB at $DB_PATH; attempting Litestream restore from \$LITESTREAM_BUCKET"
        litestream restore -if-replica-exists "$DB_PATH" || \
            echo "[entrypoint] No replica found; starting fresh"
    fi
    echo "[entrypoint] Starting uvicorn under Litestream replication"
    exec litestream replicate -exec "$UVICORN_CMD"
else
    echo "[entrypoint] LITESTREAM_BUCKET unset; running uvicorn without backup replication"
    exec sh -c "$UVICORN_CMD"
fi
