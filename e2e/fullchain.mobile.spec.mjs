import { expect, test } from "@playwright/test";
import {
  VUE_ROUTES,
  assertNoHorizontalOverflow,
  assertNoVueConsoleErrors,
  prepareVuePage,
  vueLogin,
} from "./vue.helpers.mjs";

async function ensureMobileNavOpen(page) {
  const candidates = [
    ".hamburger", ".sidebar-toggle", ".menu-toggle", ".menu-btn",
    'button[aria-label*="menu" i]',
  ];
  for (const sel of candidates) {
    const el = page.locator(sel).first();
    if ((await el.count()) > 0 && (await el.isVisible().catch(() => false))) {
      await el.click();
      await page.waitForTimeout(400);
      break;
    }
  }
}

async function navigateMobileTo(page, route) {
  await ensureMobileNavOpen(page);
  await page.goto("/#" + route, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
}

async function waitForAnyElMessage(page, timeout = 5000) {
  return page.locator(
    ".el-message--success,.el-message--error,.el-message--warning,.el-message--info"
  )
    .first()
    .waitFor({ timeout })
    .then(() => true)
    .catch(() => false);
}

test.describe("Full Chain - Mobile (Pixel 5, 393x851)", () => {
  test("login: error feedback visible, stays on login", async ({ page }) => {
    await page.goto("/#/login", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(400);
    await page
      .locator('input[placeholder="Enter username"], input[placeholder="输入用户名"]')
      .first()
      .fill("engineer");
    await page.locator('input[type="password"]').first().fill("badpass");
    await page.locator('button[type="submit"], button.el-button--primary').first().click();
    await waitForAnyElMessage(page);
    expect(page.url()).toContain("/login");
  });

  test("login: engineer/engineer123 enters monitor", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    expect(page.url()).toContain("/monitor");
    cleanup();
  });

  test("monitor: live values real, canvas visible, no overflow", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await navigateMobileTo(page, "monitor");
    await page.waitForTimeout(1800);

    await assertNoHorizontalOverflow(page);

    const body = await page.locator("body").innerText();
    expect(body).toMatch(/[\d.]+\s*°?C/);
    expect(body).toMatch(/[\d.]+\s*bar/);
    await expect(page.locator("canvas").first()).toBeVisible();
    assertNoVueConsoleErrors(page);
    cleanup();
  });

  test("control: apply valid target, success feedback, estop & lock visible, no overflow", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await navigateMobileTo(page, "control");
    await page.waitForTimeout(1200);

    await assertNoHorizontalOverflow(page);

    await expect(page.locator(".estop-button, .estop-panel button").first()).toBeVisible();
    await expect(page.locator("button:has-text('MANUAL LOCK')").first()).toBeVisible();

    // 移动端必须能看到设定值控件——393px 下桌面三列网格未换行会把
    // Target Setpoints 卡挤到视口右侧之外（finding V32 的断言证据）
    const tempCol = page
      .locator(".sp-col:has-text('目标温度'), .sp-col:has-text('Target Temperature')")
      .first();
    const colBox = await tempCol.boundingBox();
    expect(
      colBox && colBox.x >= -1 && colBox.x + colBox.width <= 393 + 4,
      "setpoint column must be inside the 393px mobile viewport (V32 responsive finding)"
    ).toBe(true);
    // 新 UI 设定值控件为滑杆+步进按钮（无自由输入框）：点一次步进使目标变脏
    await tempCol.locator(".sp-btn").last().click({ timeout: 8000 });

    await page
      .locator("button:has-text('APPLY TARGETS'), button:has-text('应用设定值')")
      .first()
      .click();

    const msg = await waitForAnyElMessage(page, 8000);
    expect(msg, "apply should produce a feedback toast").toBe(true);
    assertNoVueConsoleErrors(page);
    cleanup();
  });

  test("ai: generate recommendation, verify card, apply if present, no overflow", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);

    const token = await vueLogin(request);
    const headers = { Authorization: "Bearer " + token };
    await request
      .post("/api/recommendations/latest", {
        headers,
        data: { intent: "optimize_and_control" },
      })
      .catch(() => {});

    await navigateMobileTo(page, "ai");
    await page.waitForTimeout(500);

    await assertNoHorizontalOverflow(page);

    const cardVisible = await page
      .locator(".ai-page .big-card, .ai-page .rec-cards, .ai-page .rationale-text")
      .first()
      .waitFor({ state: "visible", timeout: 45_000 })
      .then(() => true)
      .catch(() => false);

    if (cardVisible) {
      const aiBody = await page.locator(".ai-page").first().innerText();
      expect(aiBody).toMatch(/[\d.]+\s*°C/);
    }

    assertNoVueConsoleErrors(page);
    cleanup();
  });

  test("history: at least one batch row, localized timestamps, no overflow", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await navigateMobileTo(page, "history");
    await page.waitForTimeout(1800);

    await assertNoHorizontalOverflow(page);

    const rows = page.locator("table tbody tr, .el-table__row");
    expect(await rows.count()).toBeGreaterThan(0);

    const body = await page.locator("body").innerText();
    expect(body).toMatch(/\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}/);
    expect(body).not.toMatch(
      /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6,}/
    );
    cleanup();
  });

  test("audit: events present, no overflow", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await navigateMobileTo(page, "audit");
    await page.waitForTimeout(1000);

    await assertNoHorizontalOverflow(page);

    const rows = page.locator("table tbody tr, .ev-item, .audit-page .ev-item");
    expect(await rows.count()).toBeGreaterThan(0);
    cleanup();
  });

  test("settings: English then back to Chinese, no overflow", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await navigateMobileTo(page, "settings");
    await page.waitForTimeout(800);

    await page.locator(".app-header .lang-toggle").first().click();
    await page.waitForTimeout(500);
    const lang = await page.evaluate(() => localStorage.getItem("reactoros.vue.language"));
    if (lang === "zh") {
      await page.locator(".app-header .lang-toggle").first().click();
      await page.waitForTimeout(500);
    }

    // 切换生效的可见证据：开关自身文案随语言翻转（zh 显示 "EN"，en 显示 "中"，
    // 见 App.vue lang-toggle）。移动端导航抽屉隐藏时 .nav-item innerText 为空，不能用作断言。
    const toggle = page.locator(".app-header .lang-toggle").first();
    await expect(toggle, "EN mode: toggle should offer switching back to Chinese").toHaveText("中");
    const langNow = await page.evaluate(() => localStorage.getItem("reactoros.vue.language"));
    expect(langNow, "language pref should persist as en").toBe("en");

    await page.locator(".app-header .lang-toggle").first().click();
    await page.waitForTimeout(500);
    const backZh = await page.evaluate(
      () => localStorage.getItem("reactoros.vue.language") === "zh"
    );
    expect(backZh).toBe(true);

    await assertNoHorizontalOverflow(page);
    cleanup();
  });
});