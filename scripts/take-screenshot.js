import { chromium } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");

async function main() {
  console.log("Launching headless chromium browser...");
  const browser = await chromium.launch();
  const page = await browser.newPage();
  
  // Use a standard desktop viewport.
  await page.setViewportSize({ width: 1440, height: 900 });
  
  // Load the local static landing page.
  const filePath = `file://${path.join(repoRoot, "static", "landing.html").replace(/\\/g, "/")}`;
  console.log(`Navigating to: ${filePath}`);
  await page.goto(filePath);
  
  // Wait for fonts and subtle animations to finish rendering.
  console.log("Waiting for rendering and animations...");
  await page.waitForTimeout(2500);
  
  // Capture a full-page screenshot.
  const outputPath = path.join(repoRoot, "output", "landing-fullpage.png");
  console.log(`Saving full page screenshot to: ${outputPath}`);
  await page.screenshot({ path: outputPath, fullPage: true });
  
  await browser.close();
  console.log("Screenshot successfully captured!");
}

main().catch((err) => {
  console.error("Screenshot capture failed:", err);
  process.exit(1);
});
