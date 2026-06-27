import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const ROOT = process.cwd();
const OUT_DIR = resolve(ROOT, 'output/playwright');
mkdirSync(OUT_DIR, { recursive: true });

const VUE_URL = process.env.VUE_URL || 'http://127.0.0.1:8000/';
const API_BASE = process.env.E2E_BASE_URL || 'http://127.0.0.1:8000';
const result = {
  ok: false,
  url: VUE_URL,
  apiBase: API_BASE,
  csv: null,
  xlsx: null,
  screenshots: {
    en: 'output/playwright/vue-history-xlsx-export-en.png',
    zh: 'output/playwright/vue-history-xlsx-export-zh.png'
  },
  languageChecks: {
    en: false,
    zh: false
  },
  consoleErrors: []
};

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, acceptDownloads: true });
const page = await context.newPage();
page.on('console', (msg) => {
  if (msg.type() === 'error') result.consoleErrors.push(msg.text());
});
page.on('pageerror', (error) => result.consoleErrors.push(error.message));
try {
  const login = await context.request.post(`${API_BASE}/api/auth/login`, {
    data: { username: 'engineer', password: 'engineer123' }
  });
  if (!login.ok()) throw new Error(`login failed: ${login.status()} ${await login.text()}`);
  const body = await login.json();
  const token = body.data?.token ?? body.token;
  await page.goto(VUE_URL);
  await page.evaluate((t) => {
    localStorage.setItem('reactoros.vue.auth.token', t);
    localStorage.setItem('reactoros.vue.auth.user', JSON.stringify({
      username: 'engineer',
      role: 'engineer',
      permissions: ['view_monitor', 'view_history', 'view_audit', 'export_reports', 'edit_process', 'start_stop_process', 'set_safe_targets', 'apply_ai_suggestion', 'emergency_stop', 'modbus_debug', 'ingest_sensor_sample']
    }));
    localStorage.setItem('reactoros.vue.language', 'en');
  }, token);
  await page.reload();
  await page.waitForLoadState('domcontentloaded');
  await page.goto(`${VUE_URL}#/history`);
  await page.waitForLoadState('domcontentloaded');
  await page.getByRole('heading', { name: 'History Data' }).waitFor({ timeout: 8_000 });
  await page.getByRole('button', { name: 'Export CSV' }).waitFor({ timeout: 8_000 });
  await page.getByRole('button', { name: 'Export XLSX' }).waitFor({ timeout: 8_000 });
  result.languageChecks.en = true;
  await page.screenshot({ path: resolve(ROOT, result.screenshots.en), fullPage: true });

  const [csvDownload] = await Promise.all([
    page.waitForEvent('download', { timeout: 8_000 }),
    page.getByRole('button', { name: 'Export CSV' }).click()
  ]);
  result.csv = { filename: csvDownload.suggestedFilename(), ok: csvDownload.suggestedFilename().endsWith('.csv') };
  await csvDownload.delete().catch(() => {});

  const [xlsxDownload] = await Promise.all([
    page.waitForEvent('download', { timeout: 8_000 }),
    page.getByRole('button', { name: 'Export XLSX' }).click()
  ]);
  result.xlsx = { filename: xlsxDownload.suggestedFilename(), ok: xlsxDownload.suggestedFilename().endsWith('.xlsx') };
  await xlsxDownload.delete().catch(() => {});

  await page.evaluate(() => {
    localStorage.setItem('reactoros.vue.language', 'zh');
  });
  await page.reload();
  await page.waitForLoadState('domcontentloaded');
  await page.goto(`${VUE_URL}#/history`);
  await page.waitForLoadState('domcontentloaded');
  await page.getByRole('heading', { name: '历史数据' }).waitFor({ timeout: 8_000 });
  await page.getByRole('button', { name: '导出 CSV' }).waitFor({ timeout: 8_000 });
  await page.getByRole('button', { name: '导出 XLSX' }).waitFor({ timeout: 8_000 });
  result.languageChecks.zh = true;
  await page.screenshot({ path: resolve(ROOT, result.screenshots.zh), fullPage: true });

  result.ok = result.csv.ok && result.xlsx.ok && result.languageChecks.en && result.languageChecks.zh && result.consoleErrors.length === 0;
} catch (error) {
  result.error = error instanceof Error ? error.message : String(error);
} finally {
  await browser.close();
  writeFileSync(resolve(OUT_DIR, 'vue-history-xlsx-export-verification.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  if (!result.ok) process.exit(1);
}
