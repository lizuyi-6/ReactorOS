#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const checks = [
  {
    file: "deploy/reactor-edge.service",
    mustContain: [
      "--safety-guard /opt/reactor-edge/current/bin/reactor-safety-guard",
      "Restart=on-failure",
      "StartLimitIntervalSec=600",
      "StartLimitBurst=5",
      "NoNewPrivileges=true",
      "ProtectSystem=full",
      "ProtectHome=true",
    ],
    label: "systemd service enables isolated safety guard with sandboxing",
  },
  {
    file: "scripts/package-a55-debian10.sh",
    mustContain: [
      'SAFETY_GUARD_BIN="${TARGET_DIR}/${TARGET}/${PROFILE}/reactor-safety-guard"',
      'cp "${SAFETY_GUARD_BIN}" "${PACKAGE_DIR}/bin/reactor-safety-guard"',
      '--safety-guard "${ROOT}/bin/reactor-safety-guard"',
      '"${PACKAGE_DIR}/bin/reactor-safety-guard"',
    ],
    label: "ARM64 package includes and launches safety guard",
  },
  {
    file: "deploy/install-board.sh",
    mustContain: [
      'copy_tree "${ROOT}/bin" "${SLOT_DIR}/bin"',
      'chmod +x "${SLOT_DIR}/bin/reactor-edge-daemon"',
      '"${SLOT_DIR}/bin/reactor-safety-guard"',
    ],
    label: "board installer copies package binaries",
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

console.log("Production safety guard gate passed");
