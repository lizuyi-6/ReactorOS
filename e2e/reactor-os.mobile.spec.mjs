import { expect, test } from "@playwright/test";
import {
  assertNoConsoleErrors,
  assertNoHorizontalOverflow,
  assertNoTextClipping,
  assertResponsiveLayout,
  preparePage,
  selectors,
  switchTab,
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
  await switchTab(page, selectors.programTab, "view-program");
  await expect(page.locator("#programRoot")).toContainText("Process Control Panel");
  await expect(page.locator("#programRoot")).not.toContainText("Apply manual configuration");
  await switchTab(page, selectors.materialsTab, "view-materials");
  await expect(page.locator("#historyRoot")).toContainText("History Data");
  await expect(page.locator("#historyRoot")).toContainText(
    /No experiment batches|fitted curves|Archived by real experiment batch/,
  );
  await switchTab(page, selectors.alarmsTab, "view-alarms");
  await expect(page.locator("#activeAlarmRowsFull")).toContainText("No active alarms");
  await page.locator('.nav-settings[data-tab="settings"]').click();
  await expect(page.locator(selectors.activeView)).toHaveAttribute("id", "view-settings");
  await expect(page.locator("#settingsEndpoints")).toContainText(/api\/live/i);
});
