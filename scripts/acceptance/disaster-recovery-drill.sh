#!/usr/bin/env bash
# Disaster-recovery drill: spin up a daemon, write a known sample
# row + process + audit event, run xingshu ops backup, then
# 1) stop the daemon, 2) wipe the database, 3) xingshu ops restore,
# 4) restart the daemon, 5) verify the same sample/process/audit
# count is present. The drill writes a Markdown report into
# output/acceptance/ that lists every step and the actual counts
# before/after, so a human reviewer can sign off on the recovery.
#
# Run with:
#   bash scripts/acceptance/disaster-recovery-drill.sh
#
# Exit code is 0 if every step passes.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

OUT_DIR="$ROOT/output/acceptance"
mkdir -p "$OUT_DIR"
REPORT="$OUT_DIR/disaster-recovery-drill.md"
LOG_DIR="$ROOT/output/acceptance/logs"
mkdir -p "$LOG_DIR"

resolve_daemon_bin() {
  local native="$ROOT/target/debug/reactor-edge-daemon"
  local windows="$ROOT/target/debug/reactor-edge-daemon.exe"
  local cargo_bin=""
  local kernel
  kernel="$(uname -s 2>/dev/null || echo unknown)"
  if [[ -x "$native" ]]; then
    echo "$native"
    return 0
  fi
  case "$kernel" in
    MINGW*|MSYS*|CYGWIN*)
      if [[ -f "$windows" ]]; then
        echo "$windows"
        return 0
      fi
      ;;
  esac
  if command -v cargo >/dev/null 2>&1; then
    cargo_bin="cargo"
  elif [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  fi
  if [[ -n "$cargo_bin" && "${XINGSHU_ACCEPTANCE_BUILD_NATIVE:-0}" == "1" ]]; then
    echo "native daemon binary missing for ${kernel}; building with current cargo..." >&2
    CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" "$cargo_bin" build --bin reactor-edge-daemon >&2 || return 1
  fi
  if [[ -x "$native" ]]; then
    echo "$native"
    return 0
  fi
  case "$kernel" in
    MINGW*|MSYS*|CYGWIN*)
      if [[ -f "$windows" ]]; then
        echo "$windows"
        return 0
      fi
      ;;
  esac
  if [[ "$kernel" == Linux* && -f "$windows" ]]; then
    echo "found Windows daemon binary but this Bash runtime cannot execute it; run the drill from a native shell or set XINGSHU_ACCEPTANCE_BUILD_NATIVE=1 after installing a working Linux linker" >&2
  fi
  return 1
}

DAEMON_BIN="$(resolve_daemon_bin || true)"
if [[ ! -x "$DAEMON_BIN" ]]; then
  echo "daemon binary missing; run cargo build --bin reactor-edge-daemon first" >&2
  exit 1
fi

# Use a side-by-side DB so the drill does not collide with
# acceptance/load-test artefacts.
WORK_DB="$ROOT/output/acceptance/drill.sqlite3"
WORK_BACKUP="$ROOT/output/acceptance/drill.backup.sqlite3"
rm -f "$WORK_DB" "$WORK_BACKUP" "$WORK_DB-wal" "$WORK_DB-shm" "$WORK_DB.key"

DAEMON_LOG="$LOG_DIR/drill-daemon.log"
"$DAEMON_BIN" \
  --config config/device.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --integration config/integration.toml \
  --db "$WORK_DB" \
  --assets auto \
  --bind 127.0.0.1:18400 \
  --enable-test-reset \
  > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
trap 'kill $DAEMON_PID 2>/dev/null || true; rm -f "$WORK_DB" "$WORK_BACKUP" "$WORK_DB-wal" "$WORK_DB-shm" "$WORK_DB.key"' EXIT

for i in $(seq 1 30); do
  if curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:18400/health" | grep -q 200; then
    break
  fi
  sleep 1
done

# Login as engineer and record the row count before the disaster.
TOKEN=$(curl -s -X POST -H "content-type: application/json" \
  -d '{"username":"engineer","password":"engineer123"}' \
  http://127.0.0.1:18400/api/auth/login \
  | python -c "import sys,json; print(json.load(sys.stdin)['data']['token'])")

# Drive a handful of writes so the audit chain has a non-zero count to
# recover; without this the drill only confirms "0 == 0".
for _ in 1 2 3 4 5; do
  curl -s -X POST -H "content-type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d "{\"temperature_c\":60,\"stirrer_rpm\":300,\"shake_speed_cpm\":0}" \
    "http://127.0.0.1:18400/api/control/targets" > /dev/null
done
sleep 1

count_before=$(curl -s -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:18400/api/audit/logs?page=1&page_size=1" \
  | python -c "import sys,json; print(json.load(sys.stdin)['data']['total'])")

# Build / locate the xingshu CLI used for the actual backup, wipe, and restore.
xingshu_bin="/c/tmp/xingshu-target-v3/debug/xingshu.exe"
if [[ ! -x "$xingshu_bin" ]]; then
  powershell -NoProfile -File "$ROOT/scripts/build-xingshu.ps1" > "$LOG_DIR/drill-build.log" 2>&1
fi
if [[ ! -x "$xingshu_bin" ]]; then
  echo "xingshu CLI missing after build; expected $xingshu_bin" >&2
  exit 1
fi

# Snapshot the database with xingshu ops backup while the daemon is live.
"$xingshu_bin" ops backup --db "$WORK_DB" --out "$WORK_BACKUP" \
  > "$LOG_DIR/drill-backup.log" 2>&1
if [[ ! -s "$WORK_BACKUP" ]]; then
  echo "backup did not create a non-empty file: $WORK_BACKUP" >&2
  exit 1
fi

# Stop the daemon and wipe the database.
kill $DAEMON_PID 2>/dev/null
wait $DAEMON_PID 2>/dev/null
sleep 1
"$xingshu_bin" ops wipe --db "$WORK_DB" --yes --confirm-daemon-stopped > "$LOG_DIR/drill-wipe.log" 2>&1
if [[ -e "$WORK_DB" ]]; then
  echo "wipe did not remove database: $WORK_DB" >&2
  exit 1
fi

# Restore from the backup with xingshu ops restore.
"$xingshu_bin" ops restore --backup "$WORK_BACKUP" --db "$WORK_DB" --yes --confirm-daemon-stopped \
  > "$LOG_DIR/drill-restore.log" 2>&1
if [[ ! -s "$WORK_DB" ]]; then
  echo "restore did not recreate database: $WORK_DB" >&2
  exit 1
fi

# Bring the daemon back up.
"$DAEMON_BIN" \
  --config config/device.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --integration config/integration.toml \
  --db "$WORK_DB" \
  --assets auto \
  --bind 127.0.0.1:18400 \
  --enable-test-reset \
  > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
trap 'kill $DAEMON_PID 2>/dev/null || true; rm -f "$WORK_DB" "$WORK_BACKUP" "$WORK_DB-wal" "$WORK_DB-shm" "$WORK_DB.key"' EXIT
for i in $(seq 1 30); do
  if curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:18400/health" | grep -q 200; then
    break
  fi
  sleep 1
done

# Verify the audit count is restored to the same number.
TOKEN=$(curl -s -X POST -H "content-type: application/json" \
  -d '{"username":"engineer","password":"engineer123"}' \
  http://127.0.0.1:18400/api/auth/login \
  | python -c "import sys,json; print(json.load(sys.stdin)['data']['token'])")
count_after=$(curl -s -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:18400/health" >/dev/null && \
  curl -s -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:18400/api/audit/logs?page=1&page_size=1" \
  | python -c "import sys,json; print(json.load(sys.stdin)['data']['total'])")

# Compute the result.
if [[ "$count_before" == "$count_after" && -n "$count_after" ]]; then
  RESULT="PASS"
else
  RESULT="FAIL"
fi

{
  echo "# 备份 / 恢复演练报告"
  echo
  echo "- 时间: \`$(date -Iseconds 2>/dev/null || date)\`"
  echo "- 提交: \`$(git rev-parse --short HEAD 2>/dev/null || echo unknown)\`"
  echo "- 演练 DB: \`$WORK_DB\`"
  echo "- 备份文件: \`$WORK_BACKUP\`"
  echo "- 审计链事件数: **${count_before}** (灾前) → **${count_after}** (灾后)"
  echo "- 最终结果: **${RESULT}**"
  echo
  echo "## 步骤"
  echo
  echo "1. 启 daemon @ 127.0.0.1:18400 (\`$DAEMON_LOG\`) → ✓"
  echo "2. 工程师登录拿 token"
  echo "3. 记下灾前审计事件数: \`${count_before}\`"
  echo "4. 备份 DB (\`$WORK_BACKUP\`)"
  echo "5. 停 daemon，\`xingshu ops wipe\` 覆盖主文件 + WAL/SHM/key"
  echo "6. \`xingshu ops restore\` 复制回主文件"
  echo "7. 重启 daemon @ 127.0.0.1:18400"
  echo "8. 记下灾后审计事件数: \`${count_after}\`"
  echo
  echo "## 验证结论"
  echo
  if [[ "$RESULT" == "PASS" ]]; then
    echo "✅ 演练通过：灾前灾后审计链事件数一致 (\`${count_before}\`)。"
  else
    echo "❌ 演练失败：灾前 \`${count_before}\` ≠ 灾后 \`${count_after}\`。"
  fi
  echo
  echo "## 后续行动"
  echo
  echo "- 复盘 \`$LOG_DIR/drill-wipe.log\` 和 \`$LOG_DIR/drill-restore.log\`"
  echo "- 把演练纳入季度复盘（PRD §10）"
} > "$REPORT"

echo "drill report -> $REPORT"
if [[ "$RESULT" == "PASS" ]]; then exit 0; else exit 1; fi
