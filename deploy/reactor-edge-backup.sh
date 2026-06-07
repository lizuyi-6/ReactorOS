#!/usr/bin/env bash
set -euo pipefail

DB_PATH="${REACTOR_EDGE_DB:-/var/lib/reactor-edge/reactor.sqlite3}"
BACKUP_DIR="${REACTOR_EDGE_BACKUP_DIR:-/var/lib/reactor-edge/backups}"
RETAIN_DAYS="${REACTOR_EDGE_BACKUP_RETAIN_DAYS:-30}"
XINGSHU_BIN="${REACTOR_EDGE_XINGSHU_BIN:-/opt/reactor-edge/bin/xingshu}"

if [[ ! -x "$XINGSHU_BIN" ]]; then
  echo "missing xingshu binary: $XINGSHU_BIN" >&2
  exit 1
fi
if [[ ! -f "$DB_PATH" ]]; then
  echo "missing database: $DB_PATH" >&2
  exit 1
fi
if [[ "$RETAIN_DAYS" =~ ^[0-9]+$ ]]; then
  :
else
  echo "REACTOR_EDGE_BACKUP_RETAIN_DAYS must be a non-negative integer" >&2
  exit 2
fi

mkdir -p "$BACKUP_DIR"
stamp="$(date -u +%Y%m%d-%H%M%S)"
out="$BACKUP_DIR/reactor.sqlite3.${stamp}.snapshot"

"$XINGSHU_BIN" --db "$DB_PATH" ops backup --out "$out"

ln -sfn "$(basename "$out")" "$BACKUP_DIR/latest.snapshot"
if [[ -f "${out}.sha256" ]]; then
  ln -sfn "$(basename "${out}.sha256")" "$BACKUP_DIR/latest.snapshot.sha256"
fi

if [[ "$RETAIN_DAYS" -gt 0 ]]; then
  find "$BACKUP_DIR" -maxdepth 1 -type f -name 'reactor.sqlite3.*.snapshot' -mtime +"$RETAIN_DAYS" -delete
  find "$BACKUP_DIR" -maxdepth 1 -type f -name 'reactor.sqlite3.*.snapshot.sha256' -mtime +"$RETAIN_DAYS" -delete
fi

echo "backup snapshot written: $out"
