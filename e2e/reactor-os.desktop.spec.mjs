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

test.afterEach(async ({ page }) => {
  assertNoConsoleErrors(page);
});

test("production HMI shows JSON error codes instead of fake readings without pipeline data", async ({ page, request }) => {
  await request.post("/api/test/reset");
  const unavailable = await request.get("/api/live");
  expect(unavailable.status()).toBe(503);
  const unavailableBody = await unavailable.json();
  expect(unavailableBody.code).toBe(503);
  expect(unavailableBody.message).toContain("sensor data unavailable");

  const missing = await request.get("/api/does-not-exist");
  expect(missing.status()).toBe(404);
  const missingBody = await missing.json();
  expect(missingBody.code).toBe(404);

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
  await expect(page.locator("body")).toContainText("PIPELINE 503");
  const sensorValues = await page.evaluate(() =>
    Object.fromEntries(window.__reactorState.sensors.map(sensor => [sensor.key, sensor.value]))
  );
  expect(sensorValues.temperature).toBeNull();
  expect(sensorValues.pressure).toBeNull();
  expect(sensorValues.rpm).toBeNull();
});

test("control panel can add a process when live pipeline is unavailable", async ({ page, request }) => {
  await request.post("/api/test/reset");
  const unavailable = await request.get("/api/live");
  expect(unavailable.status()).toBe(503);
  const consoleErrors = [];
  page.on("console", message => {
    if (message.type() !== "error") return;
    const text = message.text();
    if (text.includes("Failed to load resource") && text.includes("503")) return;
    consoleErrors.push(text);
  });
  page.on("pageerror", error => consoleErrors.push(error.message));
  page.consoleErrors = consoleErrors;

  await page.goto("/?v=e2e-process-offline");
  await page.waitForLoadState("domcontentloaded");
  await switchTab(page, selectors.programTab, "view-program");
  await expect(page.locator("#programRoot")).toContainText("工艺接口");
  await expect(page.locator("#programRoot")).toContainText("实时管线");
  await page.locator("#createProcessBtn").click();
  await expect(page.locator("#programRoot")).toContainText("添加下一步");
  await expect(page.locator("#programRoot")).toContainText("此工艺还没有步骤");
  await page.locator("#addProcessStepBtn").click();
  await expect(page.locator("#programRoot")).toContainText("阶段 1");
  const processes = await request.get("/api/processes");
  expect(processes.status()).toBe(200);
  const processBody = await processes.json();
  expect(processBody.code).toBe(0);
  expect(processBody.data.length).toBeGreaterThan(0);
  expect(consoleErrors).toEqual([]);
});

test("HMI dashboard and settings use only backend pipeline data", async ({ page, request }) => {
  await preparePage(page, request);
  await assertNoHorizontalOverflow(page);
  await assertNoTextClipping(page);
  await expect(page.locator(selectors.runState)).toContainText(/Idle|Running|Alarm/);
  await expect(page.locator("#pipelineState")).toContainText("Devices 1/1");
  await expect(page.locator("body")).toContainText("Process Line");
  await expect(page.locator("body")).toContainText("Detector Signals");
  await expect(page.locator("body")).toContainText("Operator Control");

  const live = await latestLive(request);
  expect(live.runtime.latest_sample).toMatchObject(pipelineSample);
  expect(live.latest_recommendation).toBeNull();
  expect(live.recent_samples[0]).toHaveProperty("batch_id");
  expect(Array.isArray(live.alarms)).toBe(true);
  const temp = pipelineSample.temperature_c.toFixed(2);
  const pressure = pipelineSample.pressure_mpa.toFixed(2);
  const rpm = pipelineSample.stirrer_rpm.toFixed(2);
  const shake = pipelineSample.shake_speed_cpm.toFixed(2);
  const flow = pipelineSample.flow_rate_l_min.toFixed(2);
  const concentration = pipelineSample.product_concentration_percent.toFixed(2);
  const ph = pipelineSample.ph.toFixed(2);

  await expect(page.locator(selectors.sideSensors)).toContainText(`${pressure} MPa`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${rpm} RPM`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${shake} CPM`);
  await expect(page.locator(selectors.sideSensors)).toContainText("TILT");
  await expect(page.locator(selectors.sideSensors)).toContainText("deg");
  await expect(page.locator(selectors.sideSensors)).toContainText(`${flow} L/min`);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${concentration} %`);
  await expect(page.locator(selectors.sideSensors)).toContainText(ph);
  await expect(page.locator(selectors.sideSensors)).toContainText(`${temp} °C`);
  await expect(page.locator(selectors.batchSummary)).toContainText("数据来源");
  await expect(page.locator(selectors.batchSummary)).toContainText("后端数据管线");
  await expect(page.locator("body")).not.toContainText("SYN-84A");
  await expect(page.locator("body")).not.toContainText("根据批次");
  await expect(page.locator("body")).not.toContainText("B-202310");
  await assertCanvasHasInk(page, selectors.mainChart);

  await switchTab(page, selectors.programTab, "view-program");
  await expect(page.locator("#programRoot")).toContainText("工艺控制面板");
  await expect(page.locator("#programRoot")).toContainText("后端暂无工艺定义");
  await expect(page.locator("#programRoot")).not.toContainText("应用手动配置");
  await page.locator("#createProcessBtn").click();
  await expect(page.locator("#programRoot")).toContainText("添加下一步");
  await page.locator("#addProcessStepBtn").click();
  await expect(page.locator("#programRoot")).toContainText("阶段 1");
  await page.locator("#addProcessStepBtn").click();
  await expect(page.locator("#programRoot")).toContainText("阶段 2");
  await page.locator("#processStepName").fill("保温确认");
  await page.locator("#saveProcessStepBtn").click();
  await expect(page.locator("#programRoot")).toContainText("保温确认");
  await page.locator("#applyProcessBtn").click();
  await expect(page.locator("#programRoot")).toContainText("工艺控制面板");
  await expect(page.locator("#programRoot")).toContainText("新工艺");
  const liveAfterProcess = await latestLive(request);
  expect(liveAfterProcess.processes.length).toBeGreaterThan(0);
  expect(liveAfterProcess.recent_batches[0].process_id).toBe(liveAfterProcess.processes[0].id);

  await switchTab(page, selectors.recipesTab, "view-recipes");
  await expect(page.locator("#batchTable")).toContainText(/等待后端批次数据|来自后端数据管线/);

  await switchTab(page, selectors.materialsTab, "view-materials");
  await expect(page.locator("#historyRoot")).toContainText("历史数据");
  await expect(page.locator("#historyRoot")).toContainText("查看学习曲线");
  await page.locator("[data-history-open]").first().click();
  await expect(page.locator("#historyRoot")).toContainText("实验拟合曲线");
  await expect(page.locator("#historyClusterReport")).toContainText(/等待结果|产物结果/);
  await assertCanvasHasInk(page, "#historyTrendChart");

  await switchTab(page, selectors.aiTab, "view-ai");
  await expect(page.locator("#aiIntentQuote")).toContainText("等待后端推荐");
  await expect(page.locator("#aiSuggestedCompare")).toContainText("等待后端推荐");

  await switchTab(page, selectors.alarmsTab, "view-alarms");
  await expect(page.locator("#activeAlarmRowsFull")).toContainText("无活动报警");

  await page.locator('.nav-settings[data-tab="settings"]').click();
  await expect(page.locator(selectors.activeView)).toHaveAttribute("id", "view-settings");
  await expect(page.locator("#settingsEndpoints")).toContainText("/api/live");
  await expect(page.locator("#settingsEndpoints")).toContainText("/api/processes");
  await expect(page.locator("#settingsEndpoints")).toContainText("/api/v1/reactor/reactor_001/samples");
  await expect(page.locator("#settingsEndpoints")).toContainText("POST");
  await expect(page.locator("#settingsDevices")).toContainText("reactor_001", { ignoreCase: true });
});
