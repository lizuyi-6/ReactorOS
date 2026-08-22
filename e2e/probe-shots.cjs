
const { chromium, devices } = require("@playwright/test");
(async () => {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ ...devices["Pixel 5"] });
  const page = await ctx.newPage();
  const login = await page.request.post("http://127.0.0.1:8000/api/auth/login", { data: { username: "engineer", password: "engineer123" } });
  const { token, user } = await login.json();
  await page.addInitScript(([t, u]) => {
    localStorage.setItem("reactoros.vue.auth.token", t);
    localStorage.setItem("reactoros.vue.auth.user", JSON.stringify(u));
    localStorage.setItem("reactoros.vue.language", "zh");
  }, [token, user]);
  await page.goto("http://127.0.0.1:8000/#/control", { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1800);
  await page.screenshot({ path: "output/e2e-acceptance/shots/mobile-control-fixed.png", fullPage: true });
  const ctx2 = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const p2 = await ctx2.newPage();
  await p2.addInitScript(([t, u]) => {
    localStorage.setItem("reactoros.vue.auth.token", t);
    localStorage.setItem("reactoros.vue.auth.user", JSON.stringify(u));
    localStorage.setItem("reactoros.vue.language", "en");
  }, [token, user]);
  await p2.goto("http://127.0.0.1:8000/#/audit", { waitUntil: "domcontentloaded" });
  await p2.waitForTimeout(1500);
  await p2.screenshot({ path: "output/e2e-acceptance/shots/en-audit-fixed.png" });
  await browser.close();
  console.log("shots saved");
})();
