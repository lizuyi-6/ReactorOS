#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const checks = [
  {
    file: "frontend/dist/index.html",
    mustContain: ['id="app"'],
    label: "Vue production single-file HMI exists",
  },
  {
    file: "deploy/reactor-edge.service",
    mustContain: ["--assets auto"],
    mustNotContain: ["/opt/reactor-edge/static"],
    label: "systemd daemon service uses auto assets",
  },
  {
    file: "deploy/install-board.sh",
    mustContain: ['"${ROOT}/frontend"', '"${SLOT_DIR}/frontend"', '"${ROOT}/static"'],
    label: "board installer copies Vue assets and legacy fallback",
  },
  {
    file: "scripts/package-a55-debian10.sh",
    mustContain: [
      "Missing frontend/dist/index.html",
      'cp -r frontend/dist "${PACKAGE_DIR}/frontend/"',
      "--assets auto",
      "sudo ./install.sh",
    ],
    mustNotContain: ['--assets "${ROOT}/static"'],
    label: "ARM64 package carries Vue dist and starts auto assets",
  },
  {
    file: "scripts/run-lubancat2-qemu.sh",
    mustContain: [
      'PACKAGE_PATH}/frontend/dist/index.html',
      'ASSETS_PATH="${PACKAGE_PATH}/frontend/dist"',
      'ASSETS_PATH="${PACKAGE_PATH}/static"',
    ],
    label: "QEMU smoke prefers packaged Vue assets with legacy fallback",
  },
  {
    file: "scripts/perf-lubancat2-qemu.mjs",
    mustNotContain: ["--assets static"],
    label: "QEMU perf does not force legacy static HMI",
  },
  {
    file: "scripts/visual-click-lubancat2-qemu.mjs",
    mustNotContain: ["--assets static"],
    label: "QEMU visual does not force legacy static HMI",
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
  for (const needle of check.mustContain ?? []) {
    if (!text.includes(needle)) {
      failures.push(`${check.label}: ${check.file} must contain ${JSON.stringify(needle)}`);
    }
  }
  for (const needle of check.mustNotContain ?? []) {
    if (text.includes(needle)) {
      failures.push(`${check.label}: ${check.file} must not contain ${JSON.stringify(needle)}`);
    }
  }
}

if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("Vue release assets gate passed");
