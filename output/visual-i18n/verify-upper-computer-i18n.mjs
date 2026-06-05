import { chromium } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

const baseUrl = process.env.XINGSHU_HMI_URL || "http://127.0.0.1:8000/";
const outDir = path.resolve("output", "visual-i18n");
const stamp = "20260605";
const tabs = [
  "monitor",
  "recipes",
  "program",
  "ai",
  "materials",
  "alarms",
  "audit",
  "modbus",
  "settings",
];
const cjkPattern = /[\u3400-\u9fff]/;
const mojibakePattern = /[�]|(?:å|æ|ç|é|è|ç|瀹|鎵|宸|绯|鍚|鏈|閾|杩|鏃|搴|娓|浣|寮|绠|绛|锛|銆|€|冧|忔||||)/;
const allowedEnglishCjk = new Set(["中文"]);

await mkdir(outDir, { recursive: true });

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const consoleMessages = [];
page.on("console", (message) => {
  if (["error", "warning"].includes(message.type())) {
    consoleMessages.push({
      type: message.type(),
      text: message.text(),
      location: message.location(),
    });
  }
});
page.on("pageerror", (error) => {
  consoleMessages.push({ type: "pageerror", text: error.message });
});

async function waitForApp() {
  await page.waitForSelector("#langToggleBtn", { timeout: 15_000 });
  await page.waitForSelector("#view-monitor", { timeout: 15_000 });
}

async function login() {
  const loginPayload = await page.evaluate(async () => {
    const response = await fetch("/api/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username: "engineer", password: "engineer123" }),
    });
    const payload = await response.json();
    if (!response.ok || payload.code !== 0) {
      throw new Error(payload.message || `login failed: ${response.status}`);
    }
    localStorage.setItem("reactoros.auth.token", payload.data.token);
    localStorage.setItem("reactoros.auth.user", JSON.stringify(payload.data.user));
    return { user: payload.data.user, expires_at: payload.data.expires_at };
  });
  await page.reload({ waitUntil: "domcontentloaded" });
  await waitForApp();
  return loginPayload;
}

async function setLanguage(lang) {
  await page.evaluate((nextLang) => {
    localStorage.setItem("reactoros.lang", nextLang);
  }, lang);
  await page.reload({ waitUntil: "domcontentloaded" });
  await waitForApp();
  await page.waitForTimeout(400);
}

async function openTab(tab) {
  await page.locator(`[data-tab="${tab}"]`).first().click();
  await page.waitForSelector(`#view-${tab}.active`, { timeout: 10_000 });
  await page.waitForTimeout(900);
  await page.evaluate(() => {
    const main = document.querySelector(".main");
    if (main) main.scrollTop = 0;
  });
}

async function collectVisibleBlocks(tab, lang) {
  return page.evaluate(
    ({ activeTab, activeLang, cjkPatternSource, mojibakePatternSource, allowed }) => {
      const cjk = new RegExp(cjkPatternSource);
      const mojibake = new RegExp(mojibakePatternSource);
      const allowedCjk = new Set(allowed);
      const isVisible = (element) => {
        if (!element) return false;
        const style = getComputedStyle(element);
        if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) return false;
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      };
      const shouldIgnore = (element) =>
        element.closest("script, style, svg, canvas, .material-symbols-outlined, .nav-icon, .state-dot");
      const nodes = [];
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          const text = node.textContent.replace(/\s+/g, " ").trim();
          const parent = node.parentElement;
          if (!text || !parent || shouldIgnore(parent) || !isVisible(parent)) return NodeFilter.FILTER_REJECT;
          return NodeFilter.FILTER_ACCEPT;
        },
      });
      while (walker.nextNode()) {
        const node = walker.currentNode;
        const parent = node.parentElement;
        const text = node.textContent.replace(/\s+/g, " ").trim();
        const rect = parent.getBoundingClientRect();
        nodes.push({
          text,
          tag: parent.tagName.toLowerCase(),
          id: parent.id || "",
          classes: parent.className || "",
          inActiveView: Boolean(parent.closest(`#view-${activeTab}`) || parent.closest(".topbar") || parent.closest(".sidebar")),
          cjk: cjk.test(text),
          mojibake: mojibake.test(text),
          clipped: parent.scrollWidth > parent.clientWidth + 2 || parent.scrollHeight > parent.clientHeight + 2,
          rect: {
            x: Math.round(rect.x),
            y: Math.round(rect.y),
            w: Math.round(rect.width),
            h: Math.round(rect.height),
          },
        });
      }
      const viewText = document.querySelector(`#view-${activeTab}`)?.innerText || "";
      const auditStats = document.querySelector("#auditStats")?.innerText || "";
      const disallowedEnglishCjk = nodes
        .filter((node) => activeLang === "en" && node.cjk && !allowedCjk.has(node.text))
        .map((node) => node.text);
      const mojibakeBlocks = nodes.filter((node) => node.mojibake).map((node) => node.text);
      const clippedBlocks = nodes
        .filter((node) => node.clipped && node.text.length > 3)
        .slice(0, 30)
        .map((node) => ({ text: node.text, id: node.id, tag: node.tag, rect: node.rect }));
      return {
        tab: activeTab,
        lang: activeLang,
        htmlLang: document.documentElement.lang,
        bodyTextSample: document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 1000),
        viewTextSample: viewText.replace(/\s+/g, " ").trim().slice(0, 1000),
        viewTextLength: viewText.trim().length,
        visibleBlockCount: nodes.length,
        cjkBlockCount: nodes.filter((node) => node.cjk).length,
        disallowedEnglishCjk: Array.from(new Set(disallowedEnglishCjk)).slice(0, 50),
        mojibakeBlocks: Array.from(new Set(mojibakeBlocks)).slice(0, 50),
        clippedBlocks,
        auditStats: activeTab === "audit" ? auditStats.replace(/\s+/g, " ").trim() : undefined,
      };
    },
    {
      activeTab: tab,
      activeLang: lang,
      cjkPatternSource: cjkPattern.source,
      mojibakePatternSource: mojibakePattern.source,
      allowed: Array.from(allowedEnglishCjk),
    },
  );
}

await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
await waitForApp();
const loginPayload = await login();

const results = {
  url: baseUrl,
  login: loginPayload,
  generated_at: new Date().toISOString(),
  computer_use: {
    requested: true,
    available: false,
    reason: "Computer Use native pipe path/nativePipe was unavailable in this Codex runtime; Playwright and in-app Browser were used for equivalent visual verification.",
  },
  pages: [],
  screenshots: [],
  consoleMessages,
};

for (const lang of ["zh", "en"]) {
  await setLanguage(lang);
  for (const tab of tabs) {
    await openTab(tab);
    const screenshotPath = path.join(outDir, `upper-computer-i18n-${tab}-${lang}-${stamp}.png`);
    await page.screenshot({ path: screenshotPath, fullPage: false });
    const pageResult = await collectVisibleBlocks(tab, lang);
    pageResult.screenshot = screenshotPath;
    results.pages.push(pageResult);
    results.screenshots.push(screenshotPath);
  }
}

results.summary = {
  pageCount: results.pages.length,
  englishPagesWithUnexpectedCjk: results.pages
    .filter((item) => item.lang === "en" && item.disallowedEnglishCjk.length)
    .map((item) => ({ tab: item.tab, values: item.disallowedEnglishCjk })),
  pagesWithMojibake: results.pages
    .filter((item) => item.mojibakeBlocks.length)
    .map((item) => ({ tab: item.tab, lang: item.lang, values: item.mojibakeBlocks })),
  pagesWithEmptyViewText: results.pages
    .filter((item) => item.viewTextLength === 0)
    .map((item) => ({ tab: item.tab, lang: item.lang })),
  auditStats: results.pages
    .filter((item) => item.tab === "audit")
    .map((item) => ({ lang: item.lang, text: item.auditStats })),
  consoleErrorCount: consoleMessages.length,
  expectedDataPipeline503Count: consoleMessages.filter(
    (message) =>
      message.text.includes("503") &&
      String(message.location?.url || "").includes("/api/live?"),
  ).length,
  unexpectedConsoleMessages: consoleMessages.filter(
    (message) =>
      !(
        message.text.includes("503") &&
        String(message.location?.url || "").includes("/api/live?")
      ),
  ),
};

const reportPath = path.join(outDir, `upper-computer-i18n-audit-${stamp}.json`);
await writeFile(reportPath, JSON.stringify(results, null, 2), "utf8");
await browser.close();
console.log(JSON.stringify({ reportPath, summary: results.summary }, null, 2));
