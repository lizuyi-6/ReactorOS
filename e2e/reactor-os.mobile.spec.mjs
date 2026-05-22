import { expect, test } from "@playwright/test";
import {
  assertNoConsoleErrors,
  assertNoHorizontalOverflow,
  assertNoTextClipping,
  assertResponsiveSingleColumn,
  preparePage,
  selectors
} from "./reactor-os.helpers.mjs";

test.beforeEach(async ({ page, request }) => {
  await preparePage(page, request);
});

test.afterEach(async ({ page }) => {
  assertNoConsoleErrors(page);
});

test("mobile operator flow: responsive layout, emergency stop and reset", async ({ page }) => {
  await assertResponsiveSingleColumn(page);
  await assertNoHorizontalOverflow(page);
  await assertNoTextClipping(page);

  await page.locator(selectors.start).click();
  await expect(page.locator(selectors.systemText)).toContainText("系统运行中");

  await page.locator(selectors.estop).click();
  await expect(page.locator(selectors.systemText)).toContainText("急停已触发");
  await expect(page.locator(selectors.auto)).toContainText("自动控制：关闭");
  await expect(page.locator(selectors.estop)).toBeDisabled();
  await expect(page.locator(selectors.resetEstop)).toBeEnabled();
  await expect(page.locator(selectors.operatorNote)).toContainText("急停已触发");

  await page.locator(selectors.resetEstop).click();
  await expect(page.locator(selectors.systemText)).toContainText("系统运行中");
  await expect(page.locator(selectors.estop)).toBeEnabled();
  await expect(page.locator(selectors.operatorNote)).toContainText("急停状态已复位");

  await page.locator(selectors.stop).click();
  await expect(page.locator(selectors.systemText)).toContainText("系统待机");
});
