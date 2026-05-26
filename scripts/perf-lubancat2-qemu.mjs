#!/usr/bin/env node

import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "@playwright/test";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = path.join(repoRoot, "output", "playwright");
const baseUrl = process.env.REACTOR_OS_QEMU_URL || "http://127.0.0.1:18080";
const bind = new URL(baseUrl).host;
const logPath = path.join(outputDir, "lubancat2-qemu-perf.log");
const reportPath = path.join(outputDir, "lubancat2-qemu-perf.json");

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
  { cwd: repoRoot, stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
);

const appendLog = (chunk) => {
  const text = chunk.toString();
  writeFile(logPath, text, { flag: "a" }).catch(() => {});
};
child.stdout.on("data", appendLog);
child.stderr.on("data", appendLog);

let browser;
try {
  await waitForHttp(`${baseUrl}/health`, 45_000);

  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const requests = [];
  const consoleErrors = [];
  page.on("requestfinished", async (request) => {
    const response = await request.response();
    requests.push({
      url: request.url(),
      status: response?.status(),
      resourceType: request.resourceType(),
    });
  });
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => consoleErrors.push(error.message));

  const start = performance.now();
  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  const domContentLoadedMs = Math.round(performance.now() - start);
  await page.waitForFunction(() => window.__reactorState?.dataReady === true, {
    timeout: 15_000,
  });
  const dataReadyMs = Math.round(performance.now() - start);
  await page.waitForFunction(() => {
    const canvas = document.querySelector("#mainChart");
    if (!canvas) return false;
    const ctx = canvas.getContext("2d");
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] !== 0) return true;
    }
    return false;
  }, { timeout: 15_000 });
  const chartReadyMs = Math.round(performance.now() - start);

  const metrics = await page.evaluate(() => {
    const nav = performance.getEntriesByType("navigation")[0];
    const resources = performance.getEntriesByType("resource").map((entry) => ({
      name: entry.name,
      transferSize: entry.transferSize,
      encodedBodySize: entry.encodedBodySize,
      duration: Math.round(entry.duration),
      initiatorType: entry.initiatorType,
    }));
    return {
      navigation: nav
        ? {
            domInteractive: Math.round(nav.domInteractive),
            domContentLoadedEventEnd: Math.round(nav.domContentLoadedEventEnd),
            loadEventEnd: Math.round(nav.loadEventEnd),
            transferSize: nav.transferSize,
            encodedBodySize: nav.encodedBodySize,
          }
        : null,
      resources,
      state: {
        dataReady: window.__reactorState?.dataReady,
        activeTab: window.__reactorState?.activeTab,
        sensorCount: window.__reactorState?.sensors?.length,
        historyPoints: window.__reactorState?.histories?.temperature?.length,
      },
    };
  });

  const totalTransfer = [
    metrics.navigation?.transferSize || 0,
    ...metrics.resources.map((resource) => resource.transferSize || 0),
  ].reduce((sum, size) => sum + size, 0);

  const report = {
    baseUrl,
    domContentLoadedMs,
    dataReadyMs,
    chartReadyMs,
    totalTransferBytes: totalTransfer,
    requestCount: requests.length,
    externalRequests: requests.filter((request) => !request.url.startsWith(baseUrl)),
    consoleErrors,
    metrics,
  };
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  console.log(JSON.stringify(report, null, 2));

  if (domContentLoadedMs > 2500 || dataReadyMs > 5000 || chartReadyMs > 6000) {
    process.exitCode = 2;
  }
  if (consoleErrors.length) process.exitCode = 1;
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
  throw new Error(`Timed out waiting for ${url}: ${lastError?.message}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
