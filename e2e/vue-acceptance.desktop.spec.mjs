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
    // Pressure card layout after the HMI rebuild: "釜内压力 / ● LIVE / 0.500 / MPa".
    // Assert a numeric MPa reading appears after the label (not the "--" placeholder);
    // the gap is bounded so this only matches within the pressure card.
    expect(body, "pressure card must show an MPa value, not '--'").toMatch(/釜内压力[\s\S]{1,60}?[\d.]+\s*MPa/);
    expect(body, "pressure must not be the empty placeholder").not.toMatch(/釜内压力[\s\S]{1,60}?--/);
    // The trend chart is rendered by ECharts into a <canvas>. After tree-shaking
    // ECharts to LineChart + GridComponent + CanvasRenderer, a missing component
    // would leave the canvas empty or log to the console - assert both.
    await expect(page.locator("canvas")).toBeVisible();
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
    const input = page.locator('.el-input-number input, input[type="number"]').first();
    await input.fill("99999");
    const btn = page.locator('button:has-text("写入"), button:has-text("Write")').first();
    const [resp] = await Promise.all([
      page
        .waitForResponse(
          (r) => r.url().includes("/api/") && ["POST", "PUT"].includes(r.request().method()),
          { timeout: 5000 }
        )
        .catch(() => null),
      btn.click({ timeout: 2000 }).catch(() => {}),
    ]);
    expect(resp, "a write request must reach the backend").not.toBeNull();
    // The rebuilt UI's el-input-number clamps to the safety max before
    // submitting, so 99999 must NEVER be sent as-is. Assert the committed
    // temperature_c is within a sane reactor range (well below 99999).
    // The backend safety gate (out-of-range refusal) is covered by api_tests.rs.
    const sentBody = resp.request().postData() ?? "";
    const tempMatch = sentBody.match(/"temperature_c"\s*:\s*([\d.]+)/);
    expect(tempMatch, "request body must carry a temperature_c field").not.toBeNull();
    const committedTemp = Number(tempMatch[1]);
    expect(committedTemp, "out-of-range input must be clamped before submit, not sent as 99999").toBeLessThan(1000);
    expect(committedTemp, "clamped value must be within reactor safety range").toBeLessThanOrEqual(300);
  });
});
