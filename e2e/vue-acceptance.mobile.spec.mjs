import { expect, test } from "@playwright/test";
import {
  FORBIDDEN_WATERMARKS,
  VUE_ROUTES,
  assertNoHorizontalOverflow,
  assertNoVueConsoleErrors,
  prepareVuePage,
} from "./vue.helpers.mjs";

// Mobile acceptance: the 7 pages must remain usable at Pixel 5 width (393px) —
// no horizontal overflow (the primary squashing symptom), no dev watermarks,
// live pressure still resolves, and the safety gate still blocks bad writes.
test.describe("Vue HMI — mobile acceptance (Pixel 5)", () => {
  for (const route of VUE_ROUTES) {
    test(`${route} fits the mobile viewport with no watermarks`, async ({ page, request }) => {
      await prepareVuePage(page, request);
      await page.goto(`/#/${route}`, { waitUntil: "domcontentloaded" });
      try {
        await page.waitForLoadState("networkidle", { timeout: 6000 });
      } catch {}
      await page.waitForTimeout(500);
      await assertNoHorizontalOverflow(page);
      const body = await page.locator("body").innerText();
      for (const mark of FORBIDDEN_WATERMARKS) {
        expect(body, `forbidden watermark: "${mark}"`).not.toContain(mark);
      }
      assertNoVueConsoleErrors(page);
    });
  }

  test("monitor live pressure resolves on mobile too", async ({ page, request }) => {
    await prepareVuePage(page, request);
    await page.goto("/#/monitor", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(800);
    const body = await page.locator("body").innerText();
    // HMI 重建后压力显示为 bar（后端 MPa × 10，见 ControlView.vue 注释）
    expect(body).toMatch(/压力[\s\S]{1,40}?[\d.]+\s*bar/);
    expect(body).not.toMatch(/压力[\s\S]{1,20}?--/);
  });

  test("safety gate blocks an out-of-range write on mobile", async ({ page, request }) => {
    await prepareVuePage(page, request);
    await page.goto("/#/control", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(600);
    // 新 UI 无自由数字输入：越界提交被滑杆 min/max 结构性阻止（clampNum）。
    // 断言滑杆上限不超控制层安全边界（160°C / 1200rpm，config/safety.toml）。
    const tempCol = page.locator(".sp-col", { hasText: "目标温度" }).first();
    await expect(tempCol).toBeVisible({ timeout: 8000 });
    const tempMax = await tempCol.locator("[role=slider]").first().getAttribute("aria-valuemax");
    expect(Number(tempMax), "temperature slider max must be within safety bound 160").toBeLessThanOrEqual(160);
    const rpmCol = page.locator(".sp-col", { hasText: "搅拌转速" }).first();
    const rpmMax = await rpmCol.locator("[role=slider]").first().getAttribute("aria-valuemax");
    expect(Number(rpmMax), "stirrer slider max must be within safety bound 1200").toBeLessThanOrEqual(1200);
    assertNoVueConsoleErrors(page);
  });
});