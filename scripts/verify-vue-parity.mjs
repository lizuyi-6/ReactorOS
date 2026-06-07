// Verify Vue parity slice across AI/History/Settings/Monitor/Modbus routes.
// Outputs:
//   output/playwright/vue-parity-verification.json
//   output/playwright/vue-parity-ai-en.png, vue-parity-ai-zh.png
//   output/playwright/vue-parity-history-en.png, vue-parity-history-zh.png
//   output/playwright/vue-parity-settings-en.png, vue-parity-settings-zh.png
//   output/playwright/vue-parity-monitor-en.png, vue-parity-monitor-zh.png
//   output/playwright/vue-parity-modbus-en.png, vue-parity-modbus-zh.png
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
    en: [
      "AI Decision",
      "Latest Recommendation Provider",
      "Recommendation Detail",
      "AI Master Control",
      "Dry-run",
      "Execute",
      "AI Result Review",
      "Decision Summary",
      "Action Review",
      "Safety Gate",
      "Recommended Targets",
      "Experiment Plan",
      "SOP Draft"
    ],
    zh: [
      "AI 决策",
      "最新推荐来源",
      "推荐内容",
      "AI 主控",
      "AI 结果复核",
      "决策摘要",
      "动作复核",
      "安全门控",
      "推荐目标",
      "实验方案",
      "加载 SOP 草案"
    ]
  },
  {
    route: "/history",
    name: "history",
    en: [
      "History Data",
      "History Filters",
      "Search",
      "All statuses",
      "Product ratio",
      "All ratios",
      "Clear Filters",
      "Batch Detail",
      "Export CSV",
      "Export XLSX",
      "Product Result Entry",
      "Save Product Result",
      "Product Outcomes",
      "Yield %",
      "Target temperature"
    ],
    zh: [
      "历史数据",
      "历史筛选",
      "搜索",
      "全部状态",
      "产物比例",
      "全部比例",
      "清空筛选",
      "批次详情",
      "导出 CSV",
      "导出 XLSX",
      "产物结果录入",
      "保存产物结果",
      "产物结果",
      "产率 %",
      "目标温度"
    ]
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
  },
  {
    route: "/modbus",
    name: "modbus",
    en: [
      "Modbus Debug",
      "Register Debug",
      "Integration Surface",
      "Base inference",
      "LoRA inference",
      "PRD LoRA/RK",
      "Holding / Input Registers"
    ],
    zh: [
      "Modbus 调试",
      "寄存器调试",
      "集成接口状态",
      "基础模型入口",
      "LoRA 推理闭环",
      "PRD LoRA/RK 闭环",
      "保持/输入寄存器"
    ]
  }
];

const RENDER_PLACEHOLDERS = ["[object Object]"];

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

async function verifyHistoryExport(page, pageResult) {
  const [csvDownload] = await Promise.all([
    page.waitForEvent("download", { timeout: 8_000 }),
    page.locator("button", { hasText: "Export CSV" }).click()
  ]);
  const csvFilename = csvDownload.suggestedFilename();
  pageResult.historyExportCsv = csvFilename.endsWith(".csv") ? "ok" : "fail";
  await csvDownload.delete().catch(() => {});
  log(pageResult, "history-export-csv", pageResult.historyExportCsv, csvFilename);

  const [xlsxDownload] = await Promise.all([
    page.waitForEvent("download", { timeout: 8_000 }),
    page.locator("button", { hasText: "Export XLSX" }).click()
  ]);
  const xlsxFilename = xlsxDownload.suggestedFilename();
  pageResult.historyExportXlsx = xlsxFilename.endsWith(".xlsx") ? "ok" : "fail";
  await xlsxDownload.delete().catch(() => {});
  log(pageResult, "history-export-xlsx", pageResult.historyExportXlsx, xlsxFilename);
}

function mockedLiveAlarmPayload() {
  return {
    runtime: {
      emergency_stop: false,
      manual_lock: false,
      targets: {
        temperature_c: 60,
        stirrer_rpm: 300,
        shake_speed_cpm: 30,
        target_pressure_mpa: 0.1
      }
    },
    recent_samples: [],
    alarms: [
      {
        type: "emergency_stop",
        level: "high",
        message: "manual emergency stop is active",
        suggestion: "confirm field safety before resetting emergency stop",
        current_value: 1,
        limit_value: 0
      }
    ]
  };
}

async function seedRealAlarmSample(request, pageResult) {
  const basePayload = {
    temperature_c: 170,
    pressure_mpa: 1.2,
    stirrer_rpm: 125.18,
    shake_speed_cpm: 30,
    tilt_state: 1,
    flow_rate_l_min: 2.42,
    product_concentration_percent: 11.1,
    ph: 6.15
  };
  let lastTypes = [];
  let lastBody = null;
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const payload = {
      ...basePayload,
      temperature_c: basePayload.temperature_c + attempt * 0.01,
      pressure_mpa: basePayload.pressure_mpa + attempt * 0.001
    };
    const res = await request.post(`${API_BASE}/api/v1/reactor/reactor_001/samples`, { data: payload });
    if (!res.ok()) throw new Error(`failed to seed alarm sample: ${res.status()} ${await res.text()}`);
    const live = await request.get(`${API_BASE}/api/live?sample_limit=1&include_processes=false&include_batches=false&include_events=false`);
    if (!live.ok()) throw new Error(`live did not accept seeded alarm sample: ${live.status()} ${await live.text()}`);
    const body = await live.json();
    const alarms = Array.isArray(body.alarms) ? body.alarms : [];
    const types = alarms.map((alarm) => alarm.type).filter(Boolean);
    lastTypes = types;
    lastBody = body;
    if (types.includes("temperature_limit") && types.includes("pressure_limit")) {
      pageResult.realAlarmSeed = {
        ok: true,
        types,
        sample: payload,
        attempts: attempt + 1
      };
      log(pageResult, "real-alarm-seed", "ok", `types=[${types.join(",") || "none"}]; attempts=${attempt + 1}`);
      return body;
    }
    await new Promise((resolveRetry) => setTimeout(resolveRetry, 40));
  }
  pageResult.realAlarmSeed = {
    ok: false,
    types: lastTypes,
    sample: basePayload,
    attempts: 20
  };
  log(pageResult, "real-alarm-seed", "fail", `types=[${lastTypes.join(",") || "none"}]; attempts=20`);
  throw new Error(`seeded sample did not produce temperature and pressure alarms: ${lastTypes.join(",")}; lastBody=${JSON.stringify(lastBody)}`);
}

async function verifyMonitorAlarmRendering(request, page, pageResult) {
  const verifyLanguage = async (language, heading, phrases, screenshotName) => {
    const alarmLive = await seedRealAlarmSample(request, pageResult);
    const liveRoute = (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(alarmLive)
      });
    };
    await page.route("**/api/live?**", liveRoute);
    await setLanguageAndReload(page, language);
    await page.goto(`${VUE_URL}#/monitor`);
    await page.waitForLoadState("domcontentloaded");
    await page.locator("h1", { hasText: heading }).waitFor({ timeout: 8_000 });
    await page.locator("button", { hasText: language === "en" ? "Load live data" : "加载实时数据" }).click();
    await page.waitForFunction(
      (expected) => expected.some((phrase) => document.body.innerText.includes(phrase)),
      phrases,
      { timeout: 8_000 }
    ).catch(() => {});
    const body = await page.locator("body").innerText();
    const missing = phrases.filter((phrase) => !body.includes(phrase));
    await page.screenshot({ path: resolve(ROOT, `output/playwright/${screenshotName}`), fullPage: true });
    await page.unroute("**/api/live?**", liveRoute);
    return { ok: missing.length === 0, missing };
  };

  pageResult.monitorAlarmEn = await verifyLanguage(
    "en",
    "Realtime Monitor",
    ["High", "Temperature limit", "Pressure limit", "Reactor temperature outside hard limit", "Current / Limit", "Vent through the validated relief path"],
    "vue-parity-monitor-alarm-en.png"
  );
  log(pageResult, "monitor-alarm-en", pageResult.monitorAlarmEn.ok ? "ok" : "fail", `missing=[${pageResult.monitorAlarmEn.missing.join(",") || "none"}]`);

  pageResult.monitorAlarmZh = await verifyLanguage(
    "zh",
    "实时监控",
    ["高", "温度越限", "压力越限", "反应温度越限", "当前/限值", "通过已验证的泄压路径"],
    "vue-parity-monitor-alarm-zh.png"
  );
  log(pageResult, "monitor-alarm-zh", pageResult.monitorAlarmZh.ok ? "ok" : "fail", `missing=[${pageResult.monitorAlarmZh.missing.join(",") || "none"}]`);
}

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, acceptDownloads: true });
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
        renderPlaceholdersEnAbsent: null,
        renderPlaceholdersZhAbsent: null,
        historyExportCsv: null,
        historyExportXlsx: null,
        monitorAlarmEn: null,
        monitorAlarmZh: null,
        realAlarmSeed: null,
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
      pageResult.renderPlaceholdersEnAbsent = !RENDER_PLACEHOLDERS.some((placeholder) => enBody.includes(placeholder));
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
      log(
        pageResult,
        "render-placeholders-en",
        pageResult.renderPlaceholdersEnAbsent ? "ok" : "fail",
        RENDER_PLACEHOLDERS.join(", ")
      );
      await page.screenshot({ path: resolve(ROOT, pageResult.screenshots.en), fullPage: true });
      if (spec.name === "history") {
        await verifyHistoryExport(page, pageResult);
      }
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
      pageResult.renderPlaceholdersZhAbsent = !RENDER_PLACEHOLDERS.some((placeholder) => zhBody.includes(placeholder));
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
      log(
        pageResult,
        "render-placeholders-zh",
        pageResult.renderPlaceholdersZhAbsent ? "ok" : "fail",
        RENDER_PLACEHOLDERS.join(", ")
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

      if (spec.name === "monitor") {
        await verifyMonitorAlarmRendering(request, page, pageResult);
      }
    }

    // Strict pass criteria: every page must open both languages, every required
    // phrase must be present in both languages, no horizontal overflow.
    result.ok = result.pages.every(
      (p) =>
        p.openEn === "ok" &&
        p.openZh === "ok" &&
        p.englishMissing.length === 0 &&
        p.chineseMissing.length === 0 &&
        p.renderPlaceholdersEnAbsent === true &&
        p.renderPlaceholdersZhAbsent === true &&
        (p.name !== "history" || p.historyExportCsv === "ok") &&
        (p.name !== "history" || p.historyExportXlsx === "ok") &&
        (p.name !== "monitor" || (p.realAlarmSeed?.ok === true && p.monitorAlarmEn?.ok === true && p.monitorAlarmZh?.ok === true)) &&
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
