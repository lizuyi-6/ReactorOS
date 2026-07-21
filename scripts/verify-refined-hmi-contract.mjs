#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

async function read(relativePath) {
  return readFile(path.join(root, relativePath), "utf8");
}

const [app, styles, refined, monitor, dist] = await Promise.all([
  read("frontend/src/App.vue"),
  read("frontend/src/styles.css"),
  read("frontend/src/styles/refined-industrial.css"),
  read("frontend/src/views/MonitorView.vue"),
  read("frontend/dist/index.html"),
]);

const checks = [
  [
    "refined theme must be the final stylesheet layer",
    styles.trimEnd().endsWith('@import "./styles/refined-industrial.css";'),
  ],
  ["Settings endpoint matrix must have a seventh pager screen", app.includes('"/settings": 7')],
  [
    "Settings page 7 must expose direct section 11",
    refined.includes("hmi-page-6.route-settings > .view-stack > section:nth-of-type(11)"),
  ],
  [
    "narrow screens must explicitly keep safety and command tags visible",
    refined.includes(".status-cluster .el-tag:nth-child(5)") &&
      refined.includes(".status-cluster .el-tag:nth-child(6)") &&
      refined.includes("display: inline-flex !important"),
  ],
  [
    "mobile shell must retain a second safety-status row",
    refined.includes("grid-template-rows: 56px 40px") && refined.includes("grid-row: 2"),
  ],
  [
    "global backend errors must remain visible",
    refined.includes(".content.hmi-fixed > .error-alert") && refined.includes("display: flex !important"),
  ],
  [
    "disabled controls must look disabled",
    refined.includes(".el-button.is-disabled") && refined.includes("cursor: not-allowed !important"),
  ],
  [
    "control-loop termination must read the boolean runtime field",
    monitor.includes('textAt(runtime.value, "control_loop_terminated", "false") === "true"'),
  ],
  ["zero alarms must remain zero", !monitor.includes("alarms.length || 1")],
  ["unknown detector state must not be hard-coded NORMAL", !monitor.includes("<em>NORMAL</em>")],
  [
    "decorative FFT and U-value charts must not masquerade as telemetry",
    !monitor.includes("Vibration FFT & U Value") && !monitor.includes("U-VALUE"),
  ],
  [
    "monitor action affordances must navigate to real review pages",
    monitor.includes('<RouterLink class="apply-ai-button" to="/ai">') &&
      monitor.includes('<RouterLink to="/history">') &&
      monitor.includes('<RouterLink to="/control">'),
  ],
  ["production artifact must remain a single Vue mount page", dist.includes('id="app"')],
  [
    "production artifact must contain the refined monitor copy",
    dist.includes("Review recommendation") && dist.includes("Backend values only"),
  ],
  [
    "production artifact must not retain dead or fabricated monitor copy",
    !dist.includes("APPLY AI PARAMS") && !dist.includes("Vibration FFT & U Value"),
  ],
];

const failures = checks.filter(([, passed]) => !passed).map(([label]) => label);
if (failures.length > 0) {
  console.error(failures.map((label) => `FAIL: ${label}`).join("\n"));
  process.exit(1);
}

console.log(`Refined HMI contract passed: ${checks.length} assertions`);
