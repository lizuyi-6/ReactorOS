#!/usr/bin/env bash
set -euo pipefail

DB_PATH="${REACTOR_EDGE_DB:-/var/lib/reactor-edge/reactor.sqlite3}"
BACKUP_DIR="${REACTOR_EDGE_BACKUP_DIR:-/var/lib/reactor-edge/backups}"
RETAIN_DAYS="${REACTOR_EDGE_BACKUP_RETAIN_DAYS:-30}"
XINGSHU_BIN="${REACTOR_EDGE_XINGSHU_BIN:-/opt/reactor-edge/current/bin/xingshu}"
LOCK_FILE="${REACTOR_EDGE_BACKUP_LOCK:-${BACKUP_DIR}/.reactor-edge-backup.lock}"

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
for required_command in awk date find flock grep ln mkdir mv rm sha256sum sync; do
  if ! command -v "$required_command" >/dev/null 2>&1; then
    echo "missing required command for backup: $required_command" >&2
    exit 2
  fi
done

mkdir -p "$BACKUP_DIR"
exec 8>"$LOCK_FILE"
if ! flock -n 8; then
  echo "another backup is already running: $LOCK_FILE" >&2
  exit 75
fi

stamp="$(date -u +%Y%m%d-%H%M%S)"
out="$BACKUP_DIR/reactor.sqlite3.${stamp}.snapshot"
tmp_out="${out}.tmp.$$"
tmp_sha="${tmp_out}.sha256"

cleanup_tmp() {
  rm -f "$tmp_out" "$tmp_sha"
}
trap cleanup_tmp EXIT

"$XINGSHU_BIN" --db "$DB_PATH" ops backup --out "$tmp_out"

if [[ ! -s "$tmp_out" ]]; then
  echo "backup command produced an empty snapshot: $tmp_out" >&2
  exit 1
fi
if [[ ! -f "$tmp_sha" ]]; then
  echo "backup command did not produce sha256 sidecar: $tmp_sha" >&2
  exit 1
fi
sha256sum -c "$tmp_sha" >/dev/null
if ! LC_ALL=C grep -a -q '^SQLite format 3' "$tmp_out"; then
  echo "backup snapshot does not have SQLite magic header: $tmp_out" >&2
  exit 1
fi
digest="$(awk '{ print $1; exit }' "$tmp_sha")"
if [[ ! "$digest" =~ ^[0-9A-Fa-f]{64}$ ]]; then
  echo "backup sha256 sidecar contains invalid digest: $tmp_sha" >&2
  exit 1
fi

mv -f "$tmp_out" "$out"
printf '%s  %s\n' "$digest" "$out" >"${out}.sha256"
sync

ln -sfn "$(basename "$out")" "$BACKUP_DIR/latest.snapshot"
ln -sfn "$(basename "${out}.sha256")" "$BACKUP_DIR/latest.snapshot.sha256"
sync

if [[ "$RETAIN_DAYS" -gt 0 ]]; then
  find "$BACKUP_DIR" -maxdepth 1 -type f -name 'reactor.sqlite3.*.snapshot' -mtime +"$RETAIN_DAYS" -delete
  find "$BACKUP_DIR" -maxdepth 1 -type f -name 'reactor.sqlite3.*.snapshot.sha256' -mtime +"$RETAIN_DAYS" -delete
fi

echo "backup snapshot written: $out"
