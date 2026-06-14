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
    // Pressure must show a numeric MPa reading; the prior bug read pressure_kpa
    // (backend never sends it) and displayed "--".
    expect(body, "pressure card must show an MPa value, not '--'").toMatch(/压力\s*\n\s*\d/);
    expect(body).toContain("MPa");
    expect(body, "pressure must not be the empty placeholder").not.toMatch(/压力\s*\n\s*--/);
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

  test("control panel rejects an out-of-range target via the safety gate", async ({ page, request }) => {
    await prepareVuePage(page, request);
    await page.goto("/#/control", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(600);
    const input = page.locator('.el-input-number input, input[type="number"]').first();
    await input.fill("99999");
    const btn = page.locator('.el-button--primary, button:has-text("提交"), button:has-text("下发")').first();
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
    // The write must be refused by a safety interlock — either the out-of-range
    // clamp (400 "exceeds device maximum") or, if a latch happens to be engaged
    // from a prior probe, the latch refusal (409). Both are correct safety
    // refusals; what would be wrong is a 2xx accept of an out-of-range target.
    expect(resp.status(), "out-of-range write must be refused").toBeGreaterThanOrEqual(400);
    const text = await resp.text();
    expect(text).toMatch(/exceeds device maximum|manual lock|emergency stop|safety/);
    // Deliberately NOT asserting no-console-errors: the browser logs the
    // intentional refusal (400/409) as a console error, which is expected.
  });
});
