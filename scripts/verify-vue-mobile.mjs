// Verify responsive Vue HMI behavior on phone and tablet viewports.
// Outputs:
//   output/playwright/vue-mobile-verification.json
//   output/playwright/vue-mobile-<viewport>-<route>-<lang>.png
import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = process.cwd();
const OUT_DIR = resolve(ROOT, "output/playwright");
mkdirSync(OUT_DIR, { recursive: true });

const VUE_URL = process.env.VUE_URL || "http://127.0.0.1:5173/";
const API_BASE = process.env.E2E_BASE_URL || "http://127.0.0.1:8000";

const VIEWPORTS = [
  {
    name: "phone",
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 3,
    isMobile: true,
    hasTouch: true
  },
  {
    name: "tablet",
    viewport: { width: 820, height: 1180 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true
  }
];

const PAGES = [
  { route: "/monitor", name: "monitor", en: "Realtime Monitor", zh: "实时监控" },
  { route: "/control", name: "control", en: "Process Control", zh: "参数配置" },
  { route: "/ai", name: "ai", en: "AI Decision", zh: "AI 决策" },
  { route: "/history", name: "history", en: "History Data", zh: "历史数据" },
  { route: "/audit", name: "audit", en: "Audit Log", zh: "审计日志" },
  {
    route: "/modbus",
    name: "modbus",
    en: "Modbus Debug",
    zh: "Modbus 调试",
    required: {
      en: ["Integration Surface", "Base inference", "LoRA inference", "PRD LoRA/RK"],
      zh: ["集成接口状态", "基础模型入口", "LoRA 推理闭环", "PRD LoRA/RK 闭环"]
    }
  },
  { route: "/settings", name: "settings", en: "System Settings", zh: "系统配置" }
];

const LANGUAGES = [
  { code: "en", key: "en" },
  { code: "zh", key: "zh" }
];

const RENDER_PLACEHOLDERS = ["[object Object]"];

const result = {
  ok: false,
  url: VUE_URL,
  apiBase: API_BASE,
  viewports: [],
  unexpectedConsoleMessages: []
};

function log(scope, step, status, info = "") {
  // eslint-disable-next-line no-console
  console.log(`[${scope}/${status}] ${step}${info ? " :: " + info : ""}`);
}

async function login(context, page) {
  const res = await context.request.post(`${API_BASE}/api/auth/login`, {
    data: { username: "engineer", password: "engineer123" }
  });
  if (!res.ok()) {
    throw new Error(`engineer login failed: ${res.status()} ${await res.text()}`);
  }
  const body = await res.json();
  const token = body.data?.token ?? body.token;
  if (!token) throw new Error("engineer login did not return a token");
  await page.goto(VUE_URL);
  await page.evaluate((t) => {
    localStorage.setItem("reactoros.vue.auth.token", t);
    localStorage.setItem(
      "reactoros.vue.auth.user",
      JSON.stringify({
        username: "engineer",
        role: "engineer",
        permissions: [
          "view_monitor",
          "view_history",
          "view_audit",
          "export_reports",
          "edit_process",
          "start_stop_process",
          "set_safe_targets",
          "apply_ai_suggestion",
          "emergency_stop",
          "modbus_debug"
        ]
      })
    );
  }, token);
}

async function setLanguage(page, code) {
  await page.evaluate((language) => {
    localStorage.setItem("reactoros.vue.language", language);
  }, code);
  await page.reload();
  await page.waitForLoadState("domcontentloaded");
  await page.waitForTimeout(600);
}

async function clickNavAndWait(page, spec, heading) {
  const link = page.locator(".nav-link", { hasText: heading }).first();
  await link.waitFor({ timeout: 10_000 });
  await link.click();
  await page.locator("h1", { hasText: heading }).waitFor({ timeout: 10_000 });
}

async function overflowSnapshot(page) {
  return page.evaluate(() => {
    const content = document.querySelector(".content") || document.body;
    const stack = document.querySelector(".view-stack") || document.body;
    const doc = document.documentElement;
    return {
      docScroll: doc.scrollWidth,
      docClient: doc.clientWidth,
      contentScroll: content.scrollWidth,
      contentClient: content.clientWidth,
      stackScroll: stack.scrollWidth,
      stackClient: stack.clientWidth
    };
  });
}

function overflowOk(snapshot) {
  return (
    snapshot.docScroll <= snapshot.docClient + 1 &&
    snapshot.contentScroll <= snapshot.contentClient + 1 &&
    snapshot.stackScroll <= snapshot.stackClient + 1
  );
}

async function scrollCheck(page) {
  return page.evaluate(() => {
    const before = window.scrollY;
    const scrollable = document.documentElement.scrollHeight > window.innerHeight + 8;
    window.scrollTo(0, document.documentElement.scrollHeight);
    const after = window.scrollY;
    window.scrollTo(0, before);
    return {
      scrollable,
      before,
      after,
      ok: !scrollable || after > before
    };
  });
}

(async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    for (const viewportSpec of VIEWPORTS) {
      const context = await browser.newContext({
        viewport: viewportSpec.viewport,
        deviceScaleFactor: viewportSpec.deviceScaleFactor,
        isMobile: viewportSpec.isMobile,
        hasTouch: viewportSpec.hasTouch
      });
      const page = await context.newPage();
      const viewportResult = {
        name: viewportSpec.name,
        viewport: viewportSpec.viewport,
        pages: []
      };
      result.viewports.push(viewportResult);

      page.on("console", (message) => {
        if (message.type() === "error") {
          const text = message.text();
          result.unexpectedConsoleMessages.push({
            viewport: viewportSpec.name,
            type: message.type(),
            text
          });
        }
      });
      page.on("pageerror", (error) => {
        result.unexpectedConsoleMessages.push({
          viewport: viewportSpec.name,
          type: "pageerror",
          url: page.url(),
          text: error.stack || error.message
        });
      });

      await login(context, page);

      for (const language of LANGUAGES) {
        await setLanguage(page, language.code);
        for (const spec of PAGES) {
          const heading = spec[language.key];
          const pageResult = {
            route: spec.route,
            name: spec.name,
            language: language.code,
            heading,
            opened: false,
            headingFound: false,
            requiredFound: [],
            requiredMissing: [],
            renderPlaceholdersAbsent: false,
            overflow: null,
            scroll: null,
            screenshot: `output/playwright/vue-mobile-${viewportSpec.name}-${spec.name}-${language.code}.png`,
            ok: false
          };
          viewportResult.pages.push(pageResult);

          try {
            await clickNavAndWait(page, spec, heading);
            pageResult.opened = page.url().includes(`#${spec.route}`);
            const bodyText = await page.locator("body").innerText();
            pageResult.headingFound = bodyText.includes(heading);
            const requiredPhrases = spec.required?.[language.key] || [];
            pageResult.requiredFound = requiredPhrases.filter((phrase) => bodyText.includes(phrase));
            pageResult.requiredMissing = requiredPhrases.filter((phrase) => !bodyText.includes(phrase));
            pageResult.renderPlaceholdersAbsent = !RENDER_PLACEHOLDERS.some((placeholder) => bodyText.includes(placeholder));
            pageResult.overflow = await overflowSnapshot(page);
            pageResult.scroll = await scrollCheck(page);
            await page.screenshot({
              path: resolve(ROOT, pageResult.screenshot),
              fullPage: true
            });
            pageResult.ok =
              pageResult.opened &&
              pageResult.headingFound &&
              pageResult.requiredMissing.length === 0 &&
              pageResult.renderPlaceholdersAbsent &&
              overflowOk(pageResult.overflow) &&
              pageResult.scroll.ok;
            log(
              `${viewportSpec.name}/${language.code}/${spec.name}`,
              "responsive-check",
              pageResult.ok ? "ok" : "fail",
              JSON.stringify({
                opened: pageResult.opened,
                headingFound: pageResult.headingFound,
                requiredMissing: pageResult.requiredMissing,
                renderPlaceholdersAbsent: pageResult.renderPlaceholdersAbsent,
                overflow: pageResult.overflow,
                scroll: pageResult.scroll
              })
            );
          } catch (error) {
            pageResult.error = error instanceof Error ? error.message : String(error);
            log(`${viewportSpec.name}/${language.code}/${spec.name}`, "responsive-check", "fail", pageResult.error);
          }
        }
      }
      await context.close();
    }

    result.ok =
      result.unexpectedConsoleMessages.length === 0 &&
      result.viewports.every((viewport) => viewport.pages.every((page) => page.ok));
    log("summary", "mobile-responsive", result.ok ? "ok" : "fail", JSON.stringify({
      viewportCount: result.viewports.length,
      pageChecks: result.viewports.reduce((count, viewport) => count + viewport.pages.length, 0),
      unexpectedConsoleMessages: result.unexpectedConsoleMessages.length
    }));
  } catch (error) {
    result.error = error instanceof Error ? error.message : String(error);
    log("summary", "error", "fail", result.error);
  } finally {
    await browser.close();
  }

  const outPath = resolve(OUT_DIR, "vue-mobile-verification.json");
  writeFileSync(outPath, JSON.stringify(result, null, 2));
  // eslint-disable-next-line no-console
  console.log(`verification -> ${outPath}`);
  if (!result.ok) process.exit(1);
})();
