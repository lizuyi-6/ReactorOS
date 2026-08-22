
// 探针：打印溢出元素的类名+文本片段（验证修复用，非验收断言）
const { chromium } = require("@playwright/test");
const routes = ["control", "history", "settings"];
(async () => {
  const browser = await chromium.launch();
  for (const vp of [{ width: 1440, height: 900 }, { width: 393, height: 851 }]) {
    const ctx = await browser.newContext({ viewport: vp });
    const page = await ctx.newPage();
    const login = await page.request.post("http://127.0.0.1:8000/api/auth/login", { data: { username: "engineer", password: "engineer123" } });
    const { token, user } = await login.json();
    await page.addInitScript(([t, u]) => {
      localStorage.setItem("reactoros.vue.auth.token", t);
      localStorage.setItem("reactoros.vue.auth.user", JSON.stringify(u));
      localStorage.setItem("reactoros.vue.language", "zh");
    }, [token, user]);
    const excl = ".el-scrollbar__wrap,.el-table__body-wrapper,.cards-scroll,.scrollable,[class*=overflow-auto],.el-table__cell .cell,.el-table__header-wrapper";
    for (const route of routes.concat(vp.width === 393 ? ["monitor", "ai", "audit", "modbus"] : [])) {
      await page.goto("http://127.0.0.1:8000/#/" + route, { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(1200);
      const items = await page.evaluate((excl) => {
        const out = [];
        (function walk(el) {
          for (const ch of el.children) {
            if (ch.scrollWidth - ch.clientWidth > 5 && !ch.matches(excl)) {
              const txt = (ch.textContent || "").replace(/\\s+/g, " ").trim().slice(0, 40);
              out.push((ch.className || ch.tagName).toString().slice(0, 45) + " | " + txt);
            }
            walk(ch);
          }
        })(document.body);
        return out.slice(0, 25);
      }, excl);
      console.log("== " + vp.width + "px /" + route + " ==");
      items.forEach(i => console.log("   " + i));
    }
    await ctx.close();
  }
  await browser.close();
})();
