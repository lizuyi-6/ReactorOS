#!/usr/bin/env node
// LubanCat 2 / RK3568 QEMU 仿真性能门禁：一次 QEMU 启动内执行多轮 Vue HMI
// 冷启动测量，再以独立 browser context 执行一次持续态观测。
// 冷测链路：DOMContentLoaded → 主趋势区域 + 实时数据就绪 → 主趋势 canvas 绘制。
// 登录、本地鉴权注入及管线喂样方式与 e2e/vue.helpers.mjs 保持一致。

import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

import { chromium } from "@playwright/test";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = path.join(repoRoot, "output", "playwright");
const logPath = path.join(outputDir, "lubancat2-qemu-perf.log");
const reportPath = path.join(outputDir, "lubancat2-qemu-perf.json");
const trendSelector = '[data-testid="monitor-main-trend"]';
const readinessTimeoutMs = 15_000;

// 与 e2e/vue.helpers.mjs 保持同一份管线样本，确保 /api/live 有数据可渲染。
const PIPELINE_SAMPLE = {
  temperature_c: 31.11,
  pressure_mpa: 0.5,
  stirrer_rpm: 125.18,
  shake_speed_cpm: 30.0,
  tilt_state: 1,
  flow_rate_l_min: 2.42,
  product_concentration_percent: 11.1,
  ph: 6.15,
};

const report = {
  generatedAt: new Date().toISOString(),
  config: null,
  runs: [],
  summary: null,
  steady: null,
  errors: [],
};

let child;
let browser;
let pipelineTimer;
let pipelinePostPromise;
let config;

await mkdir(outputDir, { recursive: true });
await writeFile(logPath, "", "utf8");
await safeWriteReport();

try {
  config = readConfig();
  report.config = config;
  await safeWriteReport();

  const qemuArgs = [`--bind ${shellQuote(config.bind)}`];
  if (config.assets) qemuArgs.push(`--assets ${shellQuote(config.assets)}`);
  child = spawn(
    "wsl.exe",
    [
      "-e",
      "bash",
      "-lc",
      `cd /mnt/x/tianhks && exec ./scripts/run-lubancat2-qemu.sh ${qemuArgs.join(" ")}`,
    ],
    { cwd: repoRoot, stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
  );

  const appendLog = (chunk) => {
    writeFile(logPath, chunk.toString(), { flag: "a" }).catch(() => {});
  };
  child.stdout.on("data", appendLog);
  child.stderr.on("data", appendLog);

  let childSpawnError;
  child.once("error", (error) => {
    childSpawnError = error;
  });
  await waitForHttp(`${config.baseUrl}/health`, 45_000, () => childSpawnError);

  // 登录拿 token，再持续喂管线样本，让 /api/live 返回 200（外部管线模式下
  // 无样本会约定性 503，那是预期空态而不是性能信号）。
  const token = await login();
  const postSample = async () => {
    const response = await fetch(`${config.baseUrl}/api/v1/reactor/reactor_001/samples`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify(PIPELINE_SAMPLE),
    });
    if (!response.ok) {
      throw new Error(`pipeline sample returned HTTP ${response.status}: ${await response.text()}`);
    }
  };
  await postSample();
  pipelineTimer = setInterval(() => {
    if (pipelinePostPromise) return;
    pipelinePostPromise = postSample()
      .catch(() => {})
      .finally(() => {
        pipelinePostPromise = undefined;
      });
  }, 2000);
  await waitForLiveSamples(token, 20_000);

  browser = await chromium.launch({ headless: true });
  for (let index = 1; index <= config.runs; index += 1) {
    const run = await measureColdRun(browser, token, index);
    report.runs.push(run);
    await safeWriteReport();
  }

  report.steady = await measureSteadyState(browser, token);
  await safeWriteReport();
} catch (error) {
  report.errors.push(serializeError(error, "setup"));
} finally {
  if (pipelineTimer) clearInterval(pipelineTimer);
  if (pipelinePostPromise) await pipelinePostPromise.catch(() => {});
  if (browser) await browser.close().catch((error) => {
    report.errors.push(serializeError(error, "browser-cleanup"));
  });
  if (child) await stopChild(child).catch((error) => {
    report.errors.push(serializeError(error, "qemu-cleanup"));
  });

  report.summary = buildSummary(report.runs, report.steady, config);
  report.errors = collectReportErrors(report);
  report.completedAt = new Date().toISOString();
  await safeWriteReport();
  console.log(JSON.stringify(report, null, 2));

  const gate = report.summary?.gate;
  if (gate && !gate.passed) {
    process.exitCode = gate.correctnessViolations.length > 0 ? 1 : 2;
  } else if (report.errors.length > 0) {
    process.exitCode = 1;
  }
}

async function measureColdRun(activeBrowser, token, index) {
  const run = {
    index,
    status: "running",
    startedAt: new Date().toISOString(),
    domContentLoadedMs: null,
    dataReadyMs: null,
    chartReadyMs: null,
    totalTransferBytes: null,
    requestCount: 0,
    resourceCount: 0,
    consoleErrors: [],
    pageErrors: [],
    externalRequests: [],
    requests: [],
    liveResponses: [],
    navigation: null,
    resources: [],
    hmi: null,
    errors: [],
    diagnosticReasons: [],
    screenshot: null,
  };

  let context;
  let page;
  let tracker;
  try {
    context = await activeBrowser.newContext({ viewport: { width: 1280, height: 800 } });
    await installInitScripts(context, token, false);
    page = await context.newPage();
    tracker = attachPageDiagnostics(page);

    const start = performance.now();
    await page.goto(`${config.baseUrl}/#/monitor`, { waitUntil: "domcontentloaded" });
    run.domContentLoadedMs = roundMs(performance.now() - start);

    await waitForDataReady(page, tracker, readinessTimeoutMs);
    run.dataReadyMs = roundMs(performance.now() - start);

    await waitForMainTrendCanvas(page, readinessTimeoutMs);
    run.chartReadyMs = roundMs(performance.now() - start);
  } catch (error) {
    run.errors.push(serializeError(error, `cold-run-${index}`));
    run.diagnosticReasons.push("measurement-error");
  } finally {
    if (tracker) applyTracker(run, tracker);
    if (page) await collectPageMetrics(page, run);

    if (run.domContentLoadedMs !== null && run.domContentLoadedMs > config.thresholds.domContentLoadedMs) {
      run.diagnosticReasons.push("dom-threshold-exceeded");
    }
    if (run.dataReadyMs !== null && run.dataReadyMs > config.thresholds.dataReadyMs) {
      run.diagnosticReasons.push("data-threshold-exceeded");
    }
    if (run.chartReadyMs !== null && run.chartReadyMs > config.thresholds.chartReadyMs) {
      run.diagnosticReasons.push("chart-threshold-exceeded");
    }
    if (run.consoleErrors.length > 0) run.diagnosticReasons.push("console-errors");
    if (run.pageErrors.length > 0) run.diagnosticReasons.push("page-errors");
    if (run.externalRequests.length > 0) run.diagnosticReasons.push("external-requests");
    run.diagnosticReasons = [...new Set(run.diagnosticReasons)];

    if (page && run.diagnosticReasons.length > 0) {
      run.screenshot = await captureFailureScreenshot(
        page,
        `lubancat2-qemu-perf-run-${String(index).padStart(2, "0")}-failure.png`,
        run.errors,
      );
    }

    run.status = run.diagnosticReasons.length > 0 || run.errors.length > 0
      ? "failed"
      : "completed";
    run.completedAt = new Date().toISOString();
    if (context) await context.close().catch((error) => {
      run.errors.push(serializeError(error, `cold-run-${index}-cleanup`));
      run.status = "failed";
    });
  }
  return run;
}

async function measureSteadyState(activeBrowser, token) {
  const steady = {
    status: "running",
    startedAt: new Date().toISOString(),
    durationMs: config.steadyMs,
    initialization: {
      domContentLoadedMs: null,
      dataReadyMs: null,
      chartReadyMs: null,
    },
    window: null,
    consoleErrors: [],
    pageErrors: [],
    externalRequests: [],
    requests: [],
    liveResponses: [],
    errors: [],
    diagnosticReasons: [],
    screenshot: null,
  };

  let context;
  let page;
  let tracker;
  try {
    context = await activeBrowser.newContext({ viewport: { width: 1280, height: 800 } });
    await installInitScripts(context, token, true);
    page = await context.newPage();
    tracker = attachPageDiagnostics(page);

    const start = performance.now();
    await page.goto(`${config.baseUrl}/#/monitor`, { waitUntil: "domcontentloaded" });
    steady.initialization.domContentLoadedMs = roundMs(performance.now() - start);
    await waitForDataReady(page, tracker, readinessTimeoutMs);
    steady.initialization.dataReadyMs = roundMs(performance.now() - start);
    await waitForMainTrendCanvas(page, readinessTimeoutMs);
    steady.initialization.chartReadyMs = roundMs(performance.now() - start);

    const windowStart = await page.evaluate(() => ({
      performanceNow: performance.now(),
      heapUsedBytes: performance.memory?.usedJSHeapSize ?? null,
      observerSupported: Boolean(window.__lubanPerfLongTaskSupported),
    }));
    tracker.windowActive = true;
    tracker.windowRequests = [];
    const wallStart = performance.now();
    await sleep(config.steadyMs);
    tracker.windowActive = false;
    const measuredDurationMs = roundMs(performance.now() - wallStart);

    const windowEnd = await page.evaluate((startTime) => {
      const endTime = performance.now();
      const longTasks = Array.isArray(window.__lubanPerfLongTasks)
        ? window.__lubanPerfLongTasks.filter(
            (entry) => entry.startTime >= startTime && entry.startTime < endTime,
          )
        : [];
      return {
        performanceNow: endTime,
        heapUsedBytes: performance.memory?.usedJSHeapSize ?? null,
        longTasks,
      };
    }, windowStart.performanceNow);

    const windowRequests = tracker.windowRequests;
    const liveRequests = windowRequests.filter((request) => isLiveUrl(request.url));
    const longTaskDurations = windowEnd.longTasks.map((entry) => entry.duration);
    steady.window = {
      configuredDurationMs: config.steadyMs,
      measuredDurationMs,
      requestCount: windowRequests.length,
      liveRequestCount: liveRequests.length,
      requests: windowRequests,
      liveRequests,
      heap: {
        available: windowStart.heapUsedBytes !== null && windowEnd.heapUsedBytes !== null,
        startBytes: windowStart.heapUsedBytes,
        endBytes: windowEnd.heapUsedBytes,
        deltaBytes:
          windowStart.heapUsedBytes !== null && windowEnd.heapUsedBytes !== null
            ? windowEnd.heapUsedBytes - windowStart.heapUsedBytes
            : null,
      },
      longTasks: {
        observerSupported: windowStart.observerSupported,
        count: windowEnd.longTasks.length,
        totalDurationMs: roundMs(longTaskDurations.reduce((sum, value) => sum + value, 0)),
        maxDurationMs: longTaskDurations.length > 0
          ? roundMs(Math.max(...longTaskDurations))
          : 0,
        entries: windowEnd.longTasks,
      },
    };
  } catch (error) {
    steady.errors.push(serializeError(error, "steady"));
    steady.diagnosticReasons.push("measurement-error");
  } finally {
    if (tracker) {
      tracker.windowActive = false;
      applyTracker(steady, tracker);
    }
    if (steady.consoleErrors.length > 0) steady.diagnosticReasons.push("console-errors");
    if (steady.pageErrors.length > 0) steady.diagnosticReasons.push("page-errors");
    if (steady.externalRequests.length > 0) steady.diagnosticReasons.push("external-requests");
    steady.diagnosticReasons = [...new Set(steady.diagnosticReasons)];

    if (page && steady.diagnosticReasons.length > 0) {
      steady.screenshot = await captureFailureScreenshot(
        page,
        "lubancat2-qemu-perf-steady-failure.png",
        steady.errors,
      );
    }
    steady.status = steady.errors.length > 0 || steady.consoleErrors.length > 0 ||
      steady.pageErrors.length > 0 || steady.externalRequests.length > 0
      ? "failed"
      : "completed";
    steady.completedAt = new Date().toISOString();
    if (context) await context.close().catch((error) => {
      steady.errors.push(serializeError(error, "steady-cleanup"));
      steady.status = "failed";
    });
  }
  return steady;
}

async function installInitScripts(context, token, observeLongTasks) {
  await context.addInitScript(
    ({ authToken, enableLongTasks }) => {
      localStorage.setItem("reactoros.vue.auth.token", authToken);
      localStorage.setItem(
        "reactoros.vue.auth.user",
        JSON.stringify({ username: "engineer", role: "engineer", permissions: [] }),
      );
      localStorage.setItem("reactoros.vue.language", "zh");

      if (enableLongTasks) {
        window.__lubanPerfLongTasks = [];
        window.__lubanPerfLongTaskSupported = false;
        try {
          const observer = new PerformanceObserver((list) => {
            for (const entry of list.getEntries()) {
              window.__lubanPerfLongTasks.push({
                name: entry.name,
                startTime: entry.startTime,
                duration: entry.duration,
              });
            }
          });
          observer.observe({ type: "longtask", buffered: true });
          window.__lubanPerfLongTaskObserver = observer;
          window.__lubanPerfLongTaskSupported = true;
        } catch {
          // Long Tasks API 不可用时保留 supported=false，持续态其余指标仍可采集。
        }
      }
    },
    { authToken: token, enableLongTasks: observeLongTasks },
  );
}

function attachPageDiagnostics(page) {
  const tracker = {
    requests: [],
    requestMap: new Map(),
    consoleErrors: [],
    pageErrors: [],
    liveResponses: [],
    liveDataReady: false,
    windowActive: false,
    windowRequests: [],
  };

  page.on("request", (request) => {
    const record = {
      url: request.url(),
      method: request.method(),
      resourceType: request.resourceType(),
      status: null,
      failure: null,
      timestamp: new Date().toISOString(),
    };
    tracker.requests.push(record);
    tracker.requestMap.set(request, record);
    if (tracker.windowActive) tracker.windowRequests.push(record);
  });
  page.on("response", (response) => {
    const record = tracker.requestMap.get(response.request());
    if (record) record.status = response.status();
    if (!isLiveUrl(response.url())) return;

    const liveRecord = {
      url: response.url(),
      status: response.status(),
      dataReady: false,
      timestamp: new Date().toISOString(),
    };
    tracker.liveResponses.push(liveRecord);
    if (!response.ok()) return;
    response.json().then((body) => {
      liveRecord.dataReady = hasLiveTemperature(body);
      if (liveRecord.dataReady) tracker.liveDataReady = true;
    }).catch((error) => {
      liveRecord.parseError = error.message;
    });
  });
  page.on("requestfailed", (request) => {
    const record = tracker.requestMap.get(request);
    if (record) record.failure = request.failure()?.errorText ?? "request failed";
  });
  page.on("console", (message) => {
    if (message.type() !== "error") return;
    tracker.consoleErrors.push({
      text: message.text(),
      location: message.location(),
      timestamp: new Date().toISOString(),
    });
  });
  page.on("pageerror", (error) => {
    tracker.pageErrors.push(serializeError(error, "page"));
  });
  return tracker;
}

async function waitForDataReady(page, tracker, timeoutMs) {
  const deadline = performance.now() + timeoutMs;
  let lastState;
  while (performance.now() < deadline) {
    lastState = await page.evaluate((selector) => {
      const anchor = document.querySelector(selector);
      const temperatureReady = [...document.querySelectorAll(".param-card")].some((card) => {
        const unit = card.querySelector(".unit")?.textContent?.trim() ?? "";
        const value = card.querySelector(".pv")?.textContent?.trim() ?? "";
        return unit.includes("°C") && value !== "--" && Number.isFinite(Number(value));
      });
      return { anchorReady: Boolean(anchor), temperatureReady };
    }, trendSelector);
    if (lastState.anchorReady && (lastState.temperatureReady || tracker.liveDataReady)) return;
    await sleep(50);
  }
  throw new Error(
    `timed out waiting for monitor data readiness: ${JSON.stringify({
      ...lastState,
      liveDataReady: tracker.liveDataReady,
      liveResponses: tracker.liveResponses,
    })}`,
  );
}

async function waitForMainTrendCanvas(page, timeoutMs) {
  await page.waitForFunction(
    (selector) => {
      const anchor = document.querySelector(selector);
      if (!anchor) return false;
      const canvases = [...anchor.querySelectorAll("canvas")];
      return canvases.some((canvas) => {
        if (canvas.width <= 0 || canvas.height <= 0) return false;
        try {
          const context = canvas.getContext("2d");
          if (!context) return false;
          const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
          for (let offset = 3; offset < pixels.length; offset += 4) {
            if (pixels[offset] !== 0) return true;
          }
        } catch {
          return false;
        }
        return false;
      });
    },
    trendSelector,
    { timeout: timeoutMs },
  );
}

async function collectPageMetrics(page, target) {
  try {
    const metrics = await page.evaluate((selector) => {
      const navigationEntry = performance.getEntriesByType("navigation")[0];
      const resources = performance.getEntriesByType("resource").map((entry) => ({
        name: entry.name,
        startTime: Math.round(entry.startTime),
        transferSize: entry.transferSize,
        encodedBodySize: entry.encodedBodySize,
        decodedBodySize: entry.decodedBodySize,
        duration: Math.round(entry.duration),
        initiatorType: entry.initiatorType,
      }));
      const anchor = document.querySelector(selector);
      return {
        navigation: navigationEntry
          ? {
              name: navigationEntry.name,
              domInteractive: Math.round(navigationEntry.domInteractive),
              domContentLoadedEventStart: Math.round(navigationEntry.domContentLoadedEventStart),
              domContentLoadedEventEnd: Math.round(navigationEntry.domContentLoadedEventEnd),
              loadEventStart: Math.round(navigationEntry.loadEventStart),
              loadEventEnd: Math.round(navigationEntry.loadEventEnd),
              duration: Math.round(navigationEntry.duration),
              transferSize: navigationEntry.transferSize,
              encodedBodySize: navigationEntry.encodedBodySize,
              decodedBodySize: navigationEntry.decodedBodySize,
            }
          : null,
        resources,
        hmi: {
          route: location.hash,
          title: document.title,
          trendAnchorPresent: Boolean(anchor),
          trendCanvasCount: anchor?.querySelectorAll("canvas").length ?? 0,
          totalCanvasCount: document.querySelectorAll("canvas").length,
        },
      };
    }, trendSelector);
    target.navigation = metrics.navigation;
    target.resources = metrics.resources;
    target.resourceCount = metrics.resources.length;
    target.hmi = metrics.hmi;
    target.totalTransferBytes = [
      metrics.navigation?.transferSize || 0,
      ...metrics.resources.map((resource) => resource.transferSize || 0),
    ].reduce((sum, size) => sum + size, 0);
  } catch (error) {
    target.errors.push(serializeError(error, "metrics-collection"));
    target.diagnosticReasons.push("metrics-collection-error");
  }
}

function applyTracker(target, tracker) {
  target.requests = tracker.requests;
  target.requestCount = tracker.requests.length;
  target.consoleErrors = tracker.consoleErrors;
  target.pageErrors = tracker.pageErrors;
  target.liveResponses = tracker.liveResponses;
  target.externalRequests = tracker.requests.filter((request) => isExternalUrl(request.url));
}

function buildSummary(runs, steady, activeConfig) {
  const metricNames = [
    "domContentLoadedMs",
    "dataReadyMs",
    "chartReadyMs",
    "totalTransferBytes",
    "requestCount",
    "resourceCount",
  ];
  const metrics = {};
  for (const name of metricNames) {
    metrics[name] = summarizeNumbers(runs.map((run) => run[name]));
  }
  metrics.consoleErrorCount = summarizeNumbers(runs.map((run) => run.consoleErrors.length));
  metrics.pageErrorCount = summarizeNumbers(runs.map((run) => run.pageErrors.length));
  metrics.externalRequestCount = summarizeNumbers(runs.map((run) => run.externalRequests.length));

  const performanceViolations = [];
  const correctnessViolations = [];
  if (!activeConfig) {
    correctnessViolations.push("configuration or setup failed before performance measurements");
  } else {
    const thresholdChecks = [
      ["domContentLoadedMs", activeConfig.thresholds.domContentLoadedMs],
      ["dataReadyMs", activeConfig.thresholds.dataReadyMs],
      ["chartReadyMs", activeConfig.thresholds.chartReadyMs],
    ];
    for (const [name, threshold] of thresholdChecks) {
      const p95 = metrics[name].p95;
      if (p95 === null) {
        performanceViolations.push(`${name} has no measurement`);
      } else if (p95 > threshold) {
        performanceViolations.push(`${name} P95 ${p95}ms exceeds ${threshold}ms`);
      }
    }
  }

  for (const run of runs) {
    if (run.errors.length > 0) correctnessViolations.push(`cold run ${run.index} measurement failed`);
    if (run.consoleErrors.length > 0) correctnessViolations.push(`cold run ${run.index} has console errors`);
    if (run.pageErrors.length > 0) correctnessViolations.push(`cold run ${run.index} has page errors`);
    if (run.externalRequests.length > 0) correctnessViolations.push(`cold run ${run.index} has external requests`);
  }
  if (!steady) {
    correctnessViolations.push("steady-state measurement did not run");
  } else {
    if (steady.errors.length > 0) correctnessViolations.push("steady-state measurement failed");
    if (steady.consoleErrors.length > 0) correctnessViolations.push("steady-state has console errors");
    if (steady.pageErrors.length > 0) correctnessViolations.push("steady-state has page errors");
    if (steady.externalRequests.length > 0) correctnessViolations.push("steady-state has external requests");
  }
  if (report.errors.length > 0) correctnessViolations.push("setup or cleanup errors occurred");

  const uniquePerformance = [...new Set(performanceViolations)];
  const uniqueCorrectness = [...new Set(correctnessViolations)];
  return {
    percentileStrategy: "nearest-rank across available cold-run measurements; for 1..10 runs P95 is the maximum",
    configuredRuns: activeConfig?.runs ?? null,
    completedRuns: runs.filter((run) => run.status === "completed").length,
    failedRuns: runs.filter((run) => run.status === "failed").length,
    metrics,
    gate: {
      passed: uniquePerformance.length === 0 && uniqueCorrectness.length === 0,
      performanceViolations: uniquePerformance,
      correctnessViolations: uniqueCorrectness,
    },
  };
}

function summarizeNumbers(values) {
  const numbers = values.filter((value) => typeof value === "number" && Number.isFinite(value));
  if (numbers.length === 0) return { count: 0, p50: null, p95: null, min: null, max: null };
  const sorted = [...numbers].sort((left, right) => left - right);
  return {
    count: sorted.length,
    p50: nearestRank(sorted, 0.5),
    p95: nearestRank(sorted, 0.95),
    min: sorted[0],
    max: sorted.at(-1),
  };
}

function nearestRank(sorted, percentile) {
  return sorted[Math.max(0, Math.ceil(percentile * sorted.length) - 1)];
}

function collectReportErrors(currentReport) {
  const collected = [...currentReport.errors];
  for (const run of currentReport.runs) {
    collected.push(...run.errors);
    collected.push(...run.consoleErrors.map((error) => ({ scope: `cold-run-${run.index}-console`, ...error })));
    collected.push(...run.pageErrors.map((error) => ({ ...error, scope: `cold-run-${run.index}-page` })));
    for (const request of run.externalRequests) {
      collected.push({ scope: `cold-run-${run.index}-external-request`, message: request.url });
    }
  }
  if (currentReport.steady) {
    collected.push(...currentReport.steady.errors);
    collected.push(...currentReport.steady.consoleErrors.map((error) => ({ scope: "steady-console", ...error })));
    collected.push(...currentReport.steady.pageErrors.map((error) => ({ ...error, scope: "steady-page" })));
    for (const request of currentReport.steady.externalRequests) {
      collected.push({ scope: "steady-external-request", message: request.url });
    }
  }
  return collected;
}

async function captureFailureScreenshot(page, fileName, errors) {
  const screenshotPath = path.join(outputDir, fileName);
  try {
    await page.screenshot({ path: screenshotPath, fullPage: true, timeout: 5000 });
    return screenshotPath;
  } catch (error) {
    errors.push(serializeError(error, "screenshot"));
    return null;
  }
}

function readConfig() {
  const requestedBaseUrl = (process.env.REACTOR_OS_QEMU_URL || "http://127.0.0.1:18080")
    .replace(/\/+$/, "");
  const parsedBaseUrl = new URL(requestedBaseUrl);
  return {
    baseUrl: requestedBaseUrl,
    bind: parsedBaseUrl.host,
    assets: normalizeWslPath(process.env.REACTOR_OS_QEMU_ASSETS),
    runs: readIntegerEnv("PERF_RUNS", 3, 1, 10),
    steadyMs: readIntegerEnv("PERF_STEADY_MS", 30_000, 1000),
    thresholds: {
      domContentLoadedMs: readIntegerEnv("PERF_DOM_MAX_MS", 2500, 1),
      dataReadyMs: readIntegerEnv("PERF_DATA_MAX_MS", 5000, 1),
      chartReadyMs: readIntegerEnv("PERF_CHART_MAX_MS", 6000, 1),
    },
    percentileStrategy: "nearest-rank P95",
    viewport: { width: 1280, height: 800 },
    trendSelector,
  };
}

function normalizeWslPath(value) {
  if (!value) return null;
  const normalized = String(value).replaceAll("\\", "/");
  const windowsPath = normalized.match(/^([A-Za-z]):\/(.*)$/);
  if (windowsPath) return `/mnt/${windowsPath[1].toLowerCase()}/${windowsPath[2]}`;
  return normalized;
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", `'"'"'`)}'`;
}

function readIntegerEnv(name, fallback, minimum, maximum = Number.MAX_SAFE_INTEGER) {
  const raw = process.env[name];
  if (raw === undefined || raw === "") return fallback;
  const value = Number(raw);
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${name} must be an integer from ${minimum} to ${maximum}; received ${raw}`);
  }
  return value;
}

async function login() {
  const body = await fetchJson(
    `${config.baseUrl}/api/auth/login`,
    null,
    "POST",
    JSON.stringify({ username: "engineer", password: "engineer123" }),
  );
  const token = body.data?.token ?? body.token;
  if (!token) throw new Error("login response has no token");
  return token;
}

async function waitForLiveSamples(token, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const live = await fetchJson(`${config.baseUrl}/api/live`, token);
      if (hasLiveTemperature(live)) return;
    } catch {
      // /api/live 在首个管线样本落地前可能约定性返回 503。
    }
    await sleep(500);
  }
  throw new Error("timed out waiting for /api/live samples");
}

function hasLiveTemperature(body) {
  const live = body?.data ?? body;
  const latest = live?.runtime?.latest_sample ?? live?.recent_samples?.at?.(-1);
  return typeof latest?.temperature_c === "number" && Number.isFinite(latest.temperature_c);
}

function isLiveUrl(url) {
  try {
    return new URL(url).pathname === "/api/live";
  } catch {
    return false;
  }
}

function isExternalUrl(url) {
  try {
    const candidate = new URL(url);
    const expected = new URL(config.baseUrl);
    if (!["http:", "https:", "ws:", "wss:"].includes(candidate.protocol)) return false;
    return candidate.host !== expected.host;
  } catch {
    return true;
  }
}

async function fetchJson(url, token, method = "GET", body) {
  const headers = { accept: "application/json" };
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const response = await fetch(url, { method, headers, body });
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}: ${await response.text()}`);
  }
  return response.json();
}

async function waitForHttp(url, timeoutMs, getStartupError) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    const startupError = getStartupError?.();
    if (startupError) throw startupError;
    if (child && child.exitCode !== null) {
      throw new Error(`QEMU launcher exited with code ${child.exitCode} before ${url} became ready`);
    }
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

async function stopChild(processHandle) {
  if (processHandle.exitCode !== null || processHandle.signalCode !== null) return;
  const exited = new Promise((resolve) => processHandle.once("exit", resolve));
  processHandle.kill("SIGTERM");
  const stopped = await Promise.race([
    exited.then(() => true),
    sleep(2000).then(() => false),
  ]);
  if (stopped) return;
  processHandle.kill("SIGKILL");
  await Promise.race([exited, sleep(2000)]);
}

async function safeWriteReport() {
  try {
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  } catch (error) {
    console.error(`failed to write ${reportPath}: ${error.message}`);
  }
}

function serializeError(error, scope) {
  return {
    scope,
    name: error?.name ?? "Error",
    message: error?.message ?? String(error),
    stack: error?.stack ?? null,
    timestamp: new Date().toISOString(),
  };
}

function roundMs(value) {
  return Math.round(value * 100) / 100;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
