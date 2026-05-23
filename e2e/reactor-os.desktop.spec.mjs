import { expect, test } from "@playwright/test";
import {
  assertCanvasHasInk,
  assertNoConsoleErrors,
  assertNoHorizontalOverflow,
  assertNoTextClipping,
  latestLive,
  pipelineSample,
  preparePage,
  selectors,
  switchTab
} from "./reactor-os.helpers.mjs";

test.beforeEach(async ({ page, request }) => {
  await preparePage(page, request);
});

test.afterEach(async ({ page }) => {
  assertNoConsoleErrors(page);
});

test("production HMI shows error state instead of fake readings without pipeline data", async ({ page, request }) => {
  await request.post("/api/test/reset");
  const unavailable = await request.get("/api/live");
  expect(unavailable.status()).toBe(503);
  const devices = await request.get("/api/devices/status");
  expect(devices.status()).toBe(200);
  const payload = await devices.json();
  expect(payload.data.online_count).toBe(0);
  expect(payload.data.devices[0].status).toBe("offline");

  const consoleErrors = [];
  page.on("console", message => {
    if (message.type() !== "error") return;
    const text = message.text();
    if (text.includes("Failed to load resource") && text.includes("503")) return;
    consoleErrors.push(text);
  });
  page.on("pageerror", error => consoleErrors.push(error.message));
  page.consoleErrors = consoleErrors;
  await page.goto("/");
  await page.waitForLoadState("domcontentloaded");
  await expect(page.locator(selectors.runState)).toContainText("Error 503");
  await expect(page.locator("#pipelineState")).toContainText("Devices 0/1 Pipeline 503");
  await expect(page.locator(selectors.sideSensors)).not.toContainText("31.11 °C");
  await expect(page.locator(selectors.sideSensors)).not.toContainText("0.50 MPa");
  await expect(page.locator(selectors.sideSensors)).not.toContainText("125.18 RPM");
});

test("HMI dashboard exposes backend-fed monitoring, program, materials, AI and alarms", async ({ page, request }) => {
  await assertNoHorizontalOverflow(page);
  await assertNoTextClipping(page);
  await expect(page.locator(selectors.runState)).toContainText(/Idle|Running|Alarm/);
  await expect(page.locator("body")).toContainText("Process Line");
  await expect(page.locator("body")).toContainText("Detector Signals");
  await expect(page.locator("body")).toContainText("Operator Control");
  const live = await latestLive(request);
  expect(live.runtime.latest_sample).toMatchObject(pipelineSample);
  const temp = pipelineSample.temperature_c.toFixed(2);
  const pressure = pipelineSample.pressure_mpa.toFixed(2);
  const rpm = pipelineSample.stirrer_rpm.toFixed(2);
  const shake = pipelineSample.shake_speed_cpm.toFixed(2);
  const flow = pipelineSample.flow_rate_l_min.toFixed(2);
  const concentration = pipelineSample.product_concentration_percent.toFixed(2);
  const ph = pipelineSample.ph.toFixed(2);
  const fittedTilt = live.runtime.latest_sample.tilt_angle_deg.toFixed(2);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${pressure} MPa`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${rpm} RPM`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${shake} CPM`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${fittedTilt} deg`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${flow} L/min`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${concentration} %`);
  await expect(page.locator(selectors.sideSensors)).toContainText(ph);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${temp} °C`);
  await expect(page.locator(selectors.batchSummary)).toContainText("Target Temp");
  await expect(page.locator(selectors.batchSummary)).toContainText(`${shake} CPM`);
  await expect(page.locator(selectors.batchSummary)).toContainText(`${fittedTilt} deg`);
  await expect(page.locator(selectors.batchSummary)).toContainText(`${flow} L/min`);
  await assertCanvasHasInk(page, selectors.mainChart);
  await assertCanvasHasInk(page, selectors.processCanvas);
  await assertCanvasHasInk(page, "#shakeChart");
  await assertCanvasHasInk(page, "#tiltChart");
  await assertCanvasHasInk(page, "#flowChart");

  await switchTab(page, selectors.programTab, "view-program");
  await expect(page.locator(selectors.timeline)).toContainText("Heat-up");
  await page.locator(selectors.addStage).click();
  await expect(page.locator(".stage-block")).toHaveCount(4);
  await page.locator(selectors.stageTemp).fill("188.50");
  await page.locator(selectors.stageTemp).blur();
  await expect(page.locator(selectors.stageTemp)).toHaveValue("188.50");
  await page.locator("#saveRecipeBtn").click();
  await expect(page.locator(selectors.activeView)).toHaveAttribute("id", "view-recipes");

  await expect(page.locator(selectors.recipeList)).toContainText("Recipe");
  await page.getByText("加载到程序").click();
  await expect(page.locator(selectors.activeView)).toHaveAttribute("id", "view-program");

  await switchTab(page, selectors.materialsTab, "view-materials");
  await expect(page.locator('#materialRows input[data-field="name"]').first()).toHaveValue("Reactant A");
  await page.locator(selectors.addMaterial).click();
  await expect(page.locator("#materialRows tr")).toHaveCount(4);
  await switchTab(page, selectors.programTab, "view-program");
  await page.locator(selectors.startProgram).click();
  await expect
    .poll(async () => {
      const next = await latestLive(request);
      return next.runtime.active_batch_id;
    }, { timeout: 12_000 })
    .not.toBeNull();
  await expect(page.locator(selectors.runState)).toContainText(/Running|Alarm/);
  await switchTab(page, selectors.materialsTab, "view-materials");
  await expect(page.locator("#finishBatchBtn")).toBeEnabled();
  await page.locator("#finishBatchBtn").click();
  await page.locator("#finishBatchBtn").click();
  await page.locator(selectors.productMass).fill("36.00");
  await page.locator(selectors.theoreticalMass).fill("48.00");
  await expect(page.locator(selectors.computedYield)).toContainText("75.00%");
  await page.locator("#saveProductBtn").click();
  await expect
    .poll(async () => {
      const next = await latestLive(request);
      return next.recent_events.map(event => event.event_type);
    }, { timeout: 20_000 })
    .toContain("product_result_recorded");

  await switchTab(page, selectors.aiTab, "view-ai");
  await expect(page.locator(selectors.aiRecommendation)).toContainText("Target Temp");
  await assertCanvasHasInk(page, "#sensitivityChart");
  await assertCanvasHasInk(page, "#learningChart");
  await page.locator(selectors.applyAi).click();
  await expect(page.locator(selectors.activeView)).toHaveAttribute("id", "view-program");

  await switchTab(page, selectors.alarmsTab, "view-alarms");
  await expect(page.locator(selectors.activeAlarmRows)).toContainText(/Acknowledge|No active alarms/);
  const before = await page.locator(".ack-btn").count();
  if (before > 0) {
    const acknowledgedId = await page.locator(".ack-btn").first().getAttribute("data-ack");
    await page.locator(".ack-btn").first().click();
    await expect(page.locator(`.ack-btn[data-ack="${acknowledgedId}"]`)).toHaveCount(0);
  }
});

test("export menu opens and offers all static export actions", async ({ page }) => {
  await page.locator(selectors.exportTrigger).click();
  await expect(page.locator(selectors.exportMenu)).toBeVisible();
  await expect(page.locator(selectors.exportMenu)).toContainText("批次报告 PDF");
  await expect(page.locator(selectors.exportMenu)).toContainText("历史数据 CSV");
  await expect(page.locator(selectors.exportMenu)).toContainText("报警记录 CSV");
});
