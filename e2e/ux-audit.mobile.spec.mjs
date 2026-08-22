import { expect, test } from "@playwright/test";
import {
  assertNoHorizontalOverflow,
  prepareVuePage,
  vueLogin,
} from "./vue.helpers.mjs";

const MOBILE_ROUTES = ["monitor", "control", "ai", "history", "audit", "settings"];

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

async function collectOverflowRegions(page) {
  return page.evaluate(() => {
    const excl = ".el-scrollbar__wrap,.el-table__body-wrapper,.cards-scroll,.scrollable,[class*=overflow-auto],.el-table__cell .cell,.el-table__header-wrapper,.topbar-cards,.bg-decor,.bg-grid";
    const results = [];
    function walk(el) {
      for (const child of el.children) {
        const ch = child;
        if (ch.scrollWidth - ch.clientWidth > 5 && !ch.matches(excl))
          results.push((ch.className || ch.tagName).toString().slice(0, 60));
        walk(ch);
      }
    }
    walk(document.body);
    return results;
  });
}

// 数据区选择器：后端/演示数据内容，与 UI 语言无关，不做中文残留判定
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
        if (/\bundefined\b|NaN|\[object Object\]/.test(t)) bad.push(t.slice(0, 80));
        if (/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6,}/.test(t))
          bad.push("RAW-ISO-" + t.slice(0, 60));
        walk(child);
      }
    };
    walk(document.body);
    return bad;
  });
}

async function sampleCardStyles(page) {
  return page.evaluate(() => {
    const cards = document.querySelectorAll(".el-card,.panel-card");
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

test.describe("UX Audit - Mobile (Pixel 5, 393x851)", () => {
  test.beforeEach(async ({ page, request }) => {
    const cleanup = await prepareVuePage(page, request);
    (page).__cleanup = cleanup;
  });

  test.afterEach(async ({ page }) => {
    const c = (page).__cleanup;
    if (c) c();
  });

  for (const route of MOBILE_ROUTES) {
    test(route + " - overflow, no bad tokens, no watermark", async ({ page }) => {
      await navigateMobileTo(page, route);
      await page.waitForTimeout(800);

      await assertNoHorizontalOverflow(page);
      const regions = await collectOverflowRegions(page);
      expect(regions, "overflow on mobile " + route).toEqual([]);

      const bad = await collectBadTexts(page);
      expect(bad.length, "bad tokens on mobile " + route).toBe(0);

      const body = await page.locator("body").innerText();
      expect(body).not.toContain("0 项权限");
      expect(body).not.toMatch(/at \(.+?\)\s+[\w/]+\.\w+\.\w+/);
    });
  }

  test("mobile - no text clipping on critical pages", async ({ page }) => {
    const clipped = [];
    for (const route of ["monitor", "control", "ai"]) {
      await navigateMobileTo(page, route);
      await page.waitForTimeout(600);
      const hits = await page.evaluate(() => {
        const hits = [];
        const excl = ".el-scrollbar__wrap,.el-table__body-wrapper,.cards-scroll,.scrollable,[class*=overflow-auto],.el-table__cell .cell,.el-table__header-wrapper,.topbar-cards,.bg-decor,.bg-grid";
        function walk(el) {
          for (const child of el.children) {
            const ch = child;
            if (/^(svg|canvas|img|br|hr)$/i.test(ch.tagName)) { walk(ch); continue; }
            if (ch.matches(excl)) { walk(ch); continue; }
            const sw = ch.scrollWidth;
            const cw = ch.clientWidth;
            if (sw - cw > 2 && ch.scrollHeight <= ch.clientHeight + 4) {
              hits.push((ch.className || ch.tagName).toString().slice(0, 50));
            }
            walk(ch);
          }
        }
        walk(document.body);
        return hits;
      });
      if (hits.length > 0) clipped.push(route + ": " + hits.length + " clipped [" + hits.slice(0, 8).join(" | ") + "]");
    }
    expect(clipped, "text clipping findings").toEqual([]);
  });

  test("mobile monitor: card radius/shadow <= 2 variants", async ({ page }) => {
    await navigateMobileTo(page, "monitor");
    await page.waitForTimeout(1200);
    const { r, s } = await sampleCardStyles(page);
    expect(r.length, "card radius variants <= 2").toBeLessThanOrEqual(2);
    expect(s.length, "card shadow variants <= 2").toBeLessThanOrEqual(2);
  });

  test("mobile: English mode no Chinese residue (exempt tokens OK)", async ({ page, request }) => {
    // 同桌面版：beforeEach 的 init script 会把语言重置回 zh，须追加后执行的 en 覆盖
    await page.addInitScript(() => localStorage.setItem("reactoros.vue.language", "en"));
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.waitForTimeout(400);

    const exempt = [
      "ReactorOS","Smart Reactor","Edge","PASS","FAIL","OK","LIVE","E-STOP",
      "StepFun","OpenAI","Local","pH","bar","\u00b0C","rpm","cpm","L/min","%",
      // 演示种子数据名是用户数据（与 UI 语言无关）
      "客户演示工艺","温和优化","高搅拌对照",
    ];
    for (const route of MOBILE_ROUTES) {
      await navigateMobileTo(page, route);
      await page.waitForTimeout(300);
      const body = await bodyTextExcludingDataZones(page);
      const hits = [];
      const re = /[\u4e00-\u9fff]{2,}/g;
      for (const m of body.matchAll(re)) {
        if (!exempt.some((t) => m[0].includes(t))) hits.push(m[0]);
      }
      expect(hits.length, "Chinese residue mobile EN " + route).toBeLessThanOrEqual(2);
    }
  });
});
