import { expect, test } from "@playwright/test";
import {
  FORBIDDEN_WATERMARKS,
  assertNoHorizontalOverflow,
  prepareVuePage,
  vueLogin,
} from "./vue.helpers.mjs";

const AUDIT_ROUTES = [
  "monitor", "control", "ai", "history", "audit", "modbus", "settings",
];

async function collectOverflowRegions(page) {
  return page.evaluate(() => {
    const scrollSel = ".el-scrollbar__wrap,.el-table__body-wrapper,.cards-scroll,.scrollable,[class*=overflow-auto],.el-table__cell .cell,.el-table__header-wrapper,.topbar-cards,.bg-decor,.bg-grid";
    const results = [];
    function walk(el) {
      for (const child of el.children) {
        const ch = child;
        if (ch.scrollWidth - ch.clientWidth > 5 && !ch.matches(scrollSel)) {
          results.push((ch.className || ch.tagName).toString().slice(0, 60));
        }
        walk(ch);
      }
    }
    walk(document.body);
    return results;
  });
}

// 数据区选择器：后端/演示数据内容（推荐语、表格单元格、事件原因、对比矩阵、批次名）
// 与 UI 语言无关，不做中文残留判定
const DATA_ZONES = ".rationale-text,.el-table .cell,.ev-text,.comp-cell,.proc-name,.kv-cell .v,.d-v,.recipe-name";

async function bodyTextExcludingDataZones(page) {
  return page.evaluate((dz) => {
    let acc = "";
    (function walk(el) {
      for (const ch of el.children) {
        if (ch.matches && ch.matches(dz)) continue;
        // display:none（如 EN 模式隐藏的 .zh）的元素 innerText 会回退为 textContent，必须显式跳过
        const cs = getComputedStyle(ch);
        if (cs.display === "none" || cs.visibility === "hidden") continue;
        if (ch.children && ch.children.length) walk(ch);
        else acc += " " + (ch.textContent || "");
      }
    })(document.body);
    return acc;
  }, DATA_ZONES);
}

async function collectBadTexts(page) {
  return page.evaluate(() => {
    const bad = [];
    function walk(el) {
      for (const child of el.children) {
        const tag = child.tagName;
        if (["SCRIPT","STYLE","NOSCRIPT","CODE"].includes(tag)) { walk(child); continue; }
        const t = (child.textContent || "").trim();
        if (/\bundefined\b|NaN|\[object Object\]/.test(t)) bad.push(t.slice(0, 100));
        if (/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6,}/.test(t))
          bad.push("RAW-ISO-" + t.slice(0, 60));
        walk(child);
      }
    }
    walk(document.body);
    return bad;
  });
}

async function sampleCardStyles(page) {
  return page.evaluate(() => {
    const cards = document.querySelectorAll(".el-card,.panel-card,.monitor-card,[class*=card]:not(.el-card__body)");
    const rSet = new Set();
    const sSet = new Set();
    for (let i = 0; i < Math.min(cards.length, 10); i++) {
      const cs = getComputedStyle(cards[i]);
      rSet.add(cs.borderRadius);
      sSet.add(cs.boxShadow);
    }
    return { r: [...rSet], s: [...sSet] };
  });
}

async function samplePrimaryColors(page) {
  return page.evaluate(() => {
    const btns = document.querySelectorAll(".el-button--primary,.big-btn.go,.el-button--success,.exec-btn.go,button.login-submit");
    const seen = new Map();
    for (let i = 0; i < Math.min(btns.length, 12); i++) {
      const bg = getComputedStyle(btns[i]).backgroundColor;
      seen.set(bg, (seen.get(bg) || 0) + 1);
    }
    return [...seen.entries()];
  });
}

test.describe("UX Audit - Desktop (1440x900)", () => {
  test.beforeEach(async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    (page).__cleanup = cleanup;
  });

  test.afterEach(async ({ page }) => {
    const c = (page).__cleanup;
    if (c) c();
  });

  for (const route of AUDIT_ROUTES) {
    test(route + " - overflow, no bad tokens, no watermarks", async ({ page }) => {
      await page.goto("/#" + route, { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(800);

      await assertNoHorizontalOverflow(page);
      const regions = await collectOverflowRegions(page);
      expect(regions, "overflow regions on " + route).toEqual([]);

      const bad = await collectBadTexts(page);
      expect(bad.length, "bad tokens on " + route).toBe(0);

      const body = await page.locator("body").innerText();
      for (const mark of FORBIDDEN_WATERMARKS) {
        expect(body, "watermark '" + mark + "'").not.toContain(mark);
      }
      expect(body).not.toContain("0 项权限");
      expect(body).not.toMatch(/at \(.+?\)\s+[\w/]+\.\w+\.\w+/);
    });
  }

  test("monitor: card radius/shadow variants <= 2, primary button colors <= 2", async ({ page }) => {
    await page.goto("/#/monitor", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1200);

    const { r, s } = await sampleCardStyles(page);
    expect(r.length, "card border-radius variants should be <= 2").toBeLessThanOrEqual(2);
    expect(s.length, "card box-shadow variants should be <= 2").toBeLessThanOrEqual(2);

    // monitor 页可能没有主按钮；若无则到 settings（保存并应用为 el-button--primary）采样
    let btnColors = await samplePrimaryColors(page);
    if (btnColors.length === 0) {
      await page.goto("/#/settings", { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(1000);
      btnColors = await samplePrimaryColors(page);
    }
    expect(btnColors.length, "primary button color variants <= 2").toBeLessThanOrEqual(2);

    const hasBrandBlue = btnColors.some(([bg]) =>
      /47,\s*155,\s*255|2f9bff/i.test(bg)
    );
    expect(hasBrandBlue, "primary should use brand blue #2f9bff").toBe(true);
  });

  test("modbus: wide content does not cause page-level horizontal scroll", async ({ page }) => {
    await page.goto("/#/modbus", { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1000);

    await assertNoHorizontalOverflow(page);

    const tableContainer = page.locator(".el-table__body-wrapper, .el-table");
    if ((await tableContainer.count()) > 0) {
      const hasInnerScroll = await tableContainer
        .first()
        .evaluate((el) => el.scrollWidth > el.clientWidth + 5);
      if (hasInnerScroll) {
        const pageOverflow = await page.evaluate(
          () => document.documentElement.scrollWidth - document.documentElement.clientWidth
        );
        expect(pageOverflow, "page should not overflow even with wide table").toBeLessThanOrEqual(5);
      }
    }
  });

  test("all pages: English mode no Chinese residue", async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    // 注意：prepareVuePage 的 addInitScript 每次导航都把语言重置为 zh，
    // 必须追加一个后执行的 init script 覆盖为 en（直接 setItem+reload 会被覆盖回去）
    await page.addInitScript(() => localStorage.setItem("reactoros.vue.language", "en"));
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.waitForTimeout(500);

    const exempt = [
      "ReactorOS","Smart Reactor","Edge","PASS","FAIL","OK","NO DATA","LIVE","E-STOP",
      "StepFun","OpenAI","Local","pH","bar","\u00b0C","rpm","cpm","L/min","%",
      // 演示种子数据名是用户数据（与 UI 语言无关）
      "客户演示工艺","温和优化","高搅拌对照",
    ];

    for (const route of AUDIT_ROUTES) {
      await page.goto("/#" + route, { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(400);
      const body = await bodyTextExcludingDataZones(page);
      const zhHits = [];
      const re = /[\u4e00-\u9fff]{2,}/g;
      for (const m of body.matchAll(re)) {
        if (!exempt.some((t) => m[0].includes(t))) zhHits.push(m[0]);
      }
      expect(
        zhHits.length,
        "Chinese residue on EN " + route + ": " + zhHits.join(", ")
      ).toBeLessThanOrEqual(2);
    }
    cleanup();
  });
});