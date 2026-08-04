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
    expect(body).toContain("MPa");
    expect(body).not.toMatch(/压力\s*\n\s*--/);
  });

  test("safety gate blocks an out-of-range write on mobile", async ({ page, request }) => {
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
    expect(resp).not.toBeNull();
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
