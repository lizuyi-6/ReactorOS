#!/usr/bin/env node

import { copyFile, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outDir = path.join(root, "output", "acceptance", "field-delivery-local-draft");
const manifestPath = path.join(outDir, "00-summary", "upper_computer_delivery_manifest.json");

const requiredInputs = [
  {
    src: "docs/upper_computer_current_gap_summary_for_lizuyi.md",
    dest: "00-summary/upper_computer_current_gap_summary_for_lizuyi.md",
    kind: "status",
  },
  {
    src: "docs/upper_computer_requirement_gap_matrix.md",
    dest: "00-summary/upper_computer_requirement_gap_matrix.md",
    kind: "status",
  },
  {
    src: "docs/upper_computer_delivery_readiness_index.md",
    dest: "00-summary/upper_computer_delivery_readiness_index.md",
    kind: "status",
  },
  {
    src: "docs/upper_computer_field_delivery_execution_pack.md",
    dest: "00-summary/upper_computer_field_delivery_execution_pack.md",
    kind: "execution_pack",
  },
  {
    src: "docs/upper_computer_field_evidence_checklist.md",
    dest: "00-summary/upper_computer_field_evidence_checklist.md",
    kind: "execution_pack",
  },
  {
    src: "docs/upper_computer_field_evidence_checklist.json",
    dest: "00-summary/upper_computer_field_evidence_checklist.json",
    kind: "execution_pack",
  },
  {
    src: "docs/upper_computer_development_doc.md",
    dest: "docs/upper_computer_development_doc.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_user_manual.md",
    dest: "docs/upper_computer_user_manual.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_test_report.md",
    dest: "docs/upper_computer_test_report.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_test_plan_traceability.md",
    dest: "docs/upper_computer_test_plan_traceability.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_api_acceptance_manual.md",
    dest: "docs/upper_computer_api_acceptance_manual.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_cli_reference.md",
    dest: "docs/upper_computer_cli_reference.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_maintenance_manual.md",
    dest: "docs/upper_computer_maintenance_manual.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_modbus_register_map.md",
    dest: "docs/upper_computer_modbus_register_map.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_rk_deployment_acceptance_guide.md",
    dest: "docs/upper_computer_rk_deployment_acceptance_guide.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_security_key_lifecycle.md",
    dest: "docs/upper_computer_security_key_lifecycle.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_external_acceptance_checklist.md",
    dest: "docs/upper_computer_external_acceptance_checklist.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_visual_evidence_index.md",
    dest: "docs/upper_computer_visual_evidence_index.md",
    kind: "doc",
  },
  {
    src: "docs/upper_computer_training_material_plan.md",
    dest: "01-training/upper_computer_training_material_plan.md",
    kind: "training",
  },
  {
    src: "docs/upper_computer_training_deck.md",
    dest: "01-training/upper_computer_training_deck.md",
    kind: "training",
  },
  {
    src: "docs/upper_computer_training_deck.pptx",
    dest: "01-training/upper_computer_training_deck.pptx",
    kind: "training",
  },
  {
    src: "docs/upper_computer_training_video_storyboard.md",
    dest: "01-training/upper_computer_training_video_storyboard.md",
    kind: "training",
  },
  {
    src: "outputs/manual-20260607-training/video/upper_computer_training_video_draft.mp4",
    dest: "01-training/upper_computer_training_video_draft.mp4",
    kind: "training",
  },
  {
    src: "outputs/manual-20260607-training/video/upper_computer_training_video_manifest.json",
    dest: "01-training/upper_computer_training_video_manifest.json",
    kind: "training",
  },
  {
    src: "docs/upper_computer_training_attendance_and_issues.md",
    dest: "01-training/upper_computer_training_attendance_and_issues.md",
    kind: "training",
  },
  {
    src: "docs/assets/upper-computer-training/README.md",
    dest: "01-training/assets/README.md",
    kind: "training_asset_boundary",
  },
  {
    src: "docs/assets/upper-computer-training/reactor-hmi-system-overview.png",
    dest: "01-training/assets/reactor-hmi-system-overview.png",
    kind: "training_asset",
  },
  {
    src: "docs/assets/upper-computer-training/reactor-workstation-hero.png",
    dest: "01-training/assets/reactor-workstation-hero.png",
    kind: "training_asset",
  },
  {
    src: "docs/assets/upper-computer-training/hmi-safety-operations.png",
    dest: "01-training/assets/hmi-safety-operations.png",
    kind: "training_asset",
  },
  {
    src: "docs/assets/upper-computer-training/acceptance-training-signoff.png",
    dest: "01-training/assets/acceptance-training-signoff.png",
    kind: "training_asset",
  },
  {
    src: "docs/assets/upper-computer-training/industrial-interface-workstation.png",
    dest: "01-training/assets/industrial-interface-workstation.png",
    kind: "training_asset",
  },
  {
    src: "docs/assets/upper-computer-training/safety-interlock-validation.png",
    dest: "01-training/assets/safety-interlock-validation.png",
    kind: "training_asset",
  },
  {
    src: "docs/assets/upper-computer-training/edge-ai-inference-pipeline.png",
    dest: "01-training/assets/edge-ai-inference-pipeline.png",
    kind: "training_asset",
  },
  {
    src: "docs/upper_computer_user_acceptance_script.md",
    dest: "02-uat/upper_computer_user_acceptance_script.md",
    kind: "uat",
  },
  {
    src: "output/playwright/vue-browser-matrix-verification.json",
    dest: "02-uat/vue-browser-matrix-verification.json",
    kind: "gate_report",
    optional: true,
  },
  {
    src: "output/acceptance/training-deliverables-report.json",
    dest: "02-uat/training-deliverables-report.json",
    kind: "gate_report",
    optional: true,
  },
  {
    src: "output/acceptance/acceptance-report.json",
    dest: "02-uat/acceptance-report.json",
    kind: "gate_report",
    optional: true,
  },
  {
    src: "output/acceptance/acceptance-report.md",
    dest: "02-uat/acceptance-report.md",
    kind: "gate_report",
    optional: true,
  },
  {
    src: "outputs/manual-20260607-training/presentations/xingshu-upper-computer-training/output/upper_computer_training_deck_manifest.json",
    dest: "01-training/upper_computer_training_deck_manifest.json",
    kind: "training",
  },
];

const previewInputs = Array.from({ length: 16 }, (_, index) => {
  const name = `slide-${String(index + 1).padStart(2, "0")}.png`;
  return {
    src: `outputs/manual-20260607-training/presentations/xingshu-upper-computer-training/preview/${name}`,
    dest: `01-training/preview/${name}`,
    kind: "training_preview",
  };
});

const frameInputs = ["frame-0005.png", "frame-0235.png", "frame-0505.png", "frame-0735.png"].map((name) => ({
  src: `output/acceptance/training-video-frames/${name}`,
  dest: `01-training/video-frames/${name}`,
  kind: "training_video_frame",
  optional: true,
}));

const allInputs = [...requiredInputs, ...previewInputs, ...frameInputs];

async function copyInput(item) {
  const src = path.join(root, item.src);
  const dest = path.join(outDir, item.dest);
  try {
    const info = await stat(src);
    if (!info.isFile() || info.size <= 0) {
      if (item.optional) {
        return { ...item, status: "skipped", reason: "optional file is missing or empty" };
      }
      throw new Error("file is missing or empty");
    }
    if (item.optional && item.kind === "gate_report" && item.src.includes("acceptance-report")) {
      const text = await readFile(src, "utf8");
      if (text.includes("browserContext.newPage") || text.includes("Firefox") && text.includes("skipped")) {
        return { ...item, status: "skipped", reason: "optional acceptance report has stale Firefox skipped browser-matrix summary" };
      }
    }
    await mkdir(path.dirname(dest), { recursive: true });
    await copyFile(src, dest);
    const content = await readFile(dest);
    return { ...item, status: "copied", bytes: info.size, sha256: createHash("sha256").update(content).digest("hex") };
  } catch (error) {
    if (item.optional) {
      return { ...item, status: "skipped", reason: error.message };
    }
    throw new Error(`${item.src}: ${error.message}`);
  }
}

function byKind(items, kind) {
  return items.filter((item) => item.kind === kind && item.status === "copied").length;
}

function currentCommit() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    return null;
  }
  return result.stdout.trim() || null;
}

async function main() {
  await rm(outDir, { recursive: true, force: true });
  await mkdir(path.dirname(manifestPath), { recursive: true });

  const copied = [];
  for (const item of allInputs) {
    copied.push(await copyInput(item));
  }

  const currentGap = await readFile(path.join(root, "docs/upper_computer_current_gap_summary_for_lizuyi.md"), "utf8");
  const caveats = [
    "This is a local draft delivery package, not a final PRD acceptance package.",
    "Training images are AI-generated visual assets and are not real hardware, real HMI, or signature evidence.",
    "upper_computer_training_video_draft.mp4 is a silent slideshow draft, not real operation screen recording or narrated final training video.",
    "Final acceptance still needs STM32 hardware, real Qwen/GGUF/LoRA/RK evidence, external AINAS/MQTT/Modbus tools, production security/performance evidence, and user signatures.",
  ];
  const manifest = {
    status: "local_draft",
    generatedAt: new Date().toISOString(),
    root,
    outputDir: outDir,
    commit: currentCommit(),
    caveats,
    counts: {
      totalCopied: copied.filter((item) => item.status === "copied").length,
      totalBytes: copied.reduce((sum, item) => sum + (item.status === "copied" ? item.bytes ?? 0 : 0), 0),
      docs: byKind(copied, "doc"),
      training: byKind(copied, "training"),
      trainingPreviews: byKind(copied, "training_preview"),
      trainingAssets: byKind(copied, "training_asset"),
      uat: byKind(copied, "uat"),
      gateReports: byKind(copied, "gate_report"),
    },
    copied,
    remainingGapSummaryExcerpt: currentGap.includes("当前剩余风险主要是")
      ? currentGap.split("当前剩余风险主要是").slice(1).join("当前剩余风险主要是").split("\n")[0].trim()
      : null,
  };

  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  const readback = JSON.parse(await readFile(manifestPath, "utf8"));
  console.log(JSON.stringify({
    outputDir: outDir,
    manifest: manifestPath,
    status: readback.status,
    copied: readback.counts.totalCopied,
  }, null, 2));
}

main().catch((error) => {
  console.error(error.stack || error.message || String(error));
  process.exit(1);
});
