#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const checks = [
  {
    file: "deploy/reactor-edge-backup.service",
    mustContain: [
      "Description=Reactor Edge SQLite Backup Snapshot",
      "ExecStartPre=/usr/bin/mkdir -p /var/lib/reactor-edge/backups",
      "ExecStart=/opt/reactor-edge/current/backup.sh",
      "NoNewPrivileges=true",
      "ProtectSystem=full",
      "ReadWritePaths=-/var/lib/reactor-edge",
    ],
    label: "backup service uses xingshu online SQLite snapshot with sandboxing",
  },
  {
    file: "deploy/reactor-edge-backup.sh",
    mustContain: [
      'XINGSHU_BIN="${REACTOR_EDGE_XINGSHU_BIN:-/opt/reactor-edge/current/bin/xingshu}"',
      'LOCK_FILE="${REACTOR_EDGE_BACKUP_LOCK:-${BACKUP_DIR}/.reactor-edge-backup.lock}"',
      'exec 8>"$LOCK_FILE"',
      "flock -n 8",
      "another backup is already running",
      "missing required command for backup",
      'stamp="$(date -u +%Y%m%d-%H%M%S)"',
      'out="$BACKUP_DIR/reactor.sqlite3.${stamp}.snapshot"',
      'tmp_out="${out}.tmp.$$"',
      'tmp_sha="${tmp_out}.sha256"',
      'ops backup --out "$tmp_out"',
      'sha256sum -c "$tmp_sha"',
      "SQLite format 3",
      'digest="$(awk \'{ print $1; exit }\' "$tmp_sha")',
      "^[0-9A-Fa-f]{64}$",
      'mv -f "$tmp_out" "$out"',
      "printf '%s  %s\\n' \"$digest\" \"$out\" >\"${out}.sha256\"",
      "sync",
      "latest.snapshot",
      "latest.snapshot.sha256",
      "REACTOR_EDGE_BACKUP_RETAIN_DAYS",
    ],
    label: "backup script validates temporary snapshots before publishing latest",
  },
  {
    file: "deploy/reactor-edge-backup.timer",
    mustContain: [
      "Description=Daily Reactor Edge SQLite Backup Snapshot",
      "OnCalendar=*-*-* 02:17:00",
      "Persistent=true",
      "RandomizedDelaySec=15m",
      "WantedBy=timers.target",
    ],
    label: "backup timer is daily and persistent",
  },
  {
    file: "scripts/package-a55-debian10.sh",
    mustContain: [
      'XINGSHU_BIN="${TARGET_DIR}/${TARGET}/${PROFILE}/xingshu"',
      'cp "${XINGSHU_BIN}" "${PACKAGE_DIR}/bin/xingshu"',
      "cp deploy/reactor-edge-backup.service",
      "cp deploy/reactor-edge-backup.timer",
      'cp deploy/reactor-edge-backup.sh "${PACKAGE_DIR}/backup.sh"',
      'cp deploy/reactor-edge-ota-update.sh "${PACKAGE_DIR}/ota-update.sh"',
      'sha256sum "${PACKAGE_NAME}.tar.gz"',
      "sudo ./install.sh",
    ],
    label: "ARM64 package carries backup service and CLI",
  },
  {
    file: "deploy/install-board.sh",
    mustContain: [
      'install -m 0755 "${ROOT}/backup.sh" "${SLOT_DIR}/backup.sh"',
      "reactor-edge-backup.service",
      "reactor-edge-backup.timer",
      'link_or_preserve_existing "${PREFIX}/current" "$SLOT_DIR"',
      'install -d -m 0750 "$DATA_DIR/backups"',
      'chmod 0750 "$DATA_DIR/backups"',
      "systemctl enable reactor-edge-backup.timer",
      "systemctl restart reactor-edge-backup.timer",
      '"${SLOT_DIR}/bin/xingshu"',
    ],
    label: "board installer enables backup timer",
  },
  {
    file: "deploy/board-health.sh",
    mustContain: [
      "--production",
      "STATUS_URL",
      "production_state=safe_idle",
      "reactor-edge-backup.timer",
      "systemctl list-timers reactor-edge-backup.timer",
    ],
    label: "board health reports backup timer and production state",
  },
];

const failures = [];
for (const check of checks) {
  const fullPath = path.join(root, check.file);
  let text;
  try {
    text = await readFile(fullPath, "utf8");
  } catch (error) {
    failures.push(`${check.label}: cannot read ${check.file}: ${error.message}`);
    continue;
  }
  for (const needle of check.mustContain) {
    if (!text.includes(needle)) {
      failures.push(`${check.label}: ${check.file} must contain ${JSON.stringify(needle)}`);
    }
  }
}

if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("Production backup schedule gate passed");
