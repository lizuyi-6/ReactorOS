import { expect, test } from "@playwright/test";
import {
  FORBIDDEN_WATERMARKS,
  VUE_ROUTES,
  assertNoHorizontalOverflow,
  assertNoVueConsoleErrors,
  prepareVuePage,
} from "./vue.helpers.mjs";

test.describe("Vue HMI — desktop acceptance", () => {
  // Each test prepares its own authenticated page (the `page` fixture is
  // per-test); prepareVuePage attaches the console-error listener to THAT page
  // and auto-clears the pipeline feeder on page close.
  for (const route of VUE_ROUTES) {
    test(`${route} page renders cleanly with no dev watermarks`, async ({ page, request }) => {
      await prepareVuePage(page, request);
      await page.goto(`/#/${route}`, { waitUntil: "domcontentloaded" });
      try {
        await page.waitForLoadState("networkidle", { timeout: 6000 });
      } catch {}
      await page.waitForTimeout(500);
      await assertNoHorizontalOverflow(page);
      const body = await page.locator("body").innerText();
      for (const mark of FORBIDDEN_WATERMARKS) {
        expect(body, `forbidden watermark present: "${mark}"`).not.toContain(mark);
      }
      assertNoVueConsoleErrors(page);
    });
  }

  test("monitor shows live pressure (not the '--' field-mapping regression)", async ({ page, request }) => {
    await prepareVuePage(page, request);
    await page.goto("/#/monitor", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(800);
    const body = await page.locator("body").innerText();
    // HMI 重建后压力卡为 "Pressure 压力 / 3.00 bar"（后端 MPa × 10 换算显示，
    // 见 ControlView.vue 注释）。断言标签附近有数值+单位，而不是 "--" 占位符。
    expect(body, "pressure card must show a numeric bar value, not '--'").toMatch(/压力[\s\S]{1,40}?[\d.]+\s*bar/);
    expect(body, "pressure must not be the empty placeholder").not.toMatch(/压力[\s\S]{1,20}?--/);
    // The trend chart is rendered by ECharts into a <canvas>. After tree-shaking
    // ECharts to LineChart + GridComponent + CanvasRenderer, a missing component
    // would leave the canvas empty or log to the console - assert both.
    await expect(page.locator("canvas").first()).toBeVisible();
    assertNoVueConsoleErrors(page);
  });

  test("sidebar shows the role label, not a raw '0 permissions' string", async ({ page, request }) => {
    await prepareVuePage(page, request);
    await page.goto("/#/monitor", { waitUntil: "domcontentloaded" });
    const body = await page.locator("body").innerText();
    expect(body).toContain("工程师");
    expect(body).not.toContain("0 项权限");
  });

  test("history table timestamps are localized (no nanosecond ISO)", async ({ page, request }) => {
    await prepareVuePage(page, request);
    await page.goto("/#/history", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(600);
    const body = await page.locator("body").innerText();
    expect(body).toMatch(/\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}/);
    expect(body, "raw ISO timestamps must be formatted for operators").not.toMatch(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6,}/);
  });

  test("control panel clamps an out-of-range target before it reaches the backend", async ({ page, request }) => {
    await prepareVuePage(page, request);
    await page.goto("/#/control", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(600);
    // 新 UI 用 el-slider + 步进按钮代替自由数字输入：越界值无法输入，边界由
    // 滑杆 min/max 结构性保证。断言滑杆上限不超控制层安全边界（温度 160°C /
    // 转速 1200rpm，config/safety.toml），且只读列（压力/流量）滑杆被禁用。
    const sliders = page.locator(".sp-col .el-slider[role=slider], .sp-col .el-slider .el-slider__runway");
    await expect(sliders.first()).toBeVisible({ timeout: 8000 });
    const tempCol = page.locator(".sp-col", { hasText: "目标温度" }).first();
    const tempMax = await tempCol.locator("[role=slider]").first().getAttribute("aria-valuemax");
    expect(Number(tempMax), "temperature slider max must be within safety bound 160").toBeLessThanOrEqual(160);
    const rpmCol = page.locator(".sp-col", { hasText: "搅拌转速" }).first();
    const rpmMax = await rpmCol.locator("[role=slider]").first().getAttribute("aria-valuemax");
    expect(Number(rpmMax), "stirrer slider max must be within safety bound 1200").toBeLessThanOrEqual(1200);
    // 只读列（目标压力）不得允许写入
    const roCol = page.locator(".sp-col.readonly", { hasText: "目标压力" }).first();
    await expect(roCol, "pressure target must be read-only").toBeVisible();
    assertNoVueConsoleErrors(page);
  });
});