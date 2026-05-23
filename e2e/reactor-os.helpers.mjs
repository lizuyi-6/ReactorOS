import { expect } from "@playwright/test";

export const selectors = {
  tabs: ".tab-btn",
  monitorTab: '.tab-btn[data-tab="monitor"]',
  programTab: '.tab-btn[data-tab="program"]',
  recipesTab: '.tab-btn[data-tab="recipes"]',
  materialsTab: '.tab-btn[data-tab="materials"]',
  aiTab: '.tab-btn[data-tab="ai"]',
  alarmsTab: '.tab-btn[data-tab="alarms"]',
  activeView: ".view.active",
  sideSensors: "#sideSensors",
  alarmSummary: "#alarmSummary",
  runState: "#runState",
  runClock: "#runClock",
  mainChart: "#mainChart",
  batchSummary: "#batchSummary",
  feedSummary: "#feedSummary",
  timeline: "#timeline",
  addStage: "#addStageBtn",
  startProgram: "#startProgramBtn",
  stageTemp: "#stageTemp",
  recipeList: "#recipeList",
  materialRows: "#materialRows",
  addMaterial: "#addMaterialBtn",
  ratioRaw: "#ratioRaw",
  productMass: "#productMass",
  theoreticalMass: "#theoreticalMass",
  computedYield: "#computedYield",
  aiRecommendation: "#aiRecommendation",
  applyAi: "#applyAiBtn",
  activeAlarmRows: "#activeAlarmRows",
  alarmHistoryRows: "#alarmHistoryRows",
  exportTrigger: "#exportTrigger",
  exportMenu: ".export-menu"
};

export const pipelineSample = {
  temperature_c: 31.11,
  pressure_mpa: 0.50,
  stirrer_rpm: 125.18,
  shake_speed_cpm: 30.00,
  flow_rate_l_min: 2.42,
  product_concentration_percent: 11.10,
  ph: 6.15
};

export async function injectPipelineSample(request, sample = pipelineSample) {
  const response = await request.post("/api/v1/reactor/reactor_001/samples", { data: sample });
  expect(response.status()).toBe(200);
  const payload = await response.json();
  expect(payload.code).toBe(0);
  return payload.data.sample;
}

export async function keepPipelineFlowing(request) {
  await injectPipelineSample(request);
  return setInterval(() => {
    injectPipelineSample(request).catch(() => {});
  }, 2000);
}

export async function preparePage(page, request) {
  await request.post("/api/test/reset");
  const unavailable = await request.get("/api/live");
  expect(unavailable.status()).toBe(503);
  const pipelineInterval = await keepPipelineFlowing(request);
  await expect
    .poll(async () => {
      const response = await request.get("/api/live");
      if (response.status() !== 200) return 0;
      const live = await response.json();
      return live.recent_samples?.length || 0;
    }, { timeout: 12_000 })
    .toBeGreaterThan(0);
  const consoleErrors = [];
  page.on("console", message => {
    if (message.type() !== "error") return;
    const text = message.text();
    if (text.includes("Failed to load resource") && text.includes("ERR_CONNECTION_TIMED_OUT")) {
      return;
    }
    consoleErrors.push(text);
  });
  page.on("pageerror", error => consoleErrors.push(error.message));
  page.on("close", () => clearInterval(pipelineInterval));
  page.consoleErrors = consoleErrors;
  await page.goto("/");
  await page.waitForLoadState("domcontentloaded");
  await expect(page.locator("body")).toContainText("ReactorOS");
  await expect(page.locator(selectors.sideSensors)).toContainText("TEMP");
  await expect(page.locator(selectors.activeView)).toHaveAttribute("id", "view-monitor");
  await page.waitForFunction(() => window.__reactorState?.dataReady === true, null, { timeout: 12_000 });
}

export function assertNoConsoleErrors(page) {
  expect(page.consoleErrors, "browser console/page errors").toEqual([]);
}

export async function assertNoHorizontalOverflow(page) {
  const overflow = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth
  }));
  expect(
    overflow.scrollWidth,
    `horizontal overflow: scrollWidth=${overflow.scrollWidth}, clientWidth=${overflow.clientWidth}`
  ).toBeLessThanOrEqual(overflow.clientWidth + 1);
}

export async function assertNoTextClipping(page) {
  const clipped = await page.evaluate(() => {
    const bad = [];
    const candidates = Array.from(
      document.querySelectorAll(
        "button, .sensor-label, .sensor-number, .kv-row, .recipe-row, .alarm-cell, th, td, .label, .metric-number"
      )
    );
    for (const el of candidates) {
      const style = getComputedStyle(el);
      if (style.display === "none" || style.visibility === "hidden") continue;
      if (el.scrollWidth > el.clientWidth + 3 || el.scrollHeight > el.clientHeight + 3) {
        bad.push({
          text: el.textContent.trim(),
          className: el.className,
          scrollWidth: el.scrollWidth,
          clientWidth: el.clientWidth,
          scrollHeight: el.scrollHeight,
          clientHeight: el.clientHeight
        });
      }
    }
    return bad;
  });
  expect(clipped, "text clipping or squeezed labels").toEqual([]);
}

export async function assertCanvasHasInk(page, selector) {
  const hasInk = await page.locator(selector).evaluate(canvas => {
    const ctx = canvas.getContext("2d");
    const { width, height } = canvas;
    const data = ctx.getImageData(0, 0, width, height).data;
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] !== 0) return true;
    }
    return false;
  });
  expect(hasInk, `${selector} should not be blank`).toBe(true);
}

export async function switchTab(page, selector, expectedViewId) {
  await page.locator(selector).click();
  await expect(page.locator(selectors.activeView)).toHaveAttribute("id", expectedViewId);
}

export async function latestLive(request) {
  const response = await request.get("/api/live");
  expect(response.status()).toBe(200);
  return response.json();
}

export async function assertResponsiveLayout(page) {
  const result = await page.evaluate(() => {
    const sidebar = document.querySelector(".sidebar").getBoundingClientRect();
    const main = document.querySelector(".main").getBoundingClientRect();
    const sensors = getComputedStyle(document.querySelector("#sideSensors")).gridTemplateColumns;
    return { sidebarHeight: sidebar.height, mainLeft: main.left, sensorColumns: sensors.split(" ").length };
  });
  expect(result.mainLeft).toBeLessThan(2);
  expect(result.sidebarHeight).toBeLessThanOrEqual(90);
  expect(result.sensorColumns).toBeGreaterThanOrEqual(5);
}
