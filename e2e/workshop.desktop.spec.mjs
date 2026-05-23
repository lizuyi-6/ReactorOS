import { expect, test } from "@playwright/test";
import {
  assertNoConsoleErrors,
  assertNoHorizontalOverflow,
  assertNoTextClipping
} from "./reactor-os.helpers.mjs";

function captureConsoleErrors(page) {
  const consoleErrors = [];
  page.on("console", message => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", error => consoleErrors.push(error.message));
  page.consoleErrors = consoleErrors;
}

test.afterEach(async ({ page }) => {
  assertNoConsoleErrors(page);
});

test("legacy workshop entry no longer serves local demo sensor data", async ({ page }) => {
  captureConsoleErrors(page);
  const apiRequests = [];
  page.on("request", request => {
    const url = request.url();
    if (url.includes("/api/") || url.includes("/ws/")) apiRequests.push(url);
  });

  await page.goto("/workshop.html", { waitUntil: "domcontentloaded" });
  await expect(page.locator("body")).toContainText("旧演示入口已停用");
  await expect(page.locator("body")).toContainText("后端数据管线");
  await expect(page.locator("body")).toContainText("NO LOCAL DEMO DATA");
  await expect(page.locator("body")).not.toContainText("WORKSHOP DEMO");
  await expect(page.locator("[data-sensor]")).toHaveCount(0);
  await expect(page.locator("canvas")).toHaveCount(0);
  await expect(page.locator("a[href='/']")).toContainText("进入真实数据管线 HMI");
  await assertNoHorizontalOverflow(page);
  await assertNoTextClipping(page);
  expect(apiRequests).toEqual([]);
});
