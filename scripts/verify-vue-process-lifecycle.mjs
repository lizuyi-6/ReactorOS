// Verify Vue process / batch lifecycle slice against the running dev build.
// Outputs: output/playwright/vue-process-lifecycle-verification.json
//          output/playwright/vue-process-lifecycle-en.png
//          output/playwright/vue-process-lifecycle-zh.png

import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = process.cwd();
const OUT_DIR = resolve(ROOT, "output/playwright");
mkdirSync(OUT_DIR, { recursive: true });

const VUE_URL = process.env.VUE_URL || "http://127.0.0.1:5173/";
const API_BASE = process.env.E2E_BASE_URL || "http://127.0.0.1:8000";

// Each phrase must be present on the Control page in the relevant language
// under the conditions the script visits the page. Phrases that are only
// rendered when a row / batch is selected (e.g. "Add Step" inside the
// process detail panel, "Active batch" inside the current-run tag) are
// excluded from the static list and asserted in the dedicated lifecycle
// steps (create-process / add-step / start-process) instead.
const ENGLISH_CHECKS = [
  "Process Control",
  "Process Recipes",
  "Process Detail",
  "Create Process",
  "Current Run",
  "Recent Batches"
];

const CHINESE_CHECKS = [
  "参数配置",
  "工艺管理",
  "工艺详情",
  "创建工艺",
  "当前运行",
  "最近批次"
];

const result = {
  ok: false,
  url: VUE_URL,
  apiBase: API_BASE,
  steps: [],
  englishChecks: ENGLISH_CHECKS,
  chineseChecks: CHINESE_CHECKS,
  englishFound: [],
  chineseFound: [],
  englishMissing: [],
  chineseMissing: [],
  processCreated: null,
  stepAdded: null,
  processStarted: null,
  processStopped: null,
  horizontalOverflow: null,
  horizontalOverflowZh: null,
  screenshots: {
    en: "output/playwright/vue-process-lifecycle-en.png",
    zh: "output/playwright/vue-process-lifecycle-zh.png"
  }
};

function log(step, status, info) {
  const entry = { step, status, info };
  result.steps.push(entry);
  // eslint-disable-next-line no-console
  console.log(`[${status}] ${step}${info ? " :: " + info : ""}`);
}

async function login(page, request) {
  const res = await request.post(`${API_BASE}/api/auth/login`, {
    data: { username: "engineer", password: "engineer123" }
  });
  if (!res.ok()) {
    const text = await res.text();
    throw new Error(`login failed: ${res.status()} ${text}`);
  }
  const body = await res.json();
  return body.data ?? body;
}

async function postJson(request, path, token) {
  const res = await request.post(`${API_BASE}${path}`, {
    headers: { Authorization: `Bearer ${token}` }
  });
  if (!res.ok()) return { status: res.status(), body: await res.text() };
  const json = await res.json();
  return { status: res.status(), body: json.data ?? json };
}

async function getJson(request, path, token) {
  const res = await request.get(`${API_BASE}${path}`, {
    headers: { Authorization: `Bearer ${token}` }
  });
  if (!res.ok()) return { status: res.status(), body: await res.text() };
  const json = await res.json();
  return { status: res.status(), body: json.data ?? json };
}

async function detectHorizontalOverflow(page) {
  return page.evaluate(() => {
    const content = document.querySelector(".content") || document.body;
    const stack = document.querySelector(".view-stack") || document.body;
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
    // 1. Login as engineer via REST, then go to Vue control page.
    const loginBody = await login(page, request);
    const token = loginBody.token;
    log("login-engineer", "ok", JSON.stringify({ role: loginBody.user.role }));

    // Inject the bearer token into localStorage so Vue reads it.
    await page.goto(`${VUE_URL}#/control`);
    await page.evaluate((t) => {
      localStorage.setItem("reactoros.vue.auth.token", t);
      localStorage.setItem("reactoros.vue.auth.user", JSON.stringify({ username: "engineer", role: "engineer", permissions: ["edit_process", "start_stop_process", "set_safe_targets", "view_monitor", "view_history", "view_audit", "export_reports", "apply_ai_suggestion", "emergency_stop", "modbus_debug"] }));
      localStorage.setItem("reactoros.vue.language", "en");
    }, token);
    await page.reload();
    await page.waitForLoadState("domcontentloaded");
    // wait for control view heading
    await page.locator("h1", { hasText: "Process Control" }).waitFor({ timeout: 12_000 });
    log("open-control-en", "ok");

    // Make sure no active batch is lingering.
    await postJson(request, "/api/processes/current/stop", token);

    // 2. EN: read English text blocks.
    const enBody = await page.locator("body").innerText();
    for (const phrase of ENGLISH_CHECKS) {
      if (enBody.includes(phrase)) result.englishFound.push(phrase);
      else result.englishMissing.push(phrase);
    }
    log("english-checks", "ok", `${result.englishFound.length}/${ENGLISH_CHECKS.length} found; missing: ${result.englishMissing.join(", ") || "none"}`);

    await page.screenshot({ path: resolve(ROOT, result.screenshots.en), fullPage: true });

    // 3. Create a process via Vue.
    const processName = `Vue process ${Date.now()}`;
    await page.locator('input[maxlength="80"]').first().fill(processName);
    const createBtn = page.locator("button", { hasText: "Create Process" });
    await createBtn.waitFor({ timeout: 5_000 });
    await createBtn.click();
    // Race: either the el-message shows, or the new row appears in the table,
    // or the API eventually returns the new process. The toast can disappear
    // before Playwright observes it.
    const createDeadline = Date.now() + 10_000;
    let created = null;
    while (Date.now() < createDeadline) {
      const listAfterCreate = await getJson(request, "/api/processes", token);
      created = (Array.isArray(listAfterCreate.body) ? listAfterCreate.body : []).find((p) => p.name === processName);
      if (created) break;
      await page.waitForTimeout(300);
    }
    if (!created) throw new Error("created process not found in /api/processes list");
    result.processCreated = { id: created.id, name: created.name };
    log("create-process", "ok", JSON.stringify(result.processCreated));

    // 4. The process becomes the selected one; fill the step form and add a step.
    const stepName = `Step-${Date.now()}`;
    const stepInputs = page.locator('input[maxlength="80"]');
    const stepInputCount = await stepInputs.count();
    if (stepInputCount < 2) {
      throw new Error(`expected at least 2 maxlength=80 inputs, found ${stepInputCount}`);
    }
    await stepInputs.nth(1).fill(stepName);
    const addStepBtn = page.locator("button", { hasText: "Add Step" });
    await addStepBtn.waitFor({ timeout: 5_000 });
    await addStepBtn.click();
    const stepDeadline = Date.now() + 10_000;
    let added = null;
    while (Date.now() < stepDeadline) {
      const detail = await getJson(request, `/api/processes/${created.id}`, token);
      const steps = Array.isArray(detail.body?.steps) ? detail.body.steps : [];
      added = steps.find((s) => s.name === stepName);
      if (added) break;
      await page.waitForTimeout(300);
    }
    if (!added) throw new Error("added step not found in /api/processes/:id detail");
    result.stepAdded = { id: added.id, name: added.name };
    log("add-step", "ok", JSON.stringify(result.stepAdded));

    // 5. Start the process through Vue.
    const startBtn = page.locator("button", { hasText: "Start" }).first();
    await startBtn.waitFor({ timeout: 5_000 });
    await startBtn.click();
    const startDeadline = Date.now() + 12_000;
    let liveAfterStart = null;
    while (Date.now() < startDeadline) {
      const live = await getJson(request, "/api/live", token);
      if (live.body?.runtime?.active_batch_id) {
        liveAfterStart = live.body.runtime;
        break;
      }
      await page.waitForTimeout(300);
    }
    if (!liveAfterStart) throw new Error("runtime.active_batch_id still empty after start");
    result.processStarted = { active_batch_id: liveAfterStart.active_batch_id, auto_enabled: liveAfterStart.auto_enabled };
    log("start-process", "ok", JSON.stringify(result.processStarted));

    // 6. Stop the running process.
    const stopBtn = page.locator("button", { hasText: "Stop Current Process" });
    await stopBtn.waitFor({ timeout: 5_000 });
    await stopBtn.click();
    const stopDeadline = Date.now() + 12_000;
    let liveAfterStop = null;
    while (Date.now() < stopDeadline) {
      const live = await getJson(request, "/api/live", token);
      if (!live.body?.runtime?.active_batch_id) {
        liveAfterStop = live.body.runtime;
        break;
      }
      await page.waitForTimeout(300);
    }
    if (!liveAfterStop) throw new Error("active_batch_id still set after stop");
    result.processStopped = { auto_enabled: liveAfterStop.auto_enabled, active_batch_id: liveAfterStop.active_batch_id };
    log("stop-process", "ok", JSON.stringify(result.processStopped));

    // 7. Horizontal overflow sanity check.
    const overflow = await detectHorizontalOverflow(page);
    result.horizontalOverflow = {
      docScroll: overflow.docScroll,
      docClient: overflow.docClient,
      contentScroll: overflow.contentScroll,
      contentClient: overflow.contentClient,
      stackScroll: overflow.stackScroll,
      stackClient: overflow.stackClient,
      ok:
        overflow.docScroll <= overflow.docClient + 1 &&
        overflow.contentScroll <= overflow.contentClient + 1 &&
        overflow.stackScroll <= overflow.stackClient + 1
    };
    log("overflow-en", result.horizontalOverflow.ok ? "ok" : "fail", JSON.stringify(overflow));

    // 8. Switch to Chinese and re-check.
    // The language switch is an el-segmented with two options: 中文 and EN.
    const zhSeg = page.locator(".el-segmented .el-segmented__item", { hasText: "中文" });
    await zhSeg.waitFor({ timeout: 5_000 });
    await zhSeg.click();
    await page.waitForFunction(
      () => document.body.innerText.includes("参数配置") && document.body.innerText.includes("工艺管理"),
      null,
      { timeout: 8_000 }
    );
    const zhBody = await page.locator("body").innerText();
    for (const phrase of CHINESE_CHECKS) {
      if (zhBody.includes(phrase)) result.chineseFound.push(phrase);
      else result.chineseMissing.push(phrase);
    }
    log("chinese-checks", "ok", `${result.chineseFound.length}/${CHINESE_CHECKS.length} found; missing: ${result.chineseMissing.join(", ") || "none"}`);
    await page.screenshot({ path: resolve(ROOT, result.screenshots.zh), fullPage: true });

    const overflowZh = await detectHorizontalOverflow(page);
    result.horizontalOverflowZh = {
      docScroll: overflowZh.docScroll,
      docClient: overflowZh.docClient,
      contentScroll: overflowZh.contentScroll,
      contentClient: overflowZh.contentClient,
      stackScroll: overflowZh.stackScroll,
      stackClient: overflowZh.stackClient,
      ok:
        overflowZh.docScroll <= overflowZh.docClient + 1 &&
        overflowZh.contentScroll <= overflowZh.contentClient + 1 &&
        overflowZh.stackScroll <= overflowZh.stackClient + 1
    };
    log("overflow-zh", result.horizontalOverflowZh.ok ? "ok" : "fail", JSON.stringify(overflowZh));

    // 9. Final pass conditions: every required phrase must be present in both
    //    languages, the lifecycle must complete end-to-end, and the page must
    //    not overflow horizontally in either language.
    const enOk = result.englishMissing.length === 0;
    const zhOk = result.chineseMissing.length === 0;
    const lifecycleOk = Boolean(
      result.processCreated?.id &&
        result.stepAdded?.id &&
        result.processStarted?.active_batch_id &&
        result.processStopped?.active_batch_id === null
    );
    const overflowOk = result.horizontalOverflow.ok && result.horizontalOverflowZh.ok;
    result.ok = enOk && zhOk && lifecycleOk && overflowOk;
    log(
      "summary",
      result.ok ? "ok" : "fail",
      JSON.stringify({ enOk, zhOk, lifecycleOk, overflowOk })
    );
  } catch (error) {
    log("error", "fail", error instanceof Error ? error.message : String(error));
    result.error = error instanceof Error ? error.message : String(error);
  } finally {
    await context.close();
    await browser.close();
  }

  const outPath = resolve(OUT_DIR, "vue-process-lifecycle-verification.json");
  writeFileSync(outPath, JSON.stringify(result, null, 2));
  // eslint-disable-next-line no-console
  console.log(`verification -> ${outPath}`);
  if (!result.ok) process.exit(1);
})();
