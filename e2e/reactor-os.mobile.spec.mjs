import { expect, test } from "@playwright/test";
import {
  assertNoConsoleErrors,
  assertNoHorizontalOverflow,
  assertNoTextClipping,
  assertResponsiveLayout,
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

test("mobile layout folds sidebar into horizontal sensor summary", async ({ page }) => {
  await assertResponsiveLayout(page);
  await assertNoHorizontalOverflow(page);
  await assertNoTextClipping(page);

  await expect(page.locator(selectors.sideSensors)).toContainText("TEMP");
  await switchTab(page, selectors.materialsTab, "view-materials");
  await expect(page.locator(selectors.ratioRaw)).toContainText("45.00");
  await switchTab(page, selectors.alarmsTab, "view-alarms");
  await expect(page.locator(selectors.activeAlarmRows)).toContainText(/Acknowledge|No active alarms/);
});
