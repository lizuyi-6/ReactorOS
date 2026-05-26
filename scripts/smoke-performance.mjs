import { chromium } from "@playwright/test";

const baseUrl = "http://127.0.0.1:8000";
const sample = {
  temperature_c: 31.11,
  pressure_mpa: 0.5,
  stirrer_rpm: 125.18,
  shake_speed_cpm: 30,
  tilt_state: 1,
  flow_rate_l_min: 2.42,
  product_concentration_percent: 11.1,
  ph: 6.15,
};

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const consoleErrors = [];
const pageErrors = [];
const liveUrls = [];

async function injectSample() {
  const response = await fetch(`${baseUrl}/api/v1/reactor/reactor_001/samples`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(sample),
  });
  if (!response.ok) {
    throw new Error(`sample injection failed ${response.status}: ${await response.text()}`);
  }
}

await injectSample();
const sampleTimer = setInterval(() => {
  injectSample().catch(() => {});
}, 1000);

page.on("console", (msg) => {
  if (msg.type() === "error") consoleErrors.push(msg.text());
});
page.on("pageerror", (err) => pageErrors.push(err.message));
page.on("request", (req) => {
  if (req.url().includes("/api/live")) liveUrls.push(req.url());
});

await page.goto(`${baseUrl}/`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(3500);

const result = await page.evaluate(async () => {
  const state = window.__reactorState;
  const canvas = document.querySelector("#mainChart");
  const ctx = canvas?.getContext("2d");
  let nonBlank = false;
  if (ctx && canvas.width && canvas.height) {
    const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] !== 0) {
        nonBlank = true;
        break;
      }
    }
  }

  document.querySelector(".nav-settings")?.click();
  await new Promise((resolve) => setTimeout(resolve, 800));
  const settingsText = document.querySelector("#view-settings")?.innerText || "";
  return {
    dataReady: state?.dataReady,
    sensorCount: state?.sensors?.length,
    activeTab: state?.activeTab,
    temp: state?.sensors?.find((s) => s.key === "temperature")?.value,
    historyLen: state?.history?.length ?? state?.samples?.length ?? state?.recentSamples?.length,
    nonBlank,
    settingsHasDevices:
      settingsText.includes("Device") ||
      settingsText.includes("component") ||
      settingsText.includes("Connected"),
  };
});

clearInterval(sampleTimer);
await browser.close();
const report = { result, liveUrls, consoleErrors, pageErrors };
console.log(JSON.stringify(report, null, 2));

if (consoleErrors.length || pageErrors.length || !result.nonBlank) process.exit(1);
if (!liveUrls.some((url) => url.includes("sample_limit=60") && url.includes("include_events=false"))) {
  process.exit(2);
}
