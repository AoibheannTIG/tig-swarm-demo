#!/bin/sh
# Container entrypoint. Ensures the data dir exists, then execs uvicorn.
# Persistence is provided by the Railway volume mounted at $DATA_DIR.
set -e

DB_DIR="${DATA_DIR:-/app}"
PORT="${PORT:-8080}"

mkdir -p "$DB_DIR"
exec uvicorn server:app --host 0.0.0.0 --port "$PORT"
