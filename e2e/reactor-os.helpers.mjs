import { expect } from "@playwright/test";

export const selectors = {
  operatorNote: "#operatorNote",
  memorySummary: "#memorySummary",
  systemText: "#systemText",
  sensorText: "#sensorText",
  aiText: "#aiText",
  batchLabel: "#batchLabel",
  targetTemp: "#targetTempInput",
  heatMinutes: "#heatMinutesInput",
  stirMinutes: "#stirMinutesInput",
  stirRpm: "#stirRpmInput",
  yieldInput: "#yieldInput",
  ratioInput: "#ratioInput",
  notesInput: "#notesInput",
  start: "#startBtn",
  stop: "#stopBtn",
  auto: "#autoBtn",
  estop: "#estopBtn",
  resetEstop: "#resetEstopBtn",
  applyRecommended: "#applyRecommended",
  saveResult: "#saveResultBtn",
  eventLog: "#eventLog"
};

export async function preparePage(page, request) {
  await request.post("/api/test/reset");
  const consoleErrors = [];
  page.on("console", message => {
    if (message.type() !== "error") return;
    const text = message.text();
    if (text.includes("the server responded with a status of 400")) return;
    consoleErrors.push(text);
  });
  page.on("pageerror", error => consoleErrors.push(error.message));
  page.consoleErrors = consoleErrors;
  await page.goto("/");
  await page.waitForLoadState("domcontentloaded");
  await expect(page.locator("body")).toContainText("ReactorOS");
  await expect(page.locator(selectors.memorySummary)).toContainText("记忆文件：");
  await expect(page.locator(selectors.sensorText)).not.toContainText("后端离线");
}

export function assertNoConsoleErrors(page) {
  expect(page.consoleErrors, "browser console/page errors").toEqual([]);
}

export async function numericInputValue(page, selector) {
  return Number(await page.locator(selector).inputValue());
}

export async function assertCriticalCopy(page) {
  await expect(page.locator(selectors.systemText)).toContainText(
    /系统运行中|系统待机|急停已触发|参数异常/
  );
  await expect(page.locator(selectors.sensorText)).toContainText(/传感器已连接|等待传感器/);
  await expect(page.locator(selectors.aiText)).toContainText(/AI 引擎就绪|控制待检查/);
  await expect(page.locator(selectors.memorySummary)).toContainText("禁区");
  await expect(page.locator(selectors.operatorNote)).not.toContainText("undefined");
  await expect(page.locator(selectors.operatorNote)).not.toContainText("null");
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
        "button, .pill, .sensor-name, .rec-name, .rec-value, .rec-compare, .dim, .inline-note, th, td"
      )
    );
    for (const el of candidates) {
      const style = getComputedStyle(el);
      if (style.display === "none" || style.visibility === "hidden") continue;
      if (el.scrollWidth > el.clientWidth + 2 || el.scrollHeight > el.clientHeight + 2) {
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

export async function assertConsistentControlStyling(page) {
  const styles = await page.evaluate(() => {
    return ["#startBtn", "#autoBtn", "#estopBtn", "#resetEstopBtn", "#saveResultBtn"].map(
      selector => {
        const el = document.querySelector(selector);
        const style = getComputedStyle(el);
        return {
          selector,
          radius: style.borderRadius,
          shadow: style.boxShadow
        };
      }
    );
  });
  const radii = new Set(styles.map(style => style.radius));
  expect([...radii], `button radius mismatch: ${JSON.stringify(styles)}`).toEqual(["5px"]);
  expect(
    styles.every(style => style.shadow === "none"),
    `unexpected shadows: ${JSON.stringify(styles)}`
  ).toBe(true);
}

export async function assertResponsiveSingleColumn(page) {
  const grid = page.locator(".layout");
  await expect(grid).toBeVisible();
  const columns = await grid.evaluate(el => getComputedStyle(el).gridTemplateColumns.split(" ").length);
  expect(columns).toBe(1);
}
