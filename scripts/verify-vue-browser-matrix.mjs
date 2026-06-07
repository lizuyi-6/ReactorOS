// Verify Vue HMI routing, bilingual headings, and layout health across available Playwright browsers.
// Outputs:
//   output/playwright/vue-browser-matrix-verification.json
//   output/playwright/vue-browser-matrix-<browser>-<route>-<lang>.png
import { chromium, firefox, webkit } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = process.cwd();
const OUT_DIR = resolve(ROOT, "output/playwright");
mkdirSync(OUT_DIR, { recursive: true });

const VUE_URL = process.env.VUE_URL || "http://127.0.0.1:5173/";
const API_BASE = process.env.E2E_BASE_URL || "http://127.0.0.1:8000";
const STRICT_ALL_BROWSERS = process.env.PLAYWRIGHT_BROWSER_MATRIX_STRICT === "1";
const CAPTURE_SCREENSHOTS = process.env.PLAYWRIGHT_BROWSER_MATRIX_SCREENSHOTS !== "0";

const BROWSERS = [
  { name: "chromium", engine: "chromium", launcher: chromium },
  { name: "chrome", engine: "chromium", channel: "chrome", launcher: chromium },
  { name: "msedge", engine: "chromium", channel: "msedge", launcher: chromium },
  { name: "firefox", engine: "firefox", launcher: firefox },
  { name: "webkit", engine: "webkit", launcher: webkit }
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

const RENDER_PLACEHOLDERS = ["[object Object]", "{{"];

const result = {
  ok: false,
  url: VUE_URL,
  apiBase: API_BASE,
  strictAllBrowsers: STRICT_ALL_BROWSERS,
  captureScreenshots: CAPTURE_SCREENSHOTS,
  browsers: [],
  unexpectedConsoleMessages: []
};

function log(scope, step, status, info = "") {
  // eslint-disable-next-line no-console
  console.log(`[${scope}/${status}] ${step}${info ? " :: " + info : ""}`);
}

function normalizeSkipReason(error) {
  const text = error instanceof Error ? error.message : String(error);
  return text.split("\n").map((line) => line.trim()).filter(Boolean).slice(0, 4).join(" | ");
}

function isBrowserEnvironmentFailure(error) {
  const text = error instanceof Error ? error.message : String(error);
  return (
    text.includes("Executable doesn't exist") ||
    text.includes("Host system is missing dependencies") ||
    text.includes("browserType.launch") ||
    text.includes("browserContext.newPage") ||
    text.includes("_page")
  );
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
  await page.evaluate((authToken) => {
    localStorage.setItem("reactoros.vue.auth.token", authToken);
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
  await page.waitForTimeout(500);
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

async function verifyBrowser(browserSpec) {
  const browserResult = {
    name: browserSpec.name,
    engine: browserSpec.engine,
    channel: browserSpec.channel ?? null,
    status: "pending",
    viewport: { width: 1366, height: 768 },
    pages: [],
    skipped: false,
    skipReason: null,
    ok: false
  };
  result.browsers.push(browserResult);

  let browser;
  try {
    browser = await browserSpec.launcher.launch({
      headless: true,
      ...(browserSpec.channel ? { channel: browserSpec.channel } : {})
    });
  } catch (error) {
    browserResult.status = "skipped";
    browserResult.skipped = true;
    browserResult.skipReason = normalizeSkipReason(error);
    browserResult.ok = !STRICT_ALL_BROWSERS;
    log(browserSpec.name, "launch", "skipped", browserResult.skipReason);
    return;
  }

  try {
    const context = await browser.newContext({
      viewport: browserResult.viewport
    });
    const page = await context.newPage();

    page.on("console", (message) => {
      if (message.type() === "error") {
        result.unexpectedConsoleMessages.push({
          browser: browserSpec.name,
          type: message.type(),
          url: page.url(),
          text: message.text()
        });
      }
    });
    page.on("pageerror", (error) => {
      result.unexpectedConsoleMessages.push({
        browser: browserSpec.name,
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
          screenshot: CAPTURE_SCREENSHOTS
            ? `output/playwright/vue-browser-matrix-${browserSpec.name}-${spec.name}-${language.code}.png`
            : null,
          ok: false
        };
        browserResult.pages.push(pageResult);

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
          if (CAPTURE_SCREENSHOTS && pageResult.screenshot) {
            await page.screenshot({
              path: resolve(ROOT, pageResult.screenshot),
              fullPage: true
            });
          }
          pageResult.ok =
            pageResult.opened &&
            pageResult.headingFound &&
            pageResult.requiredMissing.length === 0 &&
            pageResult.renderPlaceholdersAbsent &&
            overflowOk(pageResult.overflow);
          log(
            `${browserSpec.name}/${language.code}/${spec.name}`,
            "browser-check",
            pageResult.ok ? "ok" : "fail",
            JSON.stringify({
              opened: pageResult.opened,
              headingFound: pageResult.headingFound,
              requiredMissing: pageResult.requiredMissing,
              renderPlaceholdersAbsent: pageResult.renderPlaceholdersAbsent,
              overflow: pageResult.overflow
            })
          );
        } catch (error) {
          pageResult.error = error instanceof Error ? error.message : String(error);
          log(`${browserSpec.name}/${language.code}/${spec.name}`, "browser-check", "fail", pageResult.error);
        }
      }
    }

    await context.close();
    browserResult.status = browserResult.pages.every((pageResult) => pageResult.ok) ? "ok" : "fail";
    browserResult.ok = browserResult.status === "ok";
  } catch (error) {
    browserResult.error = error instanceof Error ? error.message : String(error);
    if (isBrowserEnvironmentFailure(error)) {
      browserResult.status = "skipped";
      browserResult.skipped = true;
      browserResult.skipReason = normalizeSkipReason(error);
      browserResult.ok = !STRICT_ALL_BROWSERS;
      log(browserSpec.name, "browser-run", "skipped", browserResult.skipReason);
    } else {
      browserResult.status = "fail";
      log(browserSpec.name, "browser-run", "fail", browserResult.error);
    }
  } finally {
    await browser.close();
  }
}

for (const browserSpec of BROWSERS) {
  // Run sequentially to keep evidence deterministic and avoid overloading weak RK/PC targets.
  // eslint-disable-next-line no-await-in-loop
  await verifyBrowser(browserSpec);
}

const launchedBrowsers = result.browsers.filter((browserResult) => !browserResult.skipped);
const skippedBrowsers = result.browsers.filter((browserResult) => browserResult.skipped);
result.ok =
  launchedBrowsers.length > 0 &&
  launchedBrowsers.every((browserResult) => browserResult.ok) &&
  result.unexpectedConsoleMessages.length === 0 &&
  (!STRICT_ALL_BROWSERS || skippedBrowsers.length === 0);

log("summary", "browser-matrix", result.ok ? "ok" : "fail", JSON.stringify({
  launched: launchedBrowsers.map((browserResult) => browserResult.name),
  skipped: skippedBrowsers.map((browserResult) => browserResult.name),
  pageChecks: launchedBrowsers.reduce((count, browserResult) => count + browserResult.pages.length, 0),
  unexpectedConsoleMessages: result.unexpectedConsoleMessages.length,
  strictAllBrowsers: STRICT_ALL_BROWSERS
}));

const outPath = resolve(OUT_DIR, "vue-browser-matrix-verification.json");
writeFileSync(outPath, JSON.stringify(result, null, 2));
// eslint-disable-next-line no-console
console.log(`verification -> ${outPath}`);
if (!result.ok) process.exit(1);
