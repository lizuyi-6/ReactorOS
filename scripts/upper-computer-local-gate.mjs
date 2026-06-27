import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  const arg = process.argv[index];
  if (!arg.startsWith("--")) continue;
  const key = arg.slice(2);
  const next = process.argv[index + 1];
  if (!next || next.startsWith("--")) {
    args.set(key, "true");
  } else {
    args.set(key, next);
    index += 1;
  }
}

const baseUrl = normalizeBaseUrl(
  args.get("url") || process.env.XINGSHU_GATE_URL || "http://127.0.0.1:8000",
);
const outputDir = path.resolve(args.get("out-dir") || "output");
const stamp = localDateStamp();
const reportPath = path.join(outputDir, `upper-computer-local-gate-${stamp}.json`);
const visualReportPath = path.resolve(
  args.get("visual-report") ||
    path.join("output", "visual-i18n", "upper-computer-i18n-audit-20260605.json"),
);

const requiredDocs = [
  "docs/upper_computer_development_doc.md",
  "docs/upper_computer_user_manual.md",
  "docs/upper_computer_test_report.md",
  "docs/upper_computer_gap_status.md",
  "docs/upper_computer_requirement_gap_matrix.md",
  "docs/upper_computer_api_acceptance_manual.md",
  "docs/upper_computer_cli_reference.md",
  "docs/upper_computer_maintenance_manual.md",
  "docs/upper_computer_modbus_register_map.md",
  "docs/upper_computer_rk_deployment_acceptance_guide.md",
  "docs/upper_computer_security_key_lifecycle.md",
  "docs/upper_computer_external_acceptance_checklist.md",
  "docs/upper_computer_visual_evidence_index.md",
  "docs/upper_computer_delivery_readiness_index.md",
  "docs/upper_computer_training_material_plan.md",
  "docs/architecture-deviations.md",
  "docs/third_party_interface_acceptance_report.md",
];

const checks = [];

await runCheck("health endpoint", async () => {
  const payload = await apiJson("/health");
  assert(payload.ok === true, "health.ok must be true");
  assert(payload.service === "reactor-edge-daemon", "unexpected service name");
  return payload;
});

await runCheck("web hmi shell and i18n controls", async () => {
  const html = await httpText("/");
  const vueMarkers = ['id="app"', "ReactorOS HMI", "reactoros.vue.language", "Vue 3"];
  const vueReadinessMarkers = ["Integration Surface", "Base inference", "PRD LoRA/RK"];
  const legacyMarkers = [
    'id="langToggleBtn"',
    'data-tab="monitor"',
    'data-tab="modbus"',
    "reactoros.lang",
    "function uiText",
  ];
  const isVueShell = vueMarkers.every((marker) => html.includes(marker));
  const isLegacyShell = legacyMarkers.every((marker) => html.includes(marker));
  assert(isVueShell || isLegacyShell, "missing Vue release or legacy HMI shell markers");
  if (isVueShell) {
    for (const marker of vueReadinessMarkers) {
      assert(html.includes(marker), `missing Vue readiness marker: ${marker}`);
    }
  }
  return {
    bytes: html.length,
    shell: isVueShell ? "vue-release" : "legacy-static",
    markers: isVueShell ? [...vueMarkers, ...vueReadinessMarkers] : legacyMarkers,
  };
});

await runCheck("local bearer login and role permissions", async () => {
  const users = {};
  for (const role of ["operator", "engineer", "admin"]) {
    const login = await apiJson("/api/auth/login", {
      method: "POST",
      body: { username: role, password: `${role}123` },
    });
    assert(login.code === 0, `${role} login did not return success envelope`);
    const token = login.data?.token;
    assert(token, `${role} login did not return token`);
    const me = await apiJson("/api/auth/me", { token });
    assert(me.data?.role === role, `${role} /api/auth/me returned wrong role`);
    users[role] = {
      role: me.data.role,
      permissions: me.data.permissions,
    };
  }
  assert(
    users.operator.permissions.includes("set_safe_targets"),
    "operator should keep normal safe target permission",
  );
  assert(
    !users.operator.permissions.includes("modbus_debug"),
    "operator must not have modbus debug permission",
  );
  assert(
    users.engineer.permissions.includes("modbus_debug"),
    "engineer should have read/debug visibility",
  );
  assert(users.admin.permissions.includes("manage_users"), "admin must have manage_users");
  return users;
});

await runCheck("config summary delivery surface", async () => {
  const summary = await apiJson("/api/config/summary");
  const data = summary.data;
  assert(data?.device_mode, "config summary missing device_mode");
  assert(data?.permissions?.authentication === "bearer_session_enforced", "RBAC summary mismatch");
  assert(data?.data_security?.storage_encryption?.algorithm === "AES-256-GCM", "AES summary missing");
  assert(data?.integrations?.rest_api === true, "REST API readiness missing");
  assert(data?.integrations?.cli === true, "CLI readiness missing");
  assert(data?.integrations?.ainas_task_api === true, "AINAS task readiness missing");
  assert(data?.local_ai && typeof data.local_ai.ready_for_base_inference === "boolean", "local AI base inference boundary missing");
  assert(typeof data.local_ai.ready_for_lora_inference === "boolean", "local AI LoRA inference boundary missing");
  assert(
    data.local_ai.ready_for_inference === data.local_ai.ready_for_lora_inference,
    "local_ai.ready_for_inference must remain the compatibility alias for LoRA inference readiness",
  );
  assert(typeof data.local_ai.ready_for_training === "boolean", "local AI training boundary missing");
  assert(typeof data.local_ai.ready_for_prd_lora === "boolean", "local AI PRD LoRA/RK boundary missing");
  return {
    device_mode: data.device_mode,
    storage_encryption: data.data_security.storage_encryption,
    integrations: data.integrations,
    local_ai: data.local_ai,
  };
});

await runCheck("modbus map read/write shape", async () => {
  const registers = await apiJson("/api/modbus/registers");
  const data = registers.data;
  assert(data?.device_id === "reactor_001", "wrong Modbus device_id");
  assert(Array.isArray(data.read_registers) && data.read_registers.length >= 8, "missing read registers");
  assert(Array.isArray(data.write_registers) && data.write_registers.length >= 7, "missing write registers");
  assert(Array.isArray(data.coils) && data.coils.length >= 4, "missing coils");
  assert(Array.isArray(data.discrete_inputs) && data.discrete_inputs.length >= 2, "missing discrete inputs");
  return {
    read_registers: data.read_registers.length,
    write_registers: data.write_registers.length,
    coils: data.coils.length,
    discrete_inputs: data.discrete_inputs.length,
  };
});

await runCheck("visual i18n audit evidence", async () => {
  const raw = await readFile(visualReportPath, "utf8");
  const report = JSON.parse(raw);
  const summary = report.summary || {};
  assert(summary.pageCount >= 18, "visual audit did not cover zh/en tabs");
  assert(
    (summary.englishPagesWithUnexpectedCjk || []).length === 0,
    "English visual audit found unexpected Chinese text",
  );
  assert((summary.pagesWithMojibake || []).length === 0, "visual audit found mojibake blocks");
  assert((summary.pagesWithEmptyViewText || []).length === 0, "visual audit found empty views");
  assert(
    (summary.unexpectedConsoleMessages || []).length === 0,
    "visual audit found unexpected console messages",
  );
  return {
    report: path.relative(process.cwd(), visualReportPath),
    pageCount: summary.pageCount,
    expectedDataPipeline503Count: summary.expectedDataPipeline503Count || 0,
  };
});

await runCheck("required upper-computer delivery documents", async () => {
  const files = [];
  for (const file of requiredDocs) {
    const absolute = path.resolve(file);
    const metadata = await stat(absolute);
    assert(metadata.size > 500, `${file} is unexpectedly small`);
    files.push({ file, bytes: metadata.size });
  }
  return { count: files.length, files };
});

await mkdir(outputDir, { recursive: true });
const failed = checks.filter((check) => check.status !== "passed");
const report = {
  generated_at: new Date().toISOString(),
  generated_at_local: localDateTime(),
  time_zone: "Asia/Shanghai",
  baseUrl,
  reportPath,
  summary: {
    passed: checks.length - failed.length,
    failed: failed.length,
    status: failed.length === 0 ? "passed" : "failed",
  },
  checks,
};
await writeFile(reportPath, JSON.stringify(report, null, 2), "utf8");
console.log(JSON.stringify(report.summary, null, 2));
console.log(`local gate report: ${reportPath}`);

if (failed.length) {
  process.exitCode = 1;
}

async function runCheck(name, fn) {
  const started = Date.now();
  try {
    const detail = await fn();
    checks.push({
      name,
      status: "passed",
      elapsed_ms: Date.now() - started,
      detail,
    });
  } catch (error) {
    checks.push({
      name,
      status: "failed",
      elapsed_ms: Date.now() - started,
      error: error.message,
    });
  }
}

async function apiJson(urlPath, options = {}) {
  const text = await httpText(urlPath, options);
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`${urlPath} did not return JSON: ${error.message}`);
  }
}

async function httpText(urlPath, options = {}) {
  const headers = { ...(options.headers || {}) };
  let body;
  if (options.body !== undefined) {
    headers["content-type"] = headers["content-type"] || "application/json";
    body = JSON.stringify(options.body);
  }
  if (options.token) {
    headers.authorization = `Bearer ${options.token}`;
  }
  const response = await fetch(new URL(urlPath, baseUrl), {
    method: options.method || "GET",
    headers,
    body,
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`${urlPath} returned ${response.status}: ${text.slice(0, 300)}`);
  }
  return text;
}

function normalizeBaseUrl(value) {
  return value.endsWith("/") ? value : `${value}/`;
}

function localDateStamp() {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(new Date());
  const values = Object.fromEntries(parts.map((part) => [part.type, part.value]));
  return `${values.year}${values.month}${values.day}`;
}

function localDateTime() {
  return new Intl.DateTimeFormat("zh-CN", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date());
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
