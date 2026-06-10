// AINAS / MQTT integration readiness self-check against a running daemon.
// Reports whether the integration metadata is enabled, the AINAS REST task
// endpoint accepts new tasks, and the MQTT bridge status is reachable.
//
// Usage:
//   E2E_BASE_URL=http://127.0.0.1:8000 node scripts/verify-ainas-mqtt.mjs
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const API = process.env.E2E_BASE_URL || "http://127.0.0.1:8000";
const OUT = resolve(process.cwd(), "output/local-run");
mkdirSync(OUT, { recursive: true });

const result = {
  apiBase: API,
  steps: []
};

function log(step, status, info) {
  result.steps.push({ step, status, info: info ?? null });
  // eslint-disable-next-line no-console
  console.log(`[${status}] ${step}${info ? " :: " + info : ""}`);
}

async function jsonRequest(path, init = {}) {
  const response = await fetch(`${API}${path}`, {
    method: init.method ?? "GET",
    headers: {
      Accept: "application/json",
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...(init.token ? { Authorization: `Bearer ${init.token}` } : {})
    },
    body: init.body ? JSON.stringify(init.body) : undefined
  });
  const text = await response.text();
  let body = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
  }
  return { status: response.status, body };
}

async function seedFreshSample(token) {
  const payload = {
    temperature_c: 60.2,
    pressure_mpa: 0.55,
    stirrer_rpm: 300,
    shake_speed_cpm: 0,
    tilt_state: 0,
    flow_rate_l_min: 2.2,
    product_concentration_percent: 12.4,
    ph: 6.8
  };
  const response = await jsonRequest("/api/v1/reactor/reactor_001/samples", {
    method: "POST",
    token,
    body: payload
  });
  if (response.status !== 200) {
    throw new Error(`sample seed failed: ${response.status} ${JSON.stringify(response.body).slice(0, 200)}`);
  }
  return payload;
}

(async () => {
  try {
    // 1. Login as engineer.
    const login = await jsonRequest("/api/auth/login", {
      method: "POST",
      body: { username: "engineer", password: "engineer123" }
    });
    if (login.status !== 200) throw new Error(`login failed: ${login.status}`);
    const token = login.body?.data?.token ?? login.body?.token;
    if (!token) throw new Error("login returned no token");
    log("login-engineer", "ok");

    // 2. /api/config/summary integration section.
    const summary = await jsonRequest("/api/config/summary", { token });
    const integrations = summary.body?.data?.integrations ?? {};
    const flags = {
      ainas_ready: !!integrations.ainas_ready,
      ainas_task_api: !!integrations.ainas_task_api,
      cli: !!integrations.cli,
      json_bridge: !!integrations.json_bridge,
      modbus_rtu: !!integrations.modbus_rtu,
      modbus_tcp: !!integrations.modbus_tcp,
      mqtt: !!integrations.mqtt,
      rest_api: !!integrations.rest_api
    };
    log("config-summary", "ok", JSON.stringify(flags));

    // 3. Seed a fresh sample before any target mutation path. Industrial fail-closed
    // rules reject target changes unless the field state is currently proven.
    const seeded = await seedFreshSample(token);
    log("seed-live-sample", "ok", JSON.stringify({ temperature_c: seeded.temperature_c, pressure_mpa: seeded.pressure_mpa }));

    // 4. AINAS task dispatch path: GET list and POST a new task, then GET it back.
    const list = await jsonRequest("/api/integrations/ainas/tasks?limit=1", { token });
    if (list.status !== 200) throw new Error(`ainas list failed: ${list.status}`);
    const create = await jsonRequest("/api/integrations/ainas/tasks", {
      method: "POST",
      token,
      body: {
        external_task_id: `selfcheck-${Date.now()}`,
        action: "set_targets",
        target_temperature_c: 60,
        target_stirrer_rpm: 300,
        target_shake_speed_cpm: 0,
        reason: "ainas-mqtt selfcheck"
      }
    });
    if (create.status !== 200 && create.status !== 201) {
      throw new Error(`ainas create failed: ${create.status} ${JSON.stringify(create.body).slice(0, 200)}`);
    }
    const created = create.body?.data ?? create.body;
    const taskId = created?.id ?? created?.task?.id;
    if (!taskId) throw new Error("ainas create returned no task id");
    log("ainas-create", "ok", JSON.stringify({ id: taskId, status: create.status }));

    const detail = await jsonRequest(`/api/integrations/ainas/tasks/${taskId}`, { token });
    if (detail.status !== 200) throw new Error(`ainas detail failed: ${detail.status}`);
    log("ainas-detail", "ok", `task_id=${taskId}`);

    // 5. MQTT status (read from integration config).
    const mqttStatus = integrations.mqtt_status ?? {};
    log("mqtt-status", mqttStatus.connected ? "ok" : "info", JSON.stringify({
      enabled: !!integrations.mqtt,
      connected: !!mqttStatus.connected,
      broker: mqttStatus.broker ?? null,
      tls: mqttStatus.use_tls ?? null,
      last_error: mqttStatus.last_error ?? null
    }));

    // 6. /api/live realtime check through the formal pipeline sample ingress.
    const live = await jsonRequest("/api/live?sample_limit=1");
    const liveOk = live.status === 200;
    log("live-realtime", liveOk ? "ok" : "fail", `status=${live.status}`);

    result.ok = flags.ainas_ready && flags.ainas_task_api && flags.rest_api && liveOk;
  } catch (error) {
    log("error", "fail", error instanceof Error ? error.message : String(error));
    result.ok = false;
  }
  const outPath = resolve(OUT, "ainas-mqtt-selfcheck.json");
  writeFileSync(outPath, JSON.stringify(result, null, 2));
  // eslint-disable-next-line no-console
  console.log(`selfcheck -> ${outPath}`);
  if (!result.ok) process.exit(1);
})();
