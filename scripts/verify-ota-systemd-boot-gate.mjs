#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const backend = await readFile(path.join(root, "deploy/reactor-edge.service"), "utf8");
const kiosk = await readFile(path.join(root, "deploy/reactor-os-chromium.service"), "utf8");
const bootCheck = await readFile(path.join(root, "deploy/reactor-edge-ota-boot-check.service"), "utf8");

const failures = [];

if (!backend.includes("Requires=reactor-edge-ota-boot-check.service")) {
  failures.push("backend service must require OTA boot-check unit at boot");
}
if (!backend.includes("ExecStartPre=/opt/reactor-edge/ota-boot-check.sh")) {
  failures.push("backend service must run OTA boot-check before every service start/restart");
}
if (!backend.includes("Restart=on-failure")) {
  failures.push("backend service must restart only on failure, not after intentional maintenance stops");
}
if (!backend.includes("RestartSec=5")) {
  failures.push("backend service must delay restarts to avoid tight crash loops");
}
if (!backend.includes("StartLimitIntervalSec=600") || !backend.includes("StartLimitBurst=5")) {
  failures.push("backend service must rate-limit repeated crashes into maintenance intervention");
}
if (!backend.includes("WorkingDirectory=/opt/reactor-edge/current")) {
  failures.push("backend service must run from the active current slot");
}
if (!kiosk.includes("Requires=reactor-edge-ota-boot-check.service")) {
  failures.push("kiosk service must require OTA boot-check unit");
}
if (!kiosk.includes("WorkingDirectory=/opt/reactor-edge/current")) {
  failures.push("kiosk service must run from the active current slot");
}
if (!kiosk.includes("StartLimitIntervalSec=600") || !kiosk.includes("StartLimitBurst=5")) {
  failures.push("kiosk service must rate-limit display crash loops");
}
if (!bootCheck.includes("Type=oneshot")) {
  failures.push("OTA boot-check unit must be oneshot");
}
if (bootCheck.includes("RemainAfterExit=yes")) {
  failures.push("OTA boot-check unit must not remain active after success; backend ExecStartPre must rerun the check");
}
if (!bootCheck.includes("Before=reactor-edge.service")) {
  failures.push("OTA boot-check unit must order before backend at boot");
}
if (!bootCheck.includes("ReadWritePaths=-/opt/reactor-edge") || !bootCheck.includes("ReadWritePaths=-/var/lib/reactor-edge")) {
  failures.push("OTA boot-check unit must have write access to slot links and OTA state");
}

if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("OTA systemd boot gate passed");
