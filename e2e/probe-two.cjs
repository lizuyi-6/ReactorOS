
const { chromium } = require("@playwright/test");
(async () => {
  const browser = await chromium.launch();
  for (const [w, h, route] of [[1440, 900, "control"], [393, 851, "audit"]]) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h } });
    const page = await ctx.newPage();
    const login = await page.request.post("http://127.0.0.1:8000/api/auth/login", { data: { username: "engineer", password: "engineer123" } });
    const { token, user } = await login.json();
    await page.addInitScript(([t, u]) => {
      localStorage.setItem("reactoros.vue.auth.token", t);
      localStorage.setItem("reactoros.vue.auth.user", JSON.stringify(u));
      localStorage.setItem("reactoros.vue.language", "zh");
    }, [token, user]);
    await page.goto("http://127.0.0.1:8000/#/" + route, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1500);
    const info = await page.evaluate(() => {
      const out = [];
      (function walk(el) {
        for (const ch of el.children) {
          if (ch.scrollWidth - ch.clientWidth > 5) {
            out.push({
              cls: (ch.className || ch.tagName).toString().slice(0, 40),
              sw: ch.scrollWidth, cw: ch.clientWidth,
              txt: (ch.textContent || "").replace(/\s+/g, " ").trim().slice(0, 30),
            });
          }
          walk(ch);
        }
      })(document.body);
      return out.slice(0, 14);
    });
    console.log("== " + w + " /" + route);
    info.forEach(i => console.log("  " + JSON.stringify(i)));
    await ctx.close();
  }
  await browser.close();
})();
