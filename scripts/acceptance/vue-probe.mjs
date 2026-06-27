// Exploratory acceptance probe for the Vue 3 HMI (the migration target).
// The existing Playwright suite targets the legacy static HMI; this drives the
// real Vue app: logs in via the API, injects the token into localStorage,
// walks all 7 hash routes at desktop + mobile viewports, screenshots each,
// captures console/page errors + visible text, and runs one boundary
// interaction (out-of-range target submit) to exercise the safety gate UX.
//
// Usage: node scripts/acceptance/vue-probe.mjs   (after daemon + sim are up)
import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";

const BASE = process.env.E2E_BASE_URL || "http://127.0.0.1:8000";
const OUT = "output/acceptance/vue-probe";
mkdirSync(OUT, { recursive: true });

const ROUTES = ["monitor", "control", "ai", "history", "audit", "modbus", "settings"];

const VIEWPORTS = [
  { name: "desktop", width: 1440, height: 900 },
  { name: "mobile", width: 393, height: 851 },
];

async function login(browser) {
  const ctx = await browser.newContext({ baseURL: BASE });
  const token = await loginEngineer(ctx);
  await ctx.close();
  return token;
}

async function loginEngineer(ctx) {
  const r = await ctx.request.post("/api/auth/login", {
    data: { username: "engineer", password: "engineer123" },
  });
  if (!r.ok()) throw new Error(`login failed: ${r.status()}`);
  const body = await r.json();
  return body.data?.token ?? body.token;
}

const findings = [];

async function probeViewport(browser, token, vp) {
  const ctx = await browser.newContext({
    viewport: { width: vp.width, height: vp.height },
    baseURL: BASE,
  });
  await ctx.addInitScript(([t]) => {
    localStorage.setItem("reactoros.vue.auth.token", t);
    localStorage.setItem(
      "reactoros.vue.auth.user",
      JSON.stringify({ username: "engineer", role: "engineer", permissions: [] })
    );
    localStorage.setItem("reactoros.vue.language", "zh");
  }, [token]);
  const page = await ctx.newPage();
  const consoleErrors = [];
  page.on("console", (m) => m.type() === "error" && consoleErrors.push(m.text()));
  page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));

  for (const route of ROUTES) {
    const errorsHere = [];
    const before = consoleErrors.length;
    await page.goto(`/#/${route}`, { waitUntil: "domcontentloaded" });
    try {
      await page.waitForLoadState("networkidle", { timeout: 6000 });
    } catch {}
    await page.waitForTimeout(800);
    const shot = `${OUT}/${vp.name}-${route}.png`;
    await page.screenshot({ path: shot, fullPage: true });
    const bodyText = (await page.locator("body").innerText()).replace(/\s+\n/g, "\n").slice(0, 1200);
    const overflow = await page.evaluate(() => {
      const d = document.documentElement;
      return { scrollW: d.scrollWidth, clientW: d.clientWidth, hScroll: d.scrollWidth > d.clientWidth + 2 };
    });
    errorsHere.push(...consoleErrors.slice(before));
    findings.push({ viewport: vp.name, route, errors: errorsHere, hScroll: overflow.hScroll, scrollW: overflow.scrollW, bodyText });
    console.log(`[${vp.name}] /${route} errors=${errorsHere.length} hScroll=${overflow.hScroll} (${overflow.scrollW}>${overflow.clientW})`);
  }

  // Boundary interaction on control page: submit an out-of-range target.
  await page.goto(`/#/control`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(800);
  let boundary = { attempted: false };
  try {
    // Element Plus number inputs; target temperature field.
    const tempInputs = page.locator('input[type="number"], .el-input-number input');
    const count = await tempInputs.count();
    if (count > 0) {
      await tempInputs.first().fill("99999");
      const btns = page.locator('button:has-text("提交"), button:has-text("下发"), button:has-text("确认"), .el-button--primary');
      const bc = await btns.count();
      boundary.attempted = true;
      boundary.inputCount = count;
      boundary.primaryBtnCount = bc;
      if (bc > 0) {
        const [resp] = await Promise.all([
          page.waitForResponse((r) => r.url().includes("/api/") && (r.request().method() === "POST" || r.request().method() === "PUT"), { timeout: 4000 }).catch(() => null),
          btns.first().click({ timeout: 2000 }).catch((e) => (boundary.clickErr = String(e))),
        ]);
        if (resp) {
          boundary.status = resp.status();
          boundary.body = (await resp.text()).slice(0, 300);
        }
      }
      await page.screenshot({ path: `${OUT}/${vp.name}-control-boundary.png`, fullPage: true });
    }
  } catch (e) {
    boundary.err = String(e);
  }
  findings.push({ viewport: vp.name, route: "control-boundary", boundary });
  console.log(`[${vp.name}] control-boundary: ${JSON.stringify(boundary)}`);
  await ctx.close();
}

const browser = await chromium.launch();
const token = await login(browser);
console.log("token:", token ? "ok" : "MISSING");
for (const vp of VIEWPORTS) {
  await probeViewport(browser, token, vp);
}
await browser.close();
writeFileSync(`${OUT}/findings.json`, JSON.stringify(findings, null, 2));
console.log(`\nWrote ${OUT}/findings.json and ${ROUTES.length * 2} (+2 boundary) screenshots.`);
