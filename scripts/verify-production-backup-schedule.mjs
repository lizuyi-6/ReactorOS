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
      "ExecStart=/opt/reactor-edge/backup.sh",
      "NoNewPrivileges=true",
      "ProtectSystem=full",
      "ReadWritePaths=-/var/lib/reactor-edge",
    ],
    label: "backup service uses xingshu online SQLite snapshot with sandboxing",
  },
  {
    file: "deploy/reactor-edge-backup.sh",
    mustContain: [
      'XINGSHU_BIN="${REACTOR_EDGE_XINGSHU_BIN:-/opt/reactor-edge/bin/xingshu}"',
      'stamp="$(date -u +%Y%m%d-%H%M%S)"',
      'out="$BACKUP_DIR/reactor.sqlite3.${stamp}.snapshot"',
      'ops backup --out "$out"',
      "latest.snapshot",
      "REACTOR_EDGE_BACKUP_RETAIN_DAYS",
    ],
    label: "backup script writes timestamped snapshots and latest symlink",
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
      "sudo systemctl enable --now reactor-edge-backup.timer",
    ],
    label: "ARM64 package carries backup service and CLI",
  },
  {
    file: "deploy/install-board.sh",
    mustContain: [
      'install -m 0755 "${ROOT}/backup.sh" "${PREFIX}/backup.sh"',
      "reactor-edge-backup.service",
      "reactor-edge-backup.timer",
      'install -d -m 0750 "$DATA_DIR/backups"',
      'chmod 0750 "$DATA_DIR/backups"',
      "systemctl enable reactor-edge-backup.timer",
      "systemctl restart reactor-edge-backup.timer",
      '"${PREFIX}/bin/xingshu"',
    ],
    label: "board installer enables backup timer",
  },
  {
    file: "deploy/board-health.sh",
    mustContain: [
      "reactor-edge-backup.timer",
      "systemctl list-timers reactor-edge-backup.timer",
    ],
    label: "board health reports backup timer",
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
