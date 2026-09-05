#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { brotliDecompressSync, gunzipSync } from "node:zlib";

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
      'FRONTEND_DIST="${FRONTEND_DIST:-frontend/dist}"',
      'cp -r "${FRONTEND_DIST}" "${PACKAGE_DIR}/frontend/"',
      "--assets auto",
      "sudo ./install.sh",
    ],
    mustNotContain: ['--assets "${ROOT}/static"'],
    label: "ARM64 package carries Vue dist and starts auto assets",
  },
  {
    file: "scripts/build-lubancat2-debian10.ps1",
    mustContain: [
      "npm run frontend:build",
      '"-e", "FRONTEND_DIST=frontend/dist"',
    ],
    label: "LubanCat Windows build selects Vue HMI",
  },
  {
    file: "scripts/build-lubancat2-debian10.sh",
    mustContain: [
      "npm run frontend:build",
      '-e "FRONTEND_DIST=frontend/dist"',
    ],
    label: "LubanCat Unix build selects Vue HMI",
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
    file: "Dockerfile",
    mustContain: [
      "FROM node:24-bookworm-slim AS frontend-builder",
      "RUN npm run frontend:build",
      "COPY --from=frontend-builder /src/frontend/dist ./frontend/dist",
      '"--assets", "/app/frontend/dist"',
    ],
    mustNotContain: ["COPY static ./static", '"--assets", "/app/static"'],
    label: "Docker runtime carries and serves the Vue HMI",
  },
  {
    file: "docker-compose.yml",
    mustContain: ["- /app/frontend/dist"],
    mustNotContain: ["- /app/static"],
    label: "Docker Compose serves the Vue HMI",
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
await verifyCompressedHmi(failures);
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

async function verifyCompressedHmi(output) {
  const dist = path.join(root, "frontend", "dist");
  const htmlPath = path.join(dist, "index.html");
  const brPath = `${htmlPath}.br`;
  const gzipPath = `${htmlPath}.gz`;
  try {
    const [html, brotli, gzipped] = await Promise.all([
      readFile(htmlPath),
      readFile(brPath),
      readFile(gzipPath),
    ]);
    if (!brotliDecompressSync(brotli).equals(html)) {
      output.push("Vue compressed HMI: index.html.br does not decompress to index.html");
    }
    if (!gunzipSync(gzipped).equals(html)) {
      output.push("Vue compressed HMI: index.html.gz does not decompress to index.html");
    }
    if (brotli.length >= html.length * 0.8) {
      output.push(`Vue compressed HMI: Brotli output is unexpectedly large (${brotli.length}/${html.length})`);
    }
    if (gzipped.length >= html.length * 0.8) {
      output.push(`Vue compressed HMI: gzip output is unexpectedly large (${gzipped.length}/${html.length})`);
    }
  } catch (error) {
    output.push(`Vue compressed HMI: cannot verify index.html.br/index.html.gz: ${error.message}`);
  }
}
