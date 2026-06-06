// Verify Vue parity slice across AI/History/Settings/Monitor routes.
// Outputs:
//   output/playwright/vue-parity-verification.json
//   output/playwright/vue-parity-ai-en.png, vue-parity-ai-zh.png
//   output/playwright/vue-parity-history-en.png, vue-parity-history-zh.png
//   output/playwright/vue-parity-settings-en.png, vue-parity-settings-zh.png
//   output/playwright/vue-parity-monitor-en.png, vue-parity-monitor-zh.png
import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = process.cwd();
const OUT_DIR = resolve(ROOT, "output/playwright");
mkdirSync(OUT_DIR, { recursive: true });

const VUE_URL = process.env.VUE_URL || "http://127.0.0.1:5173/";
const API_BASE = process.env.E2E_BASE_URL || "http://127.0.0.1:8000";

const PAGES = [
  {
    route: "/ai",
    name: "ai",
    en: ["AI Decision", "Latest Recommendation Provider", "Recommendation Detail", "AI Master Control", "Dry-run", "Execute", "Experiment Plan", "SOP Draft"],
    zh: ["AI 决策", "最新推荐来源", "推荐内容", "AI 主控", "实验方案", "加载 SOP 草案"]
  },
  {
    route: "/history",
    name: "history",
    en: ["History Data", "Batch Detail", "Export CSV", "Product Outcomes"],
    zh: ["历史数据", "批次详情", "导出 CSV", "产物结果"]
  },
  {
    route: "/settings",
    name: "settings",
    en: ["System Settings", "Storage Security", "Forbidden Zones", "Permission Matrix", "Endpoint Matrix", "Integration Status"],
    zh: ["系统配置", "存储安全", "禁区", "权限矩阵", "端点矩阵", "集成状态"]
  },
  {
    route: "/monitor",
    name: "monitor",
    en: ["Realtime Monitor", "Live Trend", "Alarm Center"],
    zh: ["实时监控", "实时趋势", "报警中心"]
  }
];

const result = {
  ok: false,
  url: VUE_URL,
  apiBase: API_BASE,
  pages: []
};

function log(page, step, status, info) {
  page.steps.push({ step, status, info: info ?? null });
  // eslint-disable-next-line no-console
  console.log(`[${page.name}/${status}] ${step}${info ? " :: " + info : ""}`);
}

async function ensureLoggedIn(context, request, page) {
  const res = await request.post(`${API_BASE}/api/auth/login`, {
    data: { username: "engineer", password: "engineer123" }
  });
  if (!res.ok()) throw new Error(`login failed: ${res.status()}`);
  const body = await res.json();
  const token = body.data?.token ?? body.token;
  // First navigation just to set localStorage; do not let the app mount yet.
  await page.goto(VUE_URL);
  await page.evaluate((t) => {
    localStorage.setItem("reactoros.vue.auth.token", t);
    localStorage.setItem("reactoros.vue.auth.user", JSON.stringify({ username: "engineer", role: "engineer", permissions: ["view_monitor", "view_history", "view_audit", "export_reports", "edit_process", "start_stop_process", "set_safe_targets", "apply_ai_suggestion", "emergency_stop", "modbus_debug"] }));
  }, token);
  return token;
}

async function setLanguageAndReload(page, language) {
  await page.evaluate((lang) => {
    localStorage.setItem("reactoros.vue.language", lang);
  }, language);
  await page.reload();
  await page.waitForLoadState("domcontentloaded");
  // Give Vue + Element Plus time to mount before the next goto/assertion.
  await page.waitForTimeout(800);
}

async function assertNoOverflow(page) {
  return page.evaluate(() => {
    const stack = document.querySelector(".view-stack") || document.body;
    const content = document.querySelector(".content") || document.body;
    return {
      docScroll: document.documentElement.scrollWidth,
      docClient: document.documentElement.clientWidth,
      contentScroll: content.scrollWidth,
      contentClient: content.clientWidth,
      stackScroll: stack.scrollWidth,
      stackClient: stack.clientWidth
    };
  });
}

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  const request = context.request;
  try {
    await ensureLoggedIn(context, request, page);
    for (const spec of PAGES) {
      const pageResult = {
        name: spec.name,
        route: spec.route,
        steps: [],
        englishFound: [],
        chineseFound: [],
        englishMissing: [],
        chineseMissing: [],
        overflowEn: null,
        overflowZh: null,
        openEn: null,
        openZh: null,
        screenshots: {
          en: `output/playwright/vue-parity-${spec.name}-en.png`,
          zh: `output/playwright/vue-parity-${spec.name}-zh.png`
        }
      };
      result.pages.push(pageResult);

      // English
      await setLanguageAndReload(page, "en");
      // Navigate to the page and wait for English heading.
      await page.goto(`${VUE_URL}#${spec.route}`);
      await page.waitForLoadState("domcontentloaded");
      await page.waitForTimeout(800);
      try {
        await page.locator(`h1:has-text("${spec.en[0]}")`).first().waitFor({ timeout: 8_000 });
        pageResult.openEn = "ok";
        log(pageResult, "open-en", "ok", `h1 "${spec.en[0]}" visible`);
      } catch {
        pageResult.openEn = "fail";
        log(pageResult, "open-en", "fail", `h1 "${spec.en[0]}" not visible`);
        // do not return early; we still want to enumerate missing phrases
      }
      await page.waitForTimeout(500);
      const enBody = await page.locator("body").innerText();
      for (const phrase of spec.en) {
        if (enBody.includes(phrase)) pageResult.englishFound.push(phrase);
        else pageResult.englishMissing.push(phrase);
      }
      const enAllOk = pageResult.englishMissing.length === 0;
      log(
        pageResult,
        "english-checks",
        enAllOk ? "ok" : "fail",
        `${pageResult.englishFound.length}/${spec.en.length} missing=[${pageResult.englishMissing.join(",") || "none"}]`
      );
      await page.screenshot({ path: resolve(ROOT, pageResult.screenshots.en), fullPage: true });
      const overflowEn = await assertNoOverflow(page);
      pageResult.overflowEn = {
        ...overflowEn,
        ok:
          overflowEn.docScroll <= overflowEn.docClient + 1 &&
          overflowEn.contentScroll <= overflowEn.contentClient + 1 &&
          overflowEn.stackScroll <= overflowEn.stackClient + 1
      };
      log(pageResult, "overflow-en", pageResult.overflowEn.ok ? "ok" : "fail", JSON.stringify(overflowEn));

      // Chinese
      await setLanguageAndReload(page, "zh");
      await page.goto(`${VUE_URL}#${spec.route}`);
      await page.waitForLoadState("domcontentloaded");
      await page.waitForTimeout(800);
      try {
        await page.locator(`h1:has-text("${spec.zh[0]}")`).first().waitFor({ timeout: 8_000 });
        pageResult.openZh = "ok";
        log(pageResult, "open-zh", "ok", `h1 "${spec.zh[0]}" visible`);
      } catch {
        pageResult.openZh = "fail";
        log(pageResult, "open-zh", "fail", `h1 "${spec.zh[0]}" not visible`);
      }
      await page.waitForTimeout(500);
      const zhBody = await page.locator("body").innerText();
      for (const phrase of spec.zh) {
        if (zhBody.includes(phrase)) pageResult.chineseFound.push(phrase);
        else pageResult.chineseMissing.push(phrase);
      }
      const zhAllOk = pageResult.chineseMissing.length === 0;
      log(
        pageResult,
        "chinese-checks",
        zhAllOk ? "ok" : "fail",
        `${pageResult.chineseFound.length}/${spec.zh.length} missing=[${pageResult.chineseMissing.join(",") || "none"}]`
      );
      await page.screenshot({ path: resolve(ROOT, pageResult.screenshots.zh), fullPage: true });
      const overflowZh = await assertNoOverflow(page);
      pageResult.overflowZh = {
        ...overflowZh,
        ok:
          overflowZh.docScroll <= overflowZh.docClient + 1 &&
          overflowZh.contentScroll <= overflowZh.contentClient + 1 &&
          overflowZh.stackScroll <= overflowZh.stackClient + 1
      };
      log(pageResult, "overflow-zh", pageResult.overflowZh.ok ? "ok" : "fail", JSON.stringify(overflowZh));
    }

    // Strict pass criteria: every page must open both languages, every required
    // phrase must be present in both languages, no horizontal overflow.
    result.ok = result.pages.every(
      (p) =>
        p.openEn === "ok" &&
        p.openZh === "ok" &&
        p.englishMissing.length === 0 &&
        p.chineseMissing.length === 0 &&
        p.overflowEn?.ok === true &&
        p.overflowZh?.ok === true
    );
    log({ name: "summary", steps: [] }, "summary", result.ok ? "ok" : "fail", "");
  } catch (error) {
    result.error = error instanceof Error ? error.message : String(error);
    // eslint-disable-next-line no-console
    console.log(`[fail] error :: ${result.error}`);
  } finally {
    await context.close();
    await browser.close();
  }
  const outPath = resolve(OUT_DIR, "vue-parity-verification.json");
  writeFileSync(outPath, JSON.stringify(result, null, 2));
  // eslint-disable-next-line no-console
  console.log(`verification -> ${outPath}`);
  if (!result.ok) process.exit(1);
})();
