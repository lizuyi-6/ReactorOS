
const { chromium } = require("@playwright/test");
(async () => {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const login = await page.request.post("http://127.0.0.1:8000/api/auth/login", { data: { username: "engineer", password: "engineer123" } });
  const { token, user } = await login.json();
  await page.addInitScript(([t, u]) => {
    localStorage.setItem("reactoros.vue.auth.token", t);
    localStorage.setItem("reactoros.vue.auth.user", JSON.stringify(u));
    localStorage.setItem("reactoros.vue.language", "zh");
  }, [token, user]);
  await page.goto("http://127.0.0.1:8000/#/history", { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  const info = await page.evaluate(() => {
    const out = [];
    document.querySelectorAll(".ev-text").forEach(el => {
      const cs = getComputedStyle(el);
      out.push({
        sw: el.scrollWidth, cw: el.clientWidth,
        ws: cs.whiteSpace, ow: cs.overflowWrap, wb: cs.wordBreak,
        display: cs.display, txt: (el.textContent || "").slice(0, 30),
        parentCls: (el.parentElement.className || "").toString().slice(0, 40),
        parentSw: el.parentElement.scrollWidth, parentCw: el.parentElement.clientWidth,
      });
    });
    return out.slice(0, 8);
  });
  console.log(JSON.stringify(info, null, 1));
  await browser.close();
})();
