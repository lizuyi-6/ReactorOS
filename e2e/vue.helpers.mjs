// Shared helpers for the Vue 3 HMI acceptance suite. These drive the
// production Vue app (hash routes /#/monitor … /#/settings, Pinia store,
// token in localStorage) and assert the migration invariants. (The legacy
// static-HMI suite was removed together with the old static frontend.)
import { expect } from "@playwright/test";

export const VUE_ROUTES = ["monitor", "control", "ai", "history", "audit", "modbus", "settings"];

// Tech-stack watermarks that leaked into the production HMI on every page
// during the migration. They must NOT appear anywhere in the served Vue app —
// a regression here means dev noise reached the operator again.
export const FORBIDDEN_WATERMARKS = [
  "VUE + ECHARTS",
  "Vue + ECharts",
  "ELEMENT PLUS",
  "Element Plus 表单",
  "Qwen / LoRA 边界",
  "Qwen / LoRA Readiness",
  "SQLite 历史 API",
  "SQLite History API",
  "tokio-modbus Migration Target",
  "PRD Vue 技术栈",
  "PRD Vue Stack",
  "Vue 3 / Element Plus / ECharts / Pinio",
  "迁移分支",
  "0 项权限",
];

const PIPELINE_SAMPLE = {
  temperature_c: 31.11,
  pressure_mpa: 0.5,
  stirrer_rpm: 125.18,
  shake_speed_cpm: 30.0,
  tilt_state: 1,
  flow_rate_l_min: 2.42,
  product_concentration_percent: 11.1,
  ph: 6.15,
};

export async function vueLogin(request) {
  const r = await request.post("/api/auth/login", {
    data: { username: "engineer", password: "engineer123" },
  });
  expect(r.status(), "engineer login").toBe(200);
  const body = await r.json();
  return body.data?.token ?? body.token;
}

export async function vueKeepPipelineFlowing(request, token) {
  const bearer = token ?? (await vueLogin(request));
  const post = () =>
    request
      .post("/api/v1/reactor/reactor_001/samples", {
        headers: { Authorization: `Bearer ${bearer}` },
        data: PIPELINE_SAMPLE,
      })
      .catch(() => {});
  await post();
  return setInterval(post, 2000);
}

// AI 主控执行（fullchain ai 用例点击"立即执行"）会经 aiApi.control 默认
// allow_process_start=true 自动启动工艺/批次，污染后续用例（409 控制台噪音、
// AI 页错误态）。每个用例开始前自愈：停活动工艺 + 结束未完结批次。
export async function ensureNoActiveBatch(request, token) {
  const headers = { Authorization: "Bearer " + token, "Content-Type": "application/json" };
  try {
    const cur = await request.get("/api/v1/control/process/current", { headers });
    if (cur.ok()) {
      const j = await cur.json().catch(() => ({}));
      const st = j?.data?.status ?? j?.status;
      if (st === "active") {
        await request.post("/api/v1/control/process/stop", { headers, data: { reason: "e2e 用例隔离清理" } }).catch(() => {});
      }
    }
    const b = await request.get("/api/batches", { headers });
    if (b.ok()) {
      const j = await b.json().catch(() => ({}));
      const list = j?.data?.batches ?? [];
      for (const bt of list) {
        if (!(bt.finished_at ?? bt.finishedAt)) {
          await request.post("/api/batches/" + (bt.id ?? bt.batch_id) + "/finish", { headers, data: { reason: "e2e 用例隔离清理" } }).catch(() => {});
        }
      }
    }
  } catch { /* 自愈失败不阻塞用例 */ }
}

// Load the Vue app authenticated, with a live pipeline feeding samples so the
// monitor shows real readings (not the 503 empty state). Returns a cleanup fn.
export async function prepareVuePage(page, request) {
  const token = await vueLogin(request);
  await ensureNoActiveBatch(request, token);
  // NOTE: we intentionally do NOT call /api/test/reset here. Reset wipes the
  // seeded demo batches/processes, which the history-timestamp test needs to
  // see (an empty history table has no timestamps to format-check). The
  // pipeline feeder below brings /api/live to 200 regardless of prior state.
  const interval = await vueKeepPipelineFlowing(request, token);
  // Wait until /api/live is actually serving samples before loading the UI.
  await expect
    .poll(
      async () => {
        const r = await request.get("/api/live");
        if (r.status() !== 200) return 0;
        const live = await r.json();
        return live.recent_samples?.length ?? 0;
      },
      { timeout: 12_000 }
    )
    .toBeGreaterThan(0);
  const consoleErrors = [];
  page.on("console", (m) => {
    if (m.type() !== "error") return;
    const text = m.text();
    // Tolerate the 503 flap before the pipeline feeds the first sample.
    if (text.includes("Failed to load resource") && text.includes("503")) return;
    consoleErrors.push(text);
  });
  page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));
  page.on("close", () => clearInterval(interval));
  page.consoleErrors = consoleErrors;
  await page.addInitScript(([t]) => {
    localStorage.setItem("reactoros.vue.auth.token", t);
    localStorage.setItem(
      "reactoros.vue.auth.user",
      JSON.stringify({ username: "engineer", role: "engineer", permissions: [] })
    );
    localStorage.setItem("reactoros.vue.language", "zh");
  }, [token]);
  await page.goto("/#/monitor", { waitUntil: "domcontentloaded" });
  await expect(page.locator("body")).toContainText("Reactor Overview");
  return () => clearInterval(interval);
}

export function assertNoVueConsoleErrors(page) {
  expect(page.consoleErrors, "Vue app console/page errors").toEqual([]);
}

export async function assertNoHorizontalOverflow(page) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow, "horizontal overflow (content wider than viewport)").toBeLessThanOrEqual(2);
}