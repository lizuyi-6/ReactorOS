#!/usr/bin/env node

import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const reportPath = path.join(root, "output", "acceptance", "training-deliverables-report.json");

const generatedManifestFile = "outputs/manual-20260607-training/presentations/xingshu-upper-computer-training/output/upper_computer_training_deck_manifest.json";
const expectedPreviewDir = "outputs/manual-20260607-training/presentations/xingshu-upper-computer-training/preview";
const generatedVideoFile = "outputs/manual-20260607-training/video/upper_computer_training_video_draft.mp4";
const generatedVideoManifestFile = "outputs/manual-20260607-training/video/upper_computer_training_video_manifest.json";
const generatedDeliveryManifestFile = "output/acceptance/field-delivery-local-draft/00-summary/upper_computer_delivery_manifest.json";
const browserMatrixReportFile = "output/playwright/vue-browser-matrix-verification.json";

const requiredSourceFiles = [
  "docs/upper_computer_training_deck.md",
  "docs/upper_computer_user_acceptance_script.md",
  "docs/upper_computer_training_attendance_and_issues.md",
  "docs/upper_computer_field_delivery_execution_pack.md",
  "docs/upper_computer_field_evidence_checklist.md",
  "docs/upper_computer_field_evidence_checklist.json",
  "docs/upper_computer_training_video_storyboard.md",
  "docs/assets/upper-computer-training/README.md",
  "docs/assets/upper-computer-training/reactor-hmi-system-overview.png",
  "docs/assets/upper-computer-training/reactor-workstation-hero.png",
  "docs/assets/upper-computer-training/hmi-safety-operations.png",
  "docs/assets/upper-computer-training/acceptance-training-signoff.png",
  "docs/assets/upper-computer-training/industrial-interface-workstation.png",
  "docs/assets/upper-computer-training/safety-interlock-validation.png",
  "docs/assets/upper-computer-training/edge-ai-inference-pipeline.png",
  browserMatrixReportFile,
  "scripts/generate-upper-computer-training-deck.mjs",
  "scripts/generate-upper-computer-training-video.mjs",
  "scripts/package-upper-computer-delivery.mjs",
];

const generatedFiles = [
  "docs/upper_computer_training_deck.pptx",
  generatedManifestFile,
  generatedVideoFile,
  generatedVideoManifestFile,
  generatedDeliveryManifestFile,
];

const requiredFiles = [...requiredSourceFiles, ...generatedFiles];

const requiredTextMarkers = [
  {
    file: "docs/upper_computer_training_deck.md",
    markers: [
      "上位机系统定位",
      "登录、角色和权限",
      "实时监控",
      "手动控制",
      "AI 建议",
      "Modbus 调试",
      "用户验收",
    ],
  },
  {
    file: "docs/upper_computer_user_acceptance_script.md",
    markers: ["UAT", "验收", "证据", "签字"],
  },
  {
    file: "docs/upper_computer_training_attendance_and_issues.md",
    markers: ["签到", "问题", "闭环", "签字"],
  },
  {
    file: "docs/upper_computer_field_delivery_execution_pack.md",
    markers: ["交付", "证据", "签字"],
  },
  {
    file: "docs/upper_computer_field_evidence_checklist.md",
    markers: ["local_ready", "external_required", "signature_required", "draft_only"],
  },
  {
    file: "docs/upper_computer_training_video_storyboard.md",
    markers: ["静音课件轮播草稿", "不是真实现场录屏", "最终现场版要求", "XINGSHU_TRAINING_VIDEO_SECONDS_PER_SLIDE"],
  },
  {
    file: "docs/assets/upper-computer-training/README.md",
    markers: [
      "reactor-workstation-hero.png",
      "reactor-hmi-system-overview.png",
      "industrial-interface-workstation.png",
      "safety-interlock-validation.png",
      "edge-ai-inference-pipeline.png",
      "不是实机照片",
      "不是真实 HMI 截图",
      "不是真实签字",
      "不能作为 PRD 验收证据",
    ],
  },
];

const failures = [];
const checks = [];
const generated = {
  deck: {
    attempted: false,
    exitCode: null,
    reason: null,
  },
  video: {
    attempted: false,
    exitCode: null,
    reason: null,
  },
  deliveryPackage: {
    attempted: false,
    exitCode: null,
    reason: null,
  },
};

function rel(value) {
  return path.relative(root, value).replaceAll(path.sep, "/");
}

function normalizeManifestPath(value) {
  if (!value || typeof value !== "string") return null;
  if (path.isAbsolute(value)) return value;
  if (/^[A-Za-z]:[\\/]/.test(value)) return value;
  return path.join(root, value);
}

async function requireNonEmptyFile(file) {
  const fullPath = path.join(root, file);
  try {
    const stats = await stat(fullPath);
    if (!stats.isFile()) {
      failures.push(`${file} exists but is not a file`);
      return null;
    }
    if (stats.size <= 0) {
      failures.push(`${file} is empty`);
      return null;
    }
    checks.push({ name: file, status: "ok", bytes: stats.size });
    return stats;
  } catch (error) {
    failures.push(`${file} is missing: ${error.message}`);
    return null;
  }
}

async function isNonEmptyFile(file) {
  try {
    const stats = await stat(path.join(root, file));
    return stats.isFile() && stats.size > 0;
  } catch {
    return false;
  }
}

async function hasGeneratedDeckOutputs() {
  if (!(await isNonEmptyFile("docs/upper_computer_training_deck.pptx"))) {
    return false;
  }
  if (!(await isNonEmptyFile(generatedManifestFile))) {
    return false;
  }
  for (let index = 0; index < 16; index += 1) {
    const expectedName = `slide-${String(index + 1).padStart(2, "0")}.png`;
    if (!(await isNonEmptyFile(path.join(expectedPreviewDir, expectedName)))) {
      return false;
    }
  }
  return true;
}

async function ensureGeneratedDeckOutputs() {
  if (await hasGeneratedDeckOutputs()) {
    generated.deck.reason = "existing PPTX, manifest, and previews found";
    return;
  }

  generated.deck.attempted = true;
  generated.deck.reason = "PPTX, manifest, or previews missing; regenerated from source";
  const env = { ...process.env };
  if (!env.HOME && env.USERPROFILE) {
    env.HOME = env.USERPROFILE;
  }
  const result = spawnSync(process.execPath, ["scripts/generate-upper-computer-training-deck.mjs"], {
    cwd: root,
    env,
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
  });
  generated.deck.exitCode = result.status;
  if (result.status !== 0) {
    const output = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    failures.push(`training deck regeneration failed with exit ${result.status}: ${output || result.error?.message || "no output"}`);
  }
}

async function hasGeneratedVideoOutputs() {
  return (await isNonEmptyFile(generatedVideoFile)) && (await isNonEmptyFile(generatedVideoManifestFile));
}

async function ensureGeneratedVideoOutputs() {
  if (await hasGeneratedVideoOutputs()) {
    generated.video.reason = "existing video draft and manifest found";
    return;
  }

  generated.video.attempted = true;
  generated.video.reason = "video draft or manifest missing; regenerated from slide previews";
  const env = { ...process.env };
  if (!env.HOME && env.USERPROFILE) {
    env.HOME = env.USERPROFILE;
  }
  const result = spawnSync(process.execPath, ["scripts/generate-upper-computer-training-video.mjs"], {
    cwd: root,
    env,
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
  });
  generated.video.exitCode = result.status;
  if (result.status !== 0) {
    const output = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    failures.push(`training video draft regeneration failed with exit ${result.status}: ${output || result.error?.message || "no output"}`);
  }
}

async function hasGeneratedDeliveryPackage() {
  return await isNonEmptyFile(generatedDeliveryManifestFile);
}

async function ensureGeneratedDeliveryPackage() {
  if (await hasGeneratedDeliveryPackage()) {
    generated.deliveryPackage.reason = "existing local draft delivery package manifest found";
    return;
  }

  generated.deliveryPackage.attempted = true;
  generated.deliveryPackage.reason = "local draft delivery package manifest missing; regenerated package";
  const result = spawnSync(process.execPath, ["scripts/package-upper-computer-delivery.mjs"], {
    cwd: root,
    env: { ...process.env, HOME: process.env.HOME || process.env.USERPROFILE || "" },
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
  });
  generated.deliveryPackage.exitCode = result.status;
  if (result.status !== 0) {
    const output = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    failures.push(`local draft delivery package regeneration failed with exit ${result.status}: ${output || result.error?.message || "no output"}`);
  }
}

async function readText(file) {
  try {
    return await readFile(path.join(root, file), "utf8");
  } catch (error) {
    failures.push(`${file} cannot be read as UTF-8: ${error.message}`);
    return "";
  }
}

function decodeZipName(buffer, flags) {
  return flags & 0x0800 ? buffer.toString("utf8") : buffer.toString("latin1");
}

function parseZipEntries(buffer) {
  const eocdSignature = 0x06054b50;
  const centralDirectorySignature = 0x02014b50;
  const searchStart = Math.max(0, buffer.length - 0xffff - 22);
  let eocdOffset = -1;
  for (let offset = buffer.length - 22; offset >= searchStart; offset -= 1) {
    if (buffer.readUInt32LE(offset) === eocdSignature) {
      eocdOffset = offset;
      break;
    }
  }
  if (eocdOffset < 0) {
    throw new Error("cannot find ZIP end-of-central-directory record");
  }

  const entryCount = buffer.readUInt16LE(eocdOffset + 10);
  const centralDirectoryOffset = buffer.readUInt32LE(eocdOffset + 16);
  const entries = [];
  let offset = centralDirectoryOffset;
  for (let index = 0; index < entryCount; index += 1) {
    if (buffer.readUInt32LE(offset) !== centralDirectorySignature) {
      throw new Error(`invalid central-directory signature at offset ${offset}`);
    }
    const flags = buffer.readUInt16LE(offset + 8);
    const compressedSize = buffer.readUInt32LE(offset + 20);
    const uncompressedSize = buffer.readUInt32LE(offset + 24);
    const nameLength = buffer.readUInt16LE(offset + 28);
    const extraLength = buffer.readUInt16LE(offset + 30);
    const commentLength = buffer.readUInt16LE(offset + 32);
    const nameStart = offset + 46;
    const nameEnd = nameStart + nameLength;
    entries.push({
      name: decodeZipName(buffer.subarray(nameStart, nameEnd), flags),
      compressedSize,
      uncompressedSize,
    });
    offset = nameEnd + extraLength + commentLength;
  }
  return entries;
}

async function verifyPptx() {
  const pptxPath = path.join(root, "docs", "upper_computer_training_deck.pptx");
  try {
    const buffer = await readFile(pptxPath);
    const entries = parseZipEntries(buffer);
    const slides = entries.filter((entry) => /^ppt\/slides\/slide\d+\.xml$/.test(entry.name));
    const media = entries.filter((entry) => entry.name.startsWith("ppt/media/") && !entry.name.endsWith("/"));
    const emptyEntries = entries.filter((entry) => !entry.name.endsWith("/") && entry.uncompressedSize === 0);
    if (slides.length !== 16) {
      failures.push(`docs/upper_computer_training_deck.pptx must contain 16 slide XML files, found ${slides.length}`);
    }
    if (media.length < 3) {
      failures.push(`docs/upper_computer_training_deck.pptx must contain at least 3 media files, found ${media.length}`);
    }
    if (emptyEntries.length > 0) {
      failures.push(`docs/upper_computer_training_deck.pptx contains empty ZIP entries: ${emptyEntries.map((entry) => entry.name).join(", ")}`);
    }
    checks.push({
      name: "docs/upper_computer_training_deck.pptx",
      status: slides.length === 16 && media.length >= 3 && emptyEntries.length === 0 ? "ok" : "fail",
      entries: entries.length,
      slideXmlCount: slides.length,
      mediaCount: media.length,
      emptyEntryCount: emptyEntries.length,
    });
  } catch (error) {
    failures.push(`docs/upper_computer_training_deck.pptx package check failed: ${error.message}`);
  }
}

async function verifyTextMarkers() {
  for (const item of requiredTextMarkers) {
    const text = await readText(item.file);
    const missing = item.markers.filter((marker) => !text.includes(marker));
    if (missing.length > 0) {
      failures.push(`${item.file} is missing required markers: ${missing.join(", ")}`);
    }
    checks.push({
      name: `${item.file} text markers`,
      status: missing.length === 0 ? "ok" : "fail",
      checked: item.markers.length,
      missing,
    });
  }
}

async function verifyFieldEvidenceChecklistJson() {
  let payload;
  try {
    payload = JSON.parse(await readFile(path.join(root, "docs/upper_computer_field_evidence_checklist.json"), "utf8"));
  } catch (error) {
    failures.push(`docs/upper_computer_field_evidence_checklist.json cannot be parsed: ${error.message}`);
    return;
  }
  const items = Array.isArray(payload.items) ? payload.items : [];
  const requiredStatuses = ["local_ready", "external_required", "signature_required", "draft_only"];
  const statusSet = new Set(items.map((item) => item.status));
  for (const status of requiredStatuses) {
    if (!statusSet.has(status)) {
      failures.push(`docs/upper_computer_field_evidence_checklist.json must include at least one ${status} item`);
    }
  }
  if (items.length < 15) {
    failures.push(`docs/upper_computer_field_evidence_checklist.json must include at least 15 field evidence items, found ${items.length}`);
  }
  checks.push({
    name: "docs/upper_computer_field_evidence_checklist.json",
    status: items.length >= 15 && requiredStatuses.every((status) => statusSet.has(status)) ? "ok" : "fail",
    itemCount: items.length,
    statuses: Object.fromEntries(requiredStatuses.map((status) => [status, items.filter((item) => item.status === status).length])),
  });
}

async function verifyManifest() {
  const manifestFile = generatedManifestFile;
  let manifest;
  try {
    manifest = JSON.parse(await readFile(path.join(root, manifestFile), "utf8"));
  } catch (error) {
    failures.push(`${manifestFile} cannot be parsed: ${error.message}`);
    return;
  }

  if (manifest.slideCount !== 16) {
    failures.push(`${manifestFile} slideCount must be 16, found ${manifest.slideCount}`);
  }
  if (!manifest.output || rel(path.resolve(manifest.output)) !== "docs/upper_computer_training_deck.pptx") {
    failures.push(`${manifestFile} output must point to docs/upper_computer_training_deck.pptx`);
  }
  const imageAssetKeys = ["overview", "hero", "safety", "signoff", "interface", "interlock", "edgeAi"];
  for (const key of imageAssetKeys) {
    const assetPath = normalizeManifestPath(manifest.imageAssets?.[key]);
    if (!assetPath) {
      failures.push(`${manifestFile} imageAssets.${key} is missing`);
      continue;
    }
    try {
      const stats = await stat(assetPath);
      if (!stats.isFile() || stats.size <= 0) {
        failures.push(`${manifestFile} imageAssets.${key} is not a non-empty file: ${assetPath}`);
      }
    } catch (error) {
      failures.push(`${manifestFile} imageAssets.${key} does not exist: ${assetPath}: ${error.message}`);
    }
  }

  const previewPaths = Array.isArray(manifest.previewPaths) ? manifest.previewPaths : [];
  if (previewPaths.length !== 16) {
    failures.push(`${manifestFile} previewPaths must contain 16 entries, found ${previewPaths.length}`);
  }
  for (let index = 0; index < 16; index += 1) {
    const expectedName = `slide-${String(index + 1).padStart(2, "0")}.png`;
    const previewPath = normalizeManifestPath(previewPaths[index] ?? path.join(expectedPreviewDir, expectedName));
    if (!previewPath || path.basename(previewPath) !== expectedName) {
      failures.push(`${manifestFile} preview ${index + 1} must be named ${expectedName}`);
      continue;
    }
    try {
      const stats = await stat(previewPath);
      if (!stats.isFile() || stats.size <= 0) {
        failures.push(`${expectedName} preview is not a non-empty file`);
      }
    } catch (error) {
      failures.push(`${expectedName} preview is missing: ${error.message}`);
    }
  }

  checks.push({
    name: manifestFile,
    status: manifest.slideCount === 16 && previewPaths.length === 16 ? "ok" : "fail",
    slideCount: manifest.slideCount,
    previewCount: previewPaths.length,
    imageAssetKeys,
  });
}

async function verifyVideoManifest() {
  let manifest;
  try {
    manifest = JSON.parse(await readFile(path.join(root, generatedVideoManifestFile), "utf8"));
  } catch (error) {
    failures.push(`${generatedVideoManifestFile} cannot be parsed: ${error.message}`);
    return;
  }

  if (manifest.status !== "draft_silent_slideshow") {
    failures.push(`${generatedVideoManifestFile} status must be draft_silent_slideshow, found ${manifest.status}`);
  }
  if (manifest.slideCount !== 16) {
    failures.push(`${generatedVideoManifestFile} slideCount must be 16, found ${manifest.slideCount}`);
  }
  if (!manifest.caveat || !manifest.caveat.includes("not a real") || !manifest.caveat.includes("signoff")) {
    failures.push(`${generatedVideoManifestFile} must state that the MP4 is not real现场操作录屏 or signoff evidence`);
  }
  const videoStats = await requireNonEmptyFile(generatedVideoFile);
  if (videoStats && videoStats.size < 100000) {
    failures.push(`${generatedVideoFile} is unexpectedly small for a 16-slide MP4 draft: ${videoStats.size} bytes`);
  }
  checks.push({
    name: generatedVideoManifestFile,
    status: manifest.status === "draft_silent_slideshow" && manifest.slideCount === 16 ? "ok" : "fail",
    slideCount: manifest.slideCount,
    secondsPerSlide: manifest.secondsPerSlide,
    estimatedDurationSeconds: manifest.estimatedDurationSeconds,
    ffprobeDuration: manifest.ffprobe?.format?.duration ?? null,
  });
}

async function verifyDeliveryManifest() {
  let manifest;
  try {
    manifest = JSON.parse(await readFile(path.join(root, generatedDeliveryManifestFile), "utf8"));
  } catch (error) {
    failures.push(`${generatedDeliveryManifestFile} cannot be parsed: ${error.message}`);
    return;
  }
  if (manifest.status !== "local_draft") {
    failures.push(`${generatedDeliveryManifestFile} status must be local_draft, found ${manifest.status}`);
  }
  if ((manifest.counts?.totalCopied ?? 0) < 40) {
    failures.push(`${generatedDeliveryManifestFile} totalCopied is unexpectedly low: ${manifest.counts?.totalCopied}`);
  }
  if ((manifest.counts?.totalBytes ?? 0) < 1000000) {
    failures.push(`${generatedDeliveryManifestFile} totalBytes is unexpectedly low: ${manifest.counts?.totalBytes}`);
  }
  if (!manifest.commit || typeof manifest.commit !== "string" || manifest.commit.length < 12) {
    failures.push(`${generatedDeliveryManifestFile} must record the source git commit`);
  }
  const copied = Array.isArray(manifest.copied) ? manifest.copied.filter((item) => item.status === "copied") : [];
  const missingSha = copied.filter((item) => !/^[0-9a-f]{64}$/i.test(item.sha256 ?? ""));
  if (missingSha.length > 0) {
    failures.push(`${generatedDeliveryManifestFile} copied entries missing sha256: ${missingSha.map((item) => item.dest).slice(0, 5).join(", ")}`);
  }
  const caveatText = Array.isArray(manifest.caveats) ? manifest.caveats.join("\n") : "";
  for (const marker of ["not a final PRD acceptance package", "AI-generated visual assets", "silent slideshow draft", "user signatures"]) {
    if (!caveatText.includes(marker)) {
      failures.push(`${generatedDeliveryManifestFile} caveats must include ${JSON.stringify(marker)}`);
    }
  }
  checks.push({
    name: generatedDeliveryManifestFile,
    status: manifest.status === "local_draft" && (manifest.counts?.totalCopied ?? 0) >= 40 && missingSha.length === 0 ? "ok" : "fail",
    totalCopied: manifest.counts?.totalCopied ?? null,
    totalBytes: manifest.counts?.totalBytes ?? null,
    commit: manifest.commit ?? null,
    copiedWithSha256: copied.length - missingSha.length,
    trainingPreviews: manifest.counts?.trainingPreviews ?? null,
    gateReports: manifest.counts?.gateReports ?? null,
  });
}

async function verifyBrowserMatrixReport() {
  let report;
  try {
    report = JSON.parse(await readFile(path.join(root, browserMatrixReportFile), "utf8"));
  } catch (error) {
    failures.push(`${browserMatrixReportFile} cannot be parsed: ${error.message}`);
    return;
  }

  const expectedBrowsers = ["chromium", "chrome", "msedge", "firefox", "webkit"];
  const browsers = Array.isArray(report.browsers) ? report.browsers : [];
  const byName = new Map(browsers.map((browser) => [browser.name, browser]));
  const missing = expectedBrowsers.filter((name) => !byName.has(name));
  const failed = [];
  let totalPageChecks = 0;

  for (const name of expectedBrowsers) {
    const browser = byName.get(name);
    if (!browser) continue;
    const pages = Array.isArray(browser.pages) ? browser.pages : [];
    totalPageChecks += pages.length;
    if (browser.status !== "ok" || browser.ok !== true || browser.skipped === true || pages.length < 14) {
      failed.push(name);
    }
  }

  const unexpectedConsoleMessages = Array.isArray(report.unexpectedConsoleMessages)
    ? report.unexpectedConsoleMessages.length
    : null;

  if (report.ok !== true) {
    failures.push(`${browserMatrixReportFile} ok must be true`);
  }
  if (report.strictAllBrowsers !== true) {
    failures.push(`${browserMatrixReportFile} strictAllBrowsers must be true`);
  }
  if (missing.length > 0) {
    failures.push(`${browserMatrixReportFile} missing browser results: ${missing.join(", ")}`);
  }
  if (failed.length > 0) {
    failures.push(`${browserMatrixReportFile} browsers are not fully passed: ${failed.join(", ")}`);
  }
  if (unexpectedConsoleMessages !== 0) {
    failures.push(`${browserMatrixReportFile} unexpectedConsoleMessages must be an empty array`);
  }
  if (totalPageChecks < 70) {
    failures.push(`${browserMatrixReportFile} must include at least 70 route/language page checks, found ${totalPageChecks}`);
  }

  checks.push({
    name: browserMatrixReportFile,
    status:
      report.ok === true &&
      report.strictAllBrowsers === true &&
      missing.length === 0 &&
      failed.length === 0 &&
      unexpectedConsoleMessages === 0 &&
      totalPageChecks >= 70
        ? "ok"
        : "fail",
    expectedBrowsers,
    browserCount: browsers.length,
    totalPageChecks,
    unexpectedConsoleMessages,
  });
}

for (const file of requiredSourceFiles) {
  await requireNonEmptyFile(file);
}
await ensureGeneratedDeckOutputs();
await ensureGeneratedVideoOutputs();
await ensureGeneratedDeliveryPackage();
for (const file of generatedFiles) {
  await requireNonEmptyFile(file);
}
await verifyTextMarkers();
await verifyFieldEvidenceChecklistJson();
await verifyPptx();
await verifyManifest();
await verifyVideoManifest();
await verifyBrowserMatrixReport();
await verifyDeliveryManifest();

const report = {
  status: failures.length === 0 ? "ok" : "fail",
  generatedAt: new Date().toISOString(),
  root,
  checkedFiles: requiredFiles,
  reportPath,
  generated,
  checks,
  failures,
};

await mkdir(path.dirname(reportPath), { recursive: true });
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");

if (failures.length > 0) {
  console.error(failures.join("\n"));
  console.error(`training deliverables report -> ${reportPath}`);
  process.exit(1);
}

console.log("Training deliverables gate passed");
console.log(`training deliverables report -> ${reportPath}`);
