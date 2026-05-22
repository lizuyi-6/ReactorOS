import { expect, test } from "@playwright/test";
import {
  assertConsistentControlStyling,
  assertCriticalCopy,
  assertNoConsoleErrors,
  assertNoHorizontalOverflow,
  assertNoTextClipping,
  numericInputValue,
  preparePage,
  selectors
} from "./reactor-os.helpers.mjs";

test.beforeEach(async ({ page, request }) => {
  await preparePage(page, request);
});

test.afterEach(async ({ page }) => {
  assertNoConsoleErrors(page);
});

test("normal operator flow: apply AI recommendation, start, auto, stop, record result", async ({
  page
}) => {
  await assertNoHorizontalOverflow(page);
  await assertNoTextClipping(page);
  await assertCriticalCopy(page);
  await assertConsistentControlStyling(page);

  const initialTemp = await numericInputValue(page, selectors.targetTemp);
  await page.locator(selectors.applyRecommended).click();
  await expect(page.locator(selectors.eventLog)).toContainText("AI 推荐参数已填入");
  await expect
    .poll(() => numericInputValue(page, selectors.targetTemp))
    .not.toBe(initialTemp);

  await page.locator(selectors.start).click();
  await expect(page.locator(selectors.systemText)).toContainText("系统运行中");
  await expect(page.locator(selectors.batchLabel)).toContainText("Batch #");
  await expect(page.locator(selectors.operatorNote)).toContainText("批次已启动");

  await page.locator(selectors.auto).click();
  await expect(page.locator(selectors.auto)).toContainText("自动控制：开启");
  await expect(page.locator(selectors.operatorNote)).toContainText("自动控制已开启");

  await page.locator(selectors.stop).click();
  await expect(page.locator(selectors.systemText)).toContainText("系统待机");
  await expect(page.locator(selectors.operatorNote)).toContainText("批次已结束");

  await page.locator(selectors.yieldInput).fill("86.7");
  await page.locator(selectors.ratioInput).fill("0.91");
  await page.locator(selectors.notesInput).fill("desktop acceptance normal flow");
  await page.locator(selectors.saveResult).click();
  await expect(page.locator(selectors.operatorNote)).toContainText("结果已录入");
  await expect(page.locator("tbody")).toContainText("86.7%");
  await expect(page.locator(selectors.memorySummary)).toContainText("参考 3 批");
});

test("boundary flow: invalid control input is rejected with concise operator feedback", async ({
  page
}) => {
  await assertNoHorizontalOverflow(page);

  await page.locator(selectors.targetTemp).fill("999");
  await page.locator(selectors.stirRpm).fill("9999");
  await page.keyboard.press("Tab");
  await expect(page.locator(selectors.operatorNote)).toContainText("目标提交失败");
  await expect(page.locator(selectors.operatorNote)).toContainText("target_temp");
  await expect(page.locator(selectors.operatorNote)).not.toContainText("\"data\"");
  await expect(page.locator(selectors.operatorNote)).not.toContainText("{");

  await page.locator(selectors.yieldInput).fill("101");
  await page.locator(selectors.ratioInput).fill("1.5");
  await page.locator(selectors.saveResult).click();
  const note = page.locator(selectors.operatorNote);
  await expect(note).toContainText(/没有已结束的批次可录入结果|结果录入失败/);
});
