#!/usr/bin/env node

import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const DEFAULTS = {
  mode: process.env.REACTOR_OS_SIM_MODE || "pipeline",
  baseUrl: process.env.REACTOR_OS_URL || "http://127.0.0.1:8000",
  deviceId: process.env.REACTOR_OS_DEVICE_ID || "reactor_001",
  token: process.env.REACTOR_OS_TOKEN || process.env.XINGSHU_TOKEN || "",
  intervalMs: Number(process.env.REACTOR_OS_SIM_INTERVAL_MS || 1000),
  profile: process.env.REACTOR_OS_SIM_PROFILE || "normal",
  statePath:
    process.env.REACTOR_OS_SIM_STATE ||
    path.join(repoRoot, "data", "simulator", "state.json"),
  controlPath:
    process.env.REACTOR_OS_SIM_CONTROL ||
    path.join(repoRoot, "data", "simulator", "control.json"),
};

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  printHelp();
  process.exit(0);
}

const config = {
  mode: normalizeMode(args.mode ?? DEFAULTS.mode),
  baseUrl: String(args.url ?? DEFAULTS.baseUrl).replace(/\/+$/, ""),
  deviceId: String(args["device-id"] ?? DEFAULTS.deviceId),
  token: String(args.token ?? DEFAULTS.token).trim(),
  intervalMs: positiveInt(args["interval-ms"] ?? DEFAULTS.intervalMs, "interval-ms"),
  profile: String(args.profile ?? DEFAULTS.profile),
  statePath: path.resolve(String(args.state ?? DEFAULTS.statePath)),
  controlPath: path.resolve(String(args.control ?? DEFAULTS.controlPath)),
  once: Boolean(args.once),
  quiet: Boolean(args.quiet),
  strict: Boolean(args.strict),
};

const device = createDeviceState(config.profile);
let targetCache = {
  temperature_c: 60,
  stirrer_rpm: 300,
  shake_speed_cpm: 30,
  target_pressure_mpa: 0.5,
};
let lastControlRequestId = null;
let stopped = false;

process.on("SIGINT", () => {
  stopped = true;
  log("received Ctrl+C, stopping simulator");
});
process.on("SIGTERM", () => {
  stopped = true;
  log("received SIGTERM, stopping simulator");
});

await main();

async function main() {
  log(
    [
      "ReactorOS local downstream simulator",
      `mode=${config.mode}`,
      `profile=${config.profile}`,
      `interval=${config.intervalMs}ms`,
    ].join(" | "),
  );

  if (config.mode === "json-bridge" || config.mode === "both") {
    log(`state.json -> ${config.statePath}`);
    log(`control.json <- ${config.controlPath}`);
  }
  if (config.mode === "pipeline" || config.mode === "both") {
    if (!config.token) {
      throw new Error(
        "pipeline mode requires --token, REACTOR_OS_TOKEN, or XINGSHU_TOKEN with ingest_sensor_sample permission",
      );
    }
    log(`pipeline POST -> ${sampleUrl()}`);
  }

  do {
    await tick();
    if (config.once) break;
    await sleep(config.intervalMs);
  } while (!stopped);
}

async function tick() {
  if (config.mode === "pipeline" || config.mode === "both") {
    await refreshTargetsFromBackend();
  }

  if (config.mode === "json-bridge" || config.mode === "both") {
    await consumeJsonBridgeControl();
  }

  evolveDevice();
  const sample = currentSample();

  if (config.mode === "json-bridge" || config.mode === "both") {
    await writeJsonBridgeState(sample);
  }
  if (config.mode === "pipeline" || config.mode === "both") {
    await postPipelineSample(sample);
  }
}

function createDeviceState(profile) {
  const base = {
    startedAt: Date.now(),
    temperature_c: 31.11,
    pressure_mpa: 0.5,
    stirrer_rpm: 125.18,
    shake_speed_cpm: 18,
    target_shake_speed_cpm: 30,
    flow_rate_l_min: 1.05,
    product_concentration_percent: 11.1,
    ph: 6.15,
    relay: 0,
    motor: 1,
    tilt_state: 0,
    last_command: null,
    last_command_ok: null,
    last_command_error: null,
    last_command_request_id: null,
    last_command_sent_ms: null,
  };

  if (profile === "warmup") {
    base.temperature_c = 45.25;
    base.stirrer_rpm = 240.5;
    base.product_concentration_percent = 18.4;
  } else if (profile === "production") {
    base.temperature_c = 72.4;
    base.stirrer_rpm = 420.0;
    base.shake_speed_cpm = 30.0;
    base.target_shake_speed_cpm = 32.0;
    base.product_concentration_percent = 48.0;
  } else if (profile === "alarm") {
    base.temperature_c = 152.0;
    base.pressure_mpa = 0.78;
    base.stirrer_rpm = 980.0;
    base.product_concentration_percent = 72.0;
  }
  return base;
}

function evolveDevice() {
  const elapsedS = (Date.now() - device.startedAt) / 1000;
  const profileBias = config.profile === "alarm" ? 1 : 0;
  const desiredTemp =
    device.relay === 1
      ? Math.max(targetCache.temperature_c, 85)
      : targetCache.temperature_c;
  const desiredStirrer = targetCache.stirrer_rpm;
  const desiredShake =
    device.motor === 1 ? Math.max(0, device.target_shake_speed_cpm) : 0;
  const desiredPressure =
    targetCache.target_pressure_mpa +
    Math.max(0, device.temperature_c - 35) * 0.0018 +
    device.stirrer_rpm * 0.00003;

  device.temperature_c = approach(
    device.temperature_c,
    desiredTemp + profileBias * 14 + Math.sin(elapsedS / 18) * 0.35,
    0.55,
  );
  device.pressure_mpa = approach(
    device.pressure_mpa,
    desiredPressure + profileBias * 0.08,
    0.015,
  );
  device.stirrer_rpm = approach(
    device.stirrer_rpm,
    desiredStirrer + Math.sin(elapsedS / 9) * 8,
    18,
  );
  device.shake_speed_cpm = approach(device.shake_speed_cpm, desiredShake, 2.5);
  device.flow_rate_l_min = approach(
    device.flow_rate_l_min,
    device.motor === 1 ? 1.15 + device.shake_speed_cpm * 0.018 : 0.2,
    0.04,
  );

  const qualityWindow =
    between(device.temperature_c, 62, 98) &&
    between(device.pressure_mpa, 0.42, 0.72) &&
    between(device.stirrer_rpm, 260, 650);
  const concentrationRise = qualityWindow ? 0.035 : 0.008;
  device.product_concentration_percent = clamp(
    device.product_concentration_percent + concentrationRise,
    0,
    98.5,
  );
  device.ph = approach(
    device.ph,
    6.35 - device.product_concentration_percent * 0.004 + Math.sin(elapsedS / 25) * 0.05,
    0.01,
  );

  if (device.shake_speed_cpm <= 0.01) {
    device.tilt_state = 0;
  } else {
    const periodMs = 60_000 / device.shake_speed_cpm;
    device.tilt_state = Math.floor((Date.now() % periodMs) / (periodMs / 2)) % 2;
  }
}

function currentSample() {
  return {
    temperature_c: round2(device.temperature_c),
    pressure_mpa: round2(device.pressure_mpa),
    stirrer_rpm: round2(device.stirrer_rpm),
    shake_speed_cpm: round2(device.shake_speed_cpm),
    tilt_state: device.tilt_state,
    flow_rate_l_min: round2(device.flow_rate_l_min),
    product_concentration_percent: round2(device.product_concentration_percent),
    ph: round2(device.ph),
  };
}

async function refreshTargetsFromBackend() {
  try {
    const response = await fetch(`${config.baseUrl}/api/live`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) return;
    const live = await response.json();
    const targets = live?.runtime?.targets;
    if (!targets) return;
    targetCache = {
      temperature_c: finiteOr(targets.temperature_c, targetCache.temperature_c),
      stirrer_rpm: finiteOr(targets.stirrer_rpm, targetCache.stirrer_rpm),
      shake_speed_cpm: finiteOr(targets.shake_speed_cpm, targetCache.shake_speed_cpm),
      target_pressure_mpa: finiteOr(
        targets.target_pressure_mpa,
        targetCache.target_pressure_mpa,
      ),
    };
    device.target_shake_speed_cpm = targetCache.shake_speed_cpm;
  } catch {
    // The first samples may be sent before the backend is ready. Keep simulating.
  }
}

async function postPipelineSample(sample) {
  try {
    const response = await fetch(sampleUrl(), {
      method: "POST",
      headers: {
        authorization: `Bearer ${config.token}`,
        "content-type": "application/json",
        accept: "application/json",
      },
      body: JSON.stringify(sample),
    });
    const text = await response.text();
    if (!response.ok) {
      const message = `pipeline sample rejected: HTTP ${response.status} ${text}`;
      if (config.strict) throw new Error(message);
      log(message);
      return;
    }
    log(
      `posted sample HTTP ${response.status} | temp=${fmt(sample.temperature_c)}C pressure=${fmt(
        sample.pressure_mpa,
      )}MPa rpm=${fmt(sample.stirrer_rpm)} shake=${fmt(
        sample.shake_speed_cpm,
      )}cpm tilt=${sample.tilt_state}`,
    );
  } catch (error) {
    if (config.strict) throw error;
    log(`pipeline post failed: ${error.message}`);
  }
}

async function consumeJsonBridgeControl() {
  let raw;
  try {
    raw = await readFile(config.controlPath, "utf8");
  } catch {
    return;
  }

  let control;
  try {
    control = JSON.parse(raw.replace(/^\uFEFF/, ""));
  } catch (error) {
    applyControlError(null, `invalid control.json: ${error.message}`);
    return;
  }

  if (!control?.request_id || control.request_id === lastControlRequestId) return;
  lastControlRequestId = control.request_id;

  try {
    applyControl(control);
    device.last_command = `${control.command}:${JSON.stringify(control.value ?? null)}`;
    device.last_command_request_id = control.request_id;
    device.last_command_sent_ms = Date.now();
    device.last_command_ok = true;
    device.last_command_error = null;
    log(`applied control ${device.last_command} request_id=${control.request_id}`);
  } catch (error) {
    applyControlError(control.request_id, error.message);
    log(`control rejected request_id=${control.request_id}: ${error.message}`);
  }
}

function applyControl(control) {
  const command = String(control.command || "");
  const value = control.value;
  if (command === "motor") {
    device.motor = value ? 1 : 0;
    if (device.motor === 1 && device.target_shake_speed_cpm <= 0.01) {
      device.target_shake_speed_cpm = 30;
    }
    return;
  }
  if (command === "speed") {
    if (value === "up") {
      device.target_shake_speed_cpm = clamp(device.target_shake_speed_cpm + 5, 0, 60);
      device.motor = device.target_shake_speed_cpm > 0 ? 1 : 0;
      return;
    }
    if (value === "down") {
      device.target_shake_speed_cpm = clamp(device.target_shake_speed_cpm - 5, 0, 60);
      device.motor = device.target_shake_speed_cpm > 0 ? 1 : 0;
      return;
    }
  }
  if (command === "relay") {
    device.relay = value ? 1 : 0;
    return;
  }
  if (command === "stir_speed") {
    const rpm = Number(value);
    if (!Number.isFinite(rpm)) {
      throw new Error("stir_speed command value must be a finite number");
    }
    targetCache.stirrer_rpm = clamp(rpm, 0, 2000);
    return;
  }
  throw new Error(`unsupported command ${command}`);
}

function applyControlError(requestId, message) {
  device.last_command_request_id = requestId;
  device.last_command_sent_ms = Date.now();
  device.last_command_ok = false;
  device.last_command_error = message;
}

async function writeJsonBridgeState(sample) {
  const status = (device.relay ? 1 : 0) | (device.motor ? 2 : 0) | (sample.tilt_state ? 4 : 0);
  const state = {
    connected: true,
    last_seen_ms: Date.now(),
    last_frame_hex: "SIMULATED_LOCAL_DEVICE",
    last_frame_ok: true,
    adc: adcFromConcentration(sample.product_concentration_percent),
    status,
    relay: device.relay,
    motor: device.motor,
    tilt: sample.tilt_state,
    speed_delay_us: speedDelayUs(sample.shake_speed_cpm),
    last_command: device.last_command,
    last_command_request_id: device.last_command_request_id,
    last_command_sent_ms: device.last_command_sent_ms,
    last_command_ok: device.last_command_ok,
    last_command_error: device.last_command_error,
    port: "local-simulator",
    baudrate: 115200,
    bridge_started_ms: device.startedAt,
    temperature_c: sample.temperature_c,
    pressure_mpa: sample.pressure_mpa,
    stirrer_rpm: sample.stirrer_rpm,
    shake_speed_cpm: sample.shake_speed_cpm,
    flow_rate_l_min: sample.flow_rate_l_min,
    product_concentration_percent: sample.product_concentration_percent,
    ph: sample.ph,
  };
  await atomicWriteJson(config.statePath, state);
  log(
    `wrote state.json | temp=${fmt(sample.temperature_c)}C pressure=${fmt(
      sample.pressure_mpa,
    )}MPa rpm=${fmt(sample.stirrer_rpm)} motor=${device.motor} relay=${device.relay}`,
  );
}

async function atomicWriteJson(targetPath, value) {
  await mkdir(path.dirname(targetPath), { recursive: true });
  const tmp = `${targetPath}.${process.pid}.tmp`;
  await writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  await rename(tmp, targetPath);
}

function sampleUrl() {
  return `${config.baseUrl}/api/v1/reactor/${encodeURIComponent(config.deviceId)}/samples`;
}

function parseArgs(items) {
  const parsed = {};
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    if (item === "--help" || item === "-h") {
      parsed.help = true;
    } else if (item === "--once" || item === "--quiet" || item === "--strict") {
      parsed[item.slice(2)] = true;
    } else if (item.startsWith("--")) {
      const [key, inlineValue] = item.slice(2).split("=", 2);
      if (inlineValue !== undefined) {
        parsed[key] = inlineValue;
      } else {
        const next = items[i + 1];
        if (!next || next.startsWith("--")) {
          throw new Error(`missing value for --${key}`);
        }
        parsed[key] = next;
        i += 1;
      }
    } else {
      throw new Error(`unknown argument ${item}`);
    }
  }
  return parsed;
}

function normalizeMode(mode) {
  const value = String(mode);
  if (["pipeline", "json-bridge", "both"].includes(value)) return value;
  throw new Error(`unsupported mode ${value}; expected pipeline, json-bridge, or both`);
}

function positiveInt(value, label) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${label} must be a positive integer`);
  }
  return parsed;
}

function approach(current, target, maxStep) {
  const delta = target - current;
  if (Math.abs(delta) <= maxStep) return target;
  return current + Math.sign(delta) * maxStep;
}

function between(value, min, max) {
  return value >= min && value <= max;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function finiteOr(value, fallback) {
  return Number.isFinite(Number(value)) ? Number(value) : fallback;
}

function round2(value) {
  return Math.round(Number(value) * 100) / 100;
}

function fmt(value) {
  return round2(value).toFixed(2);
}

function adcFromConcentration(concentration) {
  return Math.round(clamp(concentration, 0, 100) / 0.0244200244);
}

function speedDelayUs(cpm) {
  if (cpm <= 0.01) return null;
  return Math.round(60_000_000 / (cpm * 200));
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function log(message) {
  if (!config.quiet) {
    console.log(`[sim-device] ${new Date().toISOString()} ${message}`);
  }
}

function printHelp() {
  console.log(`ReactorOS local downstream device simulator

Usage:
  node scripts/simulate-device.js [options]

Options:
  --mode pipeline|json-bridge|both   Output mode. Default: pipeline
  --url http://127.0.0.1:8000        ReactorOS base URL for pipeline mode
  --device-id reactor_001            Device id for pipeline mode
  --token TOKEN                       Bearer token with ingest_sensor_sample permission
  --state path/to/state.json         JSON bridge state output path
  --control path/to/control.json     JSON bridge control input path
  --interval-ms 1000                 Sample interval
  --profile normal|warmup|production|alarm
  --once                             Emit one sample/frame then exit
  --strict                           Exit on pipeline rejection/failure
  --quiet                            Reduce logs

Environment:
  REACTOR_OS_SIM_MODE
  REACTOR_OS_URL
  REACTOR_OS_DEVICE_ID
  REACTOR_OS_TOKEN / XINGSHU_TOKEN
  REACTOR_OS_SIM_INTERVAL_MS
  REACTOR_OS_SIM_PROFILE
  REACTOR_OS_SIM_STATE
  REACTOR_OS_SIM_CONTROL
`);
}
