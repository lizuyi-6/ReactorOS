import { expect, test } from "@playwright/test";
import {
  assertCanvasHasInk,
  assertNoConsoleErrors,
  assertNoHorizontalOverflow,
  assertNoTextClipping
} from "./reactor-os.helpers.mjs";

const canvases = ["#processCanvas", "#mainTrend", "#stageGantt", "#alarmDist"];
const viewports = [
  { width: 1024, height: 600 },
  { width: 1280, height: 800 },
  { width: 1440, height: 900 }
];

function captureConsoleErrors(page) {
  const consoleErrors = [];
  page.on("console", message => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", error => consoleErrors.push(error.message));
  page.consoleErrors = consoleErrors;
}

test.afterEach(async ({ page }) => {
  assertNoConsoleErrors(page);
});

for (const viewport of viewports) {
  test(`workshop HMI prototype fits ${viewport.width}x${viewport.height}`, async ({ page }) => {
    captureConsoleErrors(page);
    await page.setViewportSize(viewport);
    await page.goto("/workshop.html", { waitUntil: "domcontentloaded" });
    await expect(page.locator("body")).toContainText("WORKSHOP DEMO / 未接入后端");
    await expect(page.locator("[data-sensor]")).toHaveCount(7);

    for (const selector of canvases) {
      await assertCanvasHasInk(page, selector);
    }
    await assertNoHorizontalOverflow(page);
    await assertNoTextClipping(page);
  });
}

test("workshop HMI prototype is static and touch-oriented", async ({ page }) => {
  captureConsoleErrors(page);

  const apiRequests = [];
  page.on("request", request => {
    const url = request.url();
    if (url.includes("/api/") || url.includes("/ws/")) apiRequests.push(url);
  });

  await page.goto("/workshop.html", { waitUntil: "domcontentloaded" });
  await expect(page.locator("body")).toContainText("WORKSHOP DEMO / 未接入后端");
  await expect(page.locator("body")).toContainText("Detector Signals");
  await expect(page.locator("body")).toContainText("Operator Control");
  await expect(page.locator("body")).toContainText("Alarm Queue");
  await expect(page.locator("[data-sensor]")).toHaveCount(7);
  await expect(page.locator("#startBtn")).toHaveCSS("min-height", "56px");
  await expect(page.locator("#estopBtn")).toHaveCSS("min-height", "56px");
  await page.locator("#pauseBtn").click();
  await expect(page.locator("#runText")).toHaveText("PAUSED");
  await page.locator("#startBtn").click();
  await expect(page.locator("#runText")).toHaveText("RUNNING");
  await page.locator("#estopBtn").click();
  await expect(page.locator("#estopBtn")).toHaveText("CONFIRM STOP");
  await page.locator("#estopBtn").click();
  await expect(page.locator("#runText")).toHaveText("E-STOP");

  for (const selector of canvases) {
    await assertCanvasHasInk(page, selector);
  }
  await assertNoHorizontalOverflow(page);
  await assertNoTextClipping(page);
  expect(apiRequests).toEqual([]);
});
