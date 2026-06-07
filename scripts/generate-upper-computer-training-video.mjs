#!/usr/bin/env node

import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const deckScript = path.join(repoRoot, "scripts", "generate-upper-computer-training-deck.mjs");
const previewDir = path.join(
  repoRoot,
  "outputs",
  "manual-20260607-training",
  "presentations",
  "xingshu-upper-computer-training",
  "preview",
);
const videoDir = path.join(repoRoot, "outputs", "manual-20260607-training", "video");
const outputVideo = path.join(videoDir, "upper_computer_training_video_draft.mp4");
const manifestPath = path.join(videoDir, "upper_computer_training_video_manifest.json");
const secondsPerSlide = Number.parseInt(process.env.XINGSHU_TRAINING_VIDEO_SECONDS_PER_SLIDE ?? "30", 10);
const slideCount = 16;

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env: { ...process.env, HOME: process.env.HOME || process.env.USERPROFILE || "" },
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
    ...options,
  });
  if (result.status !== 0) {
    const output = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} ${args.join(" ")} failed with exit ${result.status}: ${output || result.error?.message || "no output"}`);
  }
  return result;
}

async function isNonEmptyFile(file) {
  try {
    const info = await stat(file);
    return info.isFile() && info.size > 0;
  } catch {
    return false;
  }
}

async function ensurePreviews() {
  for (let index = 1; index <= slideCount; index += 1) {
    const file = path.join(previewDir, `slide-${String(index).padStart(2, "0")}.png`);
    if (!(await isNonEmptyFile(file))) {
      run(process.execPath, [deckScript]);
      break;
    }
  }
}

function requireFfmpeg() {
  run("ffmpeg", ["-version"]);
}

function probeVideo() {
  try {
    const result = run("ffprobe", [
      "-v",
      "error",
      "-show_entries",
      "format=duration,size",
      "-of",
      "json",
      outputVideo,
    ]);
    return JSON.parse(result.stdout);
  } catch {
    return null;
  }
}

async function main() {
  if (!Number.isFinite(secondsPerSlide) || secondsPerSlide < 1) {
    throw new Error("XINGSHU_TRAINING_VIDEO_SECONDS_PER_SLIDE must be a positive integer");
  }

  await ensurePreviews();
  requireFfmpeg();
  await mkdir(videoDir, { recursive: true });

  const inputPattern = path.join(previewDir, "slide-%02d.png");
  const totalDurationSeconds = slideCount * secondsPerSlide;
  run("ffmpeg", [
    "-y",
    "-hide_banner",
    "-loglevel",
    "error",
    "-framerate",
    `1/${secondsPerSlide}`,
    "-start_number",
    "1",
    "-i",
    inputPattern,
    "-f",
    "lavfi",
    "-t",
    String(totalDurationSeconds),
    "-i",
    "anullsrc=channel_layout=stereo:sample_rate=48000",
    "-vf",
    "scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,format=yuv420p",
    "-r",
    "30",
    "-c:v",
    "libx264",
    "-preset",
    "veryfast",
    "-crf",
    "23",
    "-c:a",
    "aac",
    "-b:a",
    "64k",
    "-shortest",
    outputVideo,
  ]);

  const videoStat = await stat(outputVideo);
  const manifest = {
    output: outputVideo,
    bytes: videoStat.size,
    status: "draft_silent_slideshow",
    caveat: "This MP4 is a silent training deck draft, not a real现场操作录屏, not narrated training, and not user signoff evidence.",
    slideCount,
    secondsPerSlide,
    estimatedDurationSeconds: totalDurationSeconds,
    sourcePreviewDir: previewDir,
    sourcePreviewPaths: Array.from({ length: slideCount }, (_, index) =>
      path.join(previewDir, `slide-${String(index + 1).padStart(2, "0")}.png`),
    ),
    deckPptx: path.join(repoRoot, "docs", "upper_computer_training_deck.pptx"),
    storyboard: path.join(repoRoot, "docs", "upper_computer_training_video_storyboard.md"),
    generatedAt: new Date().toISOString(),
    ffprobe: probeVideo(),
  };
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

  // Touch-read the manifest to catch accidental encoding or write errors early.
  JSON.parse(await readFile(manifestPath, "utf8"));
  console.log(JSON.stringify({ output: outputVideo, bytes: videoStat.size, manifest: manifestPath }, null, 2));
}

main().catch((error) => {
  console.error(error.stack || error.message || String(error));
  process.exit(1);
});
