#!/usr/bin/env bash
# Runtime perf probe for the LubanCat 2 / RK3568 ARM64 release binary, run
# inside an arm64 Debian buster container under QEMU emulation. Starts the
# daemon, waits for /health, samples RSS + CPU, then stops. NOTE: QEMU
# user-mode emulation adds overhead and does NOT reflect native A55 CPU%, but
# RSS (resident memory) and "does the optimized arm64 binary actually run on a
# Debian 10 sysroot" are meaningful signals.
set -uo pipefail

PKG=/work/pkg
OUT=/work/out/cat2-perf-probe.json
BIND=127.0.0.1:8000

apt-get update >/dev/null 2>&1
apt-get install -y --no-install-recommends ca-certificates libudev1 curl procps >/dev/null 2>&1

echo "=== arch / binary check ==="
uname -m
file "$PKG/bin/reactor-edge-daemon" | sed 's/,.*//'

cd "$PKG"
mkdir -p data
# Default external-pipeline mode: /api/live returns 503 without samples, which
# is expected. We only need the process up to measure resident memory.
./bin/reactor-edge-daemon \
  --config config/device.json_bridge.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --db data/reactor.sqlite3 \
  --assets frontend/dist \
  --bind "$BIND" \
  --seed-demo-context &
PID=$!

# Wait for /health up to 60s (QEMU start is slow).
UP=0
for i in $(seq 1 60); do
  if curl -fsS "http://$BIND/health" >/dev/null 2>&1; then UP=1; break; fi
  if ! kill -0 "$PID" 2>/dev/null; then echo "daemon exited early"; break; fi
  sleep 1
done
echo "=== health up: $UP (after ${i}s) ==="

HEALTH=$(curl -fsS "http://$BIND/health" 2>/dev/null || echo "{}")
DEVSTATUS=$(curl -fsS "http://$BIND/api/devices/status" 2>/dev/null | head -c 200 || echo "{}")

# Sample RSS (KB) and CPU% over a few seconds once warm.
sleep 3
RSS_KB=$(ps -o rss= -p "$PID" 2>/dev/null | tr -d ' ')
CPU_PCT=$(ps -o %cpu= -p "$PID" 2>/dev/null | tr -d ' ')
# Second sample after load probes.
for r in 1 2 3 4 5; do curl -fsS "http://$BIND/api/devices/status" >/dev/null 2>&1; done
RSS_KB2=$(ps -o rss= -p "$PID" 2>/dev/null | tr -d ' ')

kill "$PID" 2>/dev/null
wait "$PID" 2>/dev/null

RSS_MB=$(awk "BEGIN{printf \"%.2f\", ${RSS_KB:-0}/1024}")
RSS_MB2=$(awk "BEGIN{printf \"%.2f\", ${RSS_KB2:-0}/1024}")

mkdir -p /work/out
cat > "$OUT" <<JSON
{
  "platform": "arm64 Debian buster under QEMU user-mode (not native A55)",
  "binary": "reactor-edge-daemon (release, lto=fat, stripped)",
  "health_up": $UP,
  "health": $HEALTH,
  "device_status_head": $(printf '%s' "$DEVSTATUS" | sed 's/"/\\"/g; s/^/"/; s/$/"/'),
  "rss_mb_warm": $RSS_MB,
  "rss_mb_after_probes": $RSS_MB2,
  "cpu_pct_sample": "${CPU_PCT:-NA}",
  "note": "RSS is meaningful; CPU% under QEMU is emulation-inflated and not a native A55 figure"
}
JSON
echo "=== result ==="
cat "$OUT"
