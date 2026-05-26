#!/usr/bin/env node

import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "@playwright/test";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = path.join(repoRoot, "output", "playwright");
const baseUrl = process.env.REACTOR_OS_QEMU_URL || "http://127.0.0.1:18080";
const bind = new URL(baseUrl).host;
const logPath = path.join(outputDir, "lubancat2-qemu-visual.log");
const screenshotPath = path.join(outputDir, "lubancat2-qemu-visual.png");

await mkdir(outputDir, { recursive: true });
await writeFile(logPath, "", "utf8");

const child = spawn(
  "wsl.exe",
  [
    "-e",
    "bash",
    "-lc",
    `cd /mnt/x/tianhks && exec ./scripts/run-lubancat2-qemu.sh --bind ${bind} --assets static`,
  ],
  {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  },
);

const appendLog = (chunk) => {
  const text = chunk.toString();
  process.stdout.write(text);
  writeFile(logPath, text, { flag: "a" }).catch(() => {});
};

child.stdout.on("data", appendLog);
child.stderr.on("data", appendLog);

let browser;
try {
  await waitForHttp(`${baseUrl}/health`, 45_000);
  const live = await fetchJson(`${baseUrl}/api/live`);
  assert(live.runtime?.latest_sample, "api/live returned no latest_sample");
  assert(
    live.runtime.latest_sample.temperature_c !== null,
    "api/live latest_sample temperature is null",
  );

  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() !== "error") return;
    const text = message.text();
    if (text.includes("Failed to load resource") && text.includes("503")) return;
    if (text.includes("Failed to load resource") && text.includes("ERR_SOCKET_NOT_CONNECTED")) return;
    consoleErrors.push(text);
  });
  page.on("pageerror", (error) => consoleErrors.push(error.message));

  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => window.__reactorState?.dataReady === true, {
    timeout: 15_000,
  });
  await page.screenshot({ path: screenshotPath, fullPage: true });

  await expectText(page, "body", "ReactorOS HMI");
  await expectText(page, "#pipelineState", "Devices 1/1");
  await expectText(page, "body", "Detector Signals");
  await expectText(page, "body", "TEMP");
  await expectText(page, "body", "RPM");
  await assertCanvasInk(page, "#mainChart");

  await clickTab(page, 'button[data-tab="program"]', "#view-program");
  await page.waitForSelector("#createProcessBtn", { timeout: 15_000 });
  await page.locator("#createProcessBtn").click();
  try {
    await page.waitForSelector("#addProcessStepBtn", { timeout: 8_000 });
  } catch (error) {
    const programText = await page.locator("#programRoot").textContent().catch(() => "");
    const processes = await fetchJson(`${baseUrl}/api/processes`).catch((err) => ({
      error: err.message,
    }));
    throw new Error(
      `create process did not reveal #addProcessStepBtn\nprogram=${programText}\nprocesses=${JSON.stringify(processes)}`,
    );
  }
  await page.locator("#addProcessStepBtn").click();
  await page.waitForSelector("#saveProcessStepBtn", { timeout: 8_000 });
  await waitForJson(
    `${baseUrl}/api/processes`,
    (payload) => Array.isArray(payload.data) && payload.data.some((process) => process.step_count > 0),
    10_000,
  );

  await clickTab(page, 'button[data-tab="ai"]', "#view-ai");
  await expectText(page, "#view-ai", "AI");

  await page.locator('.nav-settings[data-tab="settings"]').click();
  await page.waitForSelector("#view-settings.active");
  await expectText(page, "#settingsDevices", "reactor_001");
  await page.waitForSelector(
    '.component-action[data-component-id="shake_stepper"][data-action="speed_up"]',
    { timeout: 10_000 },
  );
  await page.waitForSelector(
    '.component-action[data-component-id="stirrer_motor"][data-action="set_rpm"]',
    { timeout: 10_000 },
  );

  const before = await fetchJson(`${baseUrl}/api/devices/status`);
  const device = before.data?.devices?.[0];
  assert(device?.online === true, "device is not online before component click");

  const speedUp = page.locator(
    '.component-action[data-component-id="shake_stepper"][data-action="speed_up"]',
  );
  await speedUp.click();
  await waitForJson(
    `${baseUrl}/api/devices/status`,
    (payload) =>
      payload.data?.devices?.[0]?.last_command_request_id &&
      payload.data?.devices?.[0]?.last_command_ok === true,
    10_000,
  );

  const rpmButton = page.locator(
    '.component-action[data-component-id="stirrer_motor"][data-action="set_rpm"]',
  );
  await rpmButton.click();
  await waitForJson(
    `${baseUrl}/api/devices/status`,
    (payload) => {
      const current = payload.data?.devices?.[0]?.components?.find(
        (component) => component.component_id === "stirrer_motor",
      )?.state?.target_stirrer_rpm;
      return Number(current) === 300;
    },
    10_000,
  );

  await clickTab(page, 'button[data-tab="alarms"]', "#view-alarms");
  await page.waitForSelector("#activeAlarmRowsFull", { timeout: 8_000 });

  const overflow = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  assert(
    overflow.scrollWidth <= overflow.clientWidth + 1,
    `horizontal overflow: ${overflow.scrollWidth} > ${overflow.clientWidth}`,
  );

  assert(consoleErrors.length === 0, `console errors: ${consoleErrors.join(" | ")}`);

  const after = await fetchJson(`${baseUrl}/api/devices/status`);
  const afterDevice = after.data?.devices?.[0];
  const summary = {
    baseUrl,
    screenshotPath,
    online: afterDevice?.online,
    sensors: afterDevice?.sensors?.length,
    components: afterDevice?.components?.map((component) => ({
      id: component.component_id,
      status: component.status,
      actions: component.actions?.map((action) => action.action),
    })),
    lastCommandOk: afterDevice?.last_command_ok,
    lastCommandRequestId: afterDevice?.last_command_request_id,
  };

  console.log(`\nVISUAL_CLICK_RESULT ${JSON.stringify(summary, null, 2)}`);
} finally {
  if (browser) await browser.close();
  child.kill("SIGTERM");
  setTimeout(() => child.kill("SIGKILL"), 2000).unref();
}

async function fetchJson(url) {
  const response = await fetch(url, { headers: { accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}: ${await response.text()}`);
  }
  return response.json();
}

async function waitForHttp(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      await fetchJson(url);
      return;
    } catch (error) {
      lastError = error;
      await sleep(500);
    }
  }
  let log = "";
  try {
    log = await readFile(logPath, "utf8");
  } catch {
    // ignore log read errors
  }
  throw new Error(`Timed out waiting for ${url}: ${lastError?.message}\n${log}`);
}

async function waitForJson(url, predicate, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let lastPayload;
  while (Date.now() < deadline) {
    lastPayload = await fetchJson(url);
    if (predicate(lastPayload)) return lastPayload;
    await sleep(500);
  }
  throw new Error(`Timed out waiting for ${url}; last=${JSON.stringify(lastPayload)}`);
}

async function clickTab(page, selector, viewSelector) {
  await page.locator(selector).click();
  await page.waitForSelector(`${viewSelector}.active`, { timeout: 8_000 });
}

async function expectText(page, selector, text) {
  const locator = page.locator(selector);
  await locator.waitFor({ timeout: 8_000 });
  const content = await locator.textContent();
  assert(content?.includes(text), `${selector} did not contain ${text}`);
}

async function assertCanvasInk(page, selector) {
  const hasInk = await page.locator(selector).evaluate((canvas) => {
    const ctx = canvas.getContext("2d");
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] !== 0) return true;
    }
    return false;
  });
  assert(hasInk, `${selector} is blank`);
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
