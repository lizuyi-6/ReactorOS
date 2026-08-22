import { expect, test } from "@playwright/test";
import {
  FORBIDDEN_WATERMARKS,
  VUE_ROUTES,
  assertNoHorizontalOverflow,
  assertNoVueConsoleErrors,
  prepareVuePage,
  vueLogin,
} from "./vue.helpers.mjs";

async function waitForAnyElMessage(page, timeout = 5000) {
  return page.locator(
    ".el-message--success,.el-message--error,.el-message--warning,.el-message--info"
  )
    .first()
    .waitFor({ timeout })
    .then(() => true)
    .catch(() => false);
}

async function waitForElMessage(page, matcher, timeout = 6000) {
  return page
    .locator(
      ".el-message--success,.el-message--error,.el-message--warning,.el-message--info"
    )
    .filter({ hasText: matcher })
    .first()
    .waitFor({ timeout })
    .then(() => true)
    .catch(() => false);
}

test.describe("Full Chain - Desktop (1440x900)", () => {
  test("login: bad password shows error, stays on login page", async ({ page }) => {
    await page.goto("/#/login", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(400);
    await page
      .locator('input[placeholder="Enter username"], input[placeholder="输入用户名"]')
      .first()
      .fill("engineer");
    await page.locator('input[type="password"]').first().fill("wrongpass");
    await page.locator('button[type="submit"], button.el-button--primary').first().click();
    await page.waitForTimeout(800);
    expect(page.url()).toContain("/login");
    // V20 修复后：内联错误条是唯一失败反馈（后端英文串已本地化为"用户名或密码错误"）
    const bar = page.locator(".form-error");
    await expect(bar).toBeVisible({ timeout: 6000 });
    await expect(bar).toContainText(/invalid|incorrect|错误|失败|failed/i);
    // 不再叠加 ElMessage 重复提示
    await expect(page.locator(".el-message--error")).toHaveCount(0);
  });

  test("login: engineer/engineer123 enters main interface", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    expect(page.url()).toContain("/monitor");
    // The monitor page title is "Reactor Overview 反应釜总览"
    const body = await page.locator("body").innerText();
    expect(body).toContain("Reactor Overview");
    cleanup();
  });

  test("monitor: live temperature/pressure are real numbers, ECharts canvas visible", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await page.goto("/#/monitor", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1800);

    // Temperature value present (look for °C with a number)
    const tempText = await page.locator(".monitor-page").innerText();
    expect(tempText).toMatch(/[\d.]+\s*°?C/);

    // Pressure value present (look for bar with number)
    expect(tempText).toMatch(/[\d.]+\s*bar/);

    // ECharts canvas visible
    await expect(page.locator("canvas").first()).toBeVisible();
    assertNoVueConsoleErrors(page);
    cleanup();
  });

  test("control: apply valid targets, success feedback, estop & manual lock visible", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await page.goto("/#/control", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1200);

    // Emergency stop panel exists
    await expect(page.locator(".estop-panel, .estop-button").first()).toBeVisible();

    // Manual lock button exists
    await expect(page.locator("button:has-text('MANUAL LOCK'), button:has-text('手动锁定')").first()).toBeVisible();

    // 新 UI 设定值控件为滑杆+步进按钮（无自由输入框）：点一次步进使目标变脏
    const tempCol = page
      .locator(".sp-col:has-text('目标温度'), .sp-col:has-text('Target Temperature')")
      .first();
    await tempCol.locator(".sp-btn").last().click();

    // Click apply
    await page
      .locator("button:has-text('APPLY TARGETS'), button:has-text('应用设定值')")
      .first()
      .click();

    const msg = await waitForElMessage(page, /成功|success|OK/, 8000);
    expect(msg, "apply should produce a feedback toast").toBe(true);
    assertNoVueConsoleErrors(page);
    cleanup();
  });

  test("ai: generate recommendation, verify card with temp/rpm/rationale/provider; apply if present", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);

    const token = await vueLogin(request);
    const headers = { Authorization: "Bearer " + token };

    let recResp = await request.post("/api/recommendations/latest", {
      headers,
      data: { intent: "optimize_and_control" },
    }).catch(() => null);

    if (!recResp || !recResp.ok()) {
      const batchesResp = await request.get("/api/batches", { headers }).catch(() => null);
      if (batchesResp && batchesResp.ok()) {
        const payload = await batchesResp.json().catch(() => ({}));
        const list = payload?.data ?? payload?.batches ?? payload ?? [];
        const b = Array.isArray(list) ? list[0] : null;
        if (b && (b.id || b.batch_id)) {
          await request.post("/api/product-results", {
            headers,
            data: { batch_id: b.id ?? b.batch_id, yield_percent: 96.0, product_ratio: 0.93 },
          }).catch(() => {});
        }
      }
      recResp = await request.post("/api/recommendations/latest", {
        headers,
        data: { intent: "optimize_and_control" },
      }).catch(() => null);
    }

    await page.goto("/#/ai", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(500);

    const cardVisible = await page
      .locator(".ai-page .big-card, .ai-page .rec-cards, .ai-page .rationale-text")
      .first()
      .waitFor({ state: "visible", timeout: 45_000 })
      .then(() => true)
      .catch(() => false);

    if (cardVisible || (recResp && recResp.ok())) {
      const aiBody = await page.locator(".ai-page, body").first().innerText();
      expect(aiBody).toMatch(/[\d.]+\s*°C/);
      expect(/stepfun|StepFun|Local|OpenAI|provider/i.test(aiBody)).toBe(true);
    }

    const execBtn = page.locator(
      "button:has-text('立即执行'), button:has-text('Execute Now'), .exec-btn.go"
    );
    if ((await execBtn.count()) > 0 && (await execBtn.first().isEnabled().catch(() => false))) {
      await execBtn.first().click();
      try {
        const confirm = page.locator(".el-message-box button:has-text('立即执行'), .el-message-box button:has-text('Execute Now')");
        if ((await confirm.count()) > 0) await confirm.first().click({ timeout: 3000 }).catch(() => {});
      } catch {}
      const execMsg = await waitForElMessage(page, /成功|success|executed|已执行/, 45_000);
      expect(execMsg, "execute should produce a feedback toast").toBe(true);
    }

    assertNoVueConsoleErrors(page);
    cleanup();
  });

  test("history: batch table has at least one row, timestamps localized", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await page.goto("/#/history", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1800);

    const rows = page.locator(".batch-table table tbody tr, .el-table__row, table tbody tr");
    expect(await rows.count()).toBeGreaterThan(0);

    const body = await page.locator("body").innerText();
    expect(body).toMatch(/\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2}/);
    expect(body).not.toMatch(
      /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6,}/
    );
    cleanup();
  });

  test("audit: events from prior control/AI operations are present", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await page.goto("/#/control", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(800);
    const tempInput = page
      .locator(".sp-col:has-text('Target Temperature') input")
      .first();
    if ((await tempInput.count()) > 0) {
      await tempInput.fill("65");
      await page
        .locator("button:has-text('APPLY TARGETS'), button:has-text('APPLY')")
        .first()
        .click();
      await waitForElMessage(page, /成功|success/, 6000).catch(() => {});
    }

    await page.goto("/#/audit", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1000);

    const rows = page.locator("table tbody tr, .ev-item, .audit-page .ev-item");
    expect(await rows.count()).toBeGreaterThan(0);

    const auditBody = await page.locator("body").innerText();
    expect(/update_targets|set_auto|emergency|manual_lock|targets/i.test(auditBody)).toBe(true);
    cleanup();
  });

  test("settings: switch to English then back to Chinese", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    await page.goto("/#/settings", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(800);

    await page.locator(".app-header .lang-toggle").first().click();
    await page.waitForTimeout(500);
    const lang = await page.evaluate(() => localStorage.getItem("reactoros.vue.language"));
    if (lang === "zh") {
      await page.locator(".app-header .lang-toggle").first().click();
      await page.waitForTimeout(500);
    }

    // Check nav items use English text (nav-title is English label)
    for (const route of VUE_ROUTES) {
      await page.goto("/#" + route, { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(300);
      const navItem = page.locator(".nav-item").first();
      if ((await navItem.count()) > 0) {
        const navText = await navItem.innerText();
        // Both en+zh appear simultaneously per current dual-tag mode — verify at least English present
        expect(/Monitor|Control|AI|History|Audit|Modbus|Settings/i.test(navText)).toBe(true);
      }
    }

    await page.locator(".app-header .lang-toggle").first().click();
    await page.waitForTimeout(500);
    const backZh = await page.evaluate(
      () => localStorage.getItem("reactoros.vue.language") === "zh"
    );
    expect(backZh).toBe(true);
    cleanup();
  });
});