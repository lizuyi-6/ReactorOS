#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");
const workspace = path.join(
  repoRoot,
  "outputs",
  "manual-20260607-training",
  "presentations",
  "xingshu-upper-computer-training",
);
const outputPptx = path.join(repoRoot, "docs", "upper_computer_training_deck.pptx");
const manifestPath = path.join(workspace, "output", "upper_computer_training_deck_manifest.json");
const previewDir = path.join(workspace, "preview");
const assetDir = path.join(repoRoot, "docs", "assets", "upper-computer-training");
const imageAssets = {
  overview: path.join(assetDir, "reactor-hmi-system-overview.png"),
  hero: path.join(assetDir, "reactor-workstation-hero.png"),
  safety: path.join(assetDir, "hmi-safety-operations.png"),
  signoff: path.join(assetDir, "acceptance-training-signoff.png"),
  interface: path.join(assetDir, "industrial-interface-workstation.png"),
  interlock: path.join(assetDir, "safety-interlock-validation.png"),
  edgeAi: path.join(assetDir, "edge-ai-inference-pipeline.png"),
};

const size = { width: 1280, height: 720 };
const C = {
  bg: "#f7f5ef",
  ink: "#17202a",
  muted: "#5f6872",
  line: "#cfc7b5",
  red: "#b34a4a",
  blue: "#2f5d7c",
  green: "#557d62",
  amber: "#b9823f",
  paleBlue: "#dfe9ee",
  paleGreen: "#e3eadf",
  paleAmber: "#efe2cc",
  white: "#ffffff",
};

const slides = [
  {
    kicker: "SYSTEM ROLE",
    title: "上位机已经具备联调准备版能力，但最终验收仍要补外部证据",
    subtitle: "Position the RK/PC edge console, not the entire reactor program, as the current deliverable.",
    body: [
      "负责 Web HMI、REST API、CLI、SQLite 数据、审计链、安全门控、第三方接口和 AI 推荐入口。",
      "对下连接 STM32/Modbus/JSON bridge；对上连接用户、AINAS、MQTT、第三方 REST 和导出文件。",
      "当前不能宣称 PRD 全量完成：真实硬件、真实 Qwen/LoRA、外部平台、安全扫描和签字仍需补证。",
    ],
    proofTitle: "Current evidence",
    proof: [
      ["HMI", "七路由 + 中英 + 浏览器矩阵"],
      ["Safety", "RBAC + safety guard + 审计"],
      ["Interfaces", "REST + AINAS + MQTT + Modbus map"],
    ],
    foot: "Source: docs/upper_computer_current_gap_summary_for_lizuyi.md",
  },
  {
    kicker: "FLOW",
    title: "采集、控制、AI 和审计共用一条受控数据链路",
    subtitle: "Every command path must converge before device write.",
    body: [
      "采集：STM32/样本入口进入 daemon、SQLite、实时监控和历史曲线。",
      "控制：HMI/CLI/AI/AINAS/MQTT/Modbus debug 先过 RBAC、reason 和 safety guard。",
      "追溯：控制、导出、AI 和第三方任务写入 control_events hash chain。",
    ],
    flow: ["Device / STM32", "daemon", "SQLite", "HMI / CLI", "Audit"],
    foot: "Source: docs/upper_computer_development_doc.md",
  },
  {
    kicker: "RBAC",
    title: "日常操作、工程调试和管理员维护必须分权",
    subtitle: "权限通过不是安全通过，所有写入仍受 safety guard 约束。",
    body: [
      "operator 用于日常监控、历史查询和基础操作。",
      "engineer 用于联调诊断，不应作为生产值守账号。",
      "admin 用于高危配置和调试写入，必须留下审计 reason。",
    ],
    table: [
      ["Role", "Typical scope", "Training emphasis"],
      ["operator", "Monitor / History", "日常账号，避免高危调试"],
      ["engineer", "Debug / validation", "调试账号，仍受安全限制"],
      ["admin", "Maintenance / write", "高危账号，必须审计"],
    ],
    foot: "Source: docs/upper_computer_user_manual.md",
  },
  {
    kicker: "MONITOR",
    title: "实时监控页要同时看当前值、趋势、健康和报警",
    subtitle: "Normal data and abnormal data must be visually distinguishable.",
    body: [
      "正常样本应刷新温度、压力、搅拌、流量等指标和趋势图。",
      "越限样本应显示报警级别、类型、当前值、限值和建议。",
      "无新鲜样本时不得误导用户继续按正常状态操作。",
    ],
    proofTitle: "Verified screenshots",
    proof: [
      ["ZH alarm", "Chinese alarm view"],
      ["EN alarm", "English alarm view"],
    ],
    foot: "Source: docs/upper_computer_visual_evidence_index.md",
  },
  {
    kicker: "CONTROL",
    title: "合法目标可以提交，危险目标必须被拒绝并可追溯",
    subtitle: "Safety rejection is a product feature, not a usability bug.",
    body: [
      "范围、单次步长、急停、人工锁、传感器超时和温度-转速禁区同时生效。",
      "HMI、CLI、AI、AINAS、MQTT 和 Modbus 调试写入都不能绕过安全链路。",
      "每次通过或拒绝都应能在审计页定位角色、动作、reason 和结果。",
    ],
    proofTitle: "Safety gates",
    proof: [
      ["Range", "min / max"],
      ["Step", "delta limit"],
      ["Interlock", "E-stop / lock"],
      ["Freshness", "timeout"],
      ["Zone", "temp + stirrer"],
    ],
    foot: "Source: config/safety.toml",
  },
  {
    kicker: "AI",
    title: "AI 页面承担建议和复核，不承担绕过人工审核的自动执行",
    subtitle: "Dry-run, SOP draft and execute must remain visibly separate.",
    body: [
      "AI 推荐可展示目标、rationale、provider/stale/fallback 状态。",
      "AI master-control dry-run 展示决策摘要、动作复核、安全门控和推荐目标。",
      "本地 LoRA 已有数据集导出和训练编排边界，但真实权重、GGUF 和 RK 延迟仍未验收。",
    ],
    proofTitle: "AI readiness boundary",
    proof: [
      ["Done", "dataset export / manifest / promote boundary"],
      ["Pending", "Qwen weights / production adapter / RK report"],
    ],
    hideProof: true,
    foot: "Source: docs/local_ai_adapter_status_addendum.md",
  },
  {
    kicker: "PROCESS",
    title: "工艺探索验收要覆盖从批次开始到结果录入的闭环",
    subtitle: "The page is not complete if it only shows live telemetry.",
    body: [
      "创建或选择批次，记录目标条件和运行状态。",
      "运行期间采集样本，必要时暂停、恢复或停止。",
      "结束后录入产率、产物比例和备注，再导出报告。",
    ],
    flow: ["Prepare", "Start", "Run", "Stop", "Result", "Report"],
    foot: "Source: output/playwright/vue-process-lifecycle-verification.json",
  },
  {
    kicker: "HISTORY",
    title: "历史页已经覆盖查询、结果录入和导出，现场还要补真实数据样本",
    subtitle: "Use History as the acceptance bridge from experiment execution to evidence.",
    body: [
      "批次搜索、状态筛选、产物比例筛选和结果联动已纳入视觉 gate。",
      "产品结果录入、产率、产物比例、目标温度和 CSV/XLSX 下载已验证。",
      "最终验收需要真实实验样本和报告归档。",
    ],
    proofTitle: "Verified outputs",
    proof: [
      ["ZH", "vue-parity-history-zh.png"],
      ["EN", "vue-parity-history-en.png"],
      ["Download", "CSV click covered"],
    ],
    foot: "Source: output/playwright/vue-parity-verification.json",
  },
  {
    kicker: "AUDIT",
    title: "审计链用于回答谁、何时、为什么做了什么",
    subtitle: "Hash-chain integrity is local evidence; production archive is still an external duty.",
    body: [
      "控制、AI、Modbus 写入、第三方任务和导出都应留痕。",
      "审计 CSV 可作为问题复盘和验收归档附件。",
      "生产环境仍需备份、归档、防删和恢复演练。",
    ],
    proofTitle: "Trace fields",
    proof: [
      ["Actor", "role / token"],
      ["Action", "endpoint / command"],
      ["Reason", "non-empty reason"],
      ["Result", "allowed / rejected"],
      ["Hash", "chain window"],
    ],
    foot: "Source: docs/upper_computer_maintenance_manual.md",
  },
  {
    kicker: "MODBUS",
    title: "Modbus 调试页是联调工具，不是生产绕行通道",
    subtitle: "Register writes must remain permissioned, reasoned and safety-checked.",
    body: [
      "寄存器 map 覆盖核心指标和目标点位，但最终地址/单位/缩放系数要硬件确认。",
      "读入口用于核对设备状态；写入口只用于受控调试。",
      "Modbus Poll/Slave 和 STM32 RTU 实机截图仍是 P0 外部验收证据。",
    ],
    proofTitle: "External evidence still needed",
    proof: [
      ["STM32", "final register manual"],
      ["RTU", "RS485 read/write log"],
      ["TCP/TLS", "Modbus Poll/Slave screenshots"],
    ],
    foot: "Source: docs/upper_computer_modbus_register_map.md",
  },
  {
    kicker: "INTEGRATION",
    title: "REST、AINAS 和 MQTT 已有本地路径，外部平台要逐项签收",
    subtitle: "Local protocol coverage is not the same as third-party acceptance.",
    body: [
      "REST API 是 HMI、CLI 和第三方系统共用入口。",
      "AINAS 支持任务创建、查询、执行回执和 AES 静态加密。",
      "MQTT 支持 task、receipt、alert 和 TLS 配置边界，仍需外部 broker 验收。",
    ],
    table: [
      ["Interface", "Local basis", "External proof"],
      ["REST", "API manual", "Postman / third-party logs"],
      ["AINAS", "task + receipt", "real platform screenshots"],
      ["MQTT", "bridge + TLS fields", "broker / reconnect records"],
    ],
    foot: "Source: docs/upper_computer_api_acceptance_manual.md",
  },
  {
    kicker: "CONFIG",
    title: "配置培训要强调可改项、敏感项和变更复验",
    subtitle: "Configuration is part of the safety case.",
    body: [
      "`device.toml` 管设备和采集；`safety.toml` 管范围、步长、禁区和超时。",
      "`integration.toml` 管 AINAS/MQTT/Modbus TCP/TLS；`ai_memory.toml` 管 AI readiness。",
      "数据库加密 key、auth secret、证书私钥和 provider key 不得出现在培训画面中。",
    ],
    proofTitle: "Config set",
    proof: [
      ["Device", "config/device.toml"],
      ["Safety", "config/safety.toml"],
      ["Integration", "config/integration.toml"],
      ["AI", "config/ai_memory.toml"],
    ],
    foot: "Source: docs/upper_computer_security_key_lifecycle.md",
  },
  {
    kicker: "INCIDENTS",
    title: "异常处理先保安全，再定位证据和责任边界",
    subtitle: "Every incident should produce a reproducible record.",
    body: [
      "温度/压力越限时优先现场安全动作，再看上位机报警和审计。",
      "控制被拒绝时先检查权限、reason、安全范围、禁区、急停和人工锁。",
      "AI、MQTT、AINAS 不可用时不应阻断本地核心监控、控制、审计和历史查看。",
    ],
    proofTitle: "Evidence to collect",
    proof: [
      ["Screenshot", "HMI state"],
      ["API", "response body"],
      ["Audit", "event id"],
      ["Log", "daemon / broker / platform"],
    ],
    foot: "Source: docs/upper_computer_maintenance_manual.md",
  },
  {
    kicker: "OPERATIONS",
    title: "部署验收要覆盖启动、健康、备份、恢复和回滚",
    subtitle: "A working demo is not a maintainable production installation.",
    body: [
      "RK/PC 部署要记录版本、SHA256、配置、启动命令和 systemd 状态。",
      "备份要覆盖 SQLite、配置、证书、审计导出和 AI 资产路径。",
      "恢复后必须复测 `/health`、历史数据、产品结果和审计链。",
    ],
    proofTitle: "Current local artifacts",
    proof: [
      ["Guide", "upper_computer_rk_deployment_acceptance_guide.md"],
      ["Drill", "output/acceptance/restore-drill"],
      ["Report", "output/acceptance/acceptance-report.md"],
    ],
    foot: "Source: docs/upper_computer_delivery_readiness_index.md",
  },
  {
    kicker: "ACCEPTANCE",
    title: "用户验收脚本已经准备好，签字前仍只能叫联调准备版",
    subtitle: "UAT converts local evidence into project acceptance.",
    body: [
      "脚本覆盖七大页面、中英、RBAC、监控、报警、控制、AI、批次、历史、审计、Modbus、配置、第三方、多端和部署。",
      "每项都要求步骤、预期结果、证据、问题编号和复测结果。",
      "P0 项未通过或未风险接受时，不应签最终通过。",
    ],
    proofTitle: "UAT artifacts",
    proof: [
      ["Script", "UAT operation script"],
      ["Issues", "training issue log"],
      ["Checklist", "external acceptance checklist"],
    ],
    foot: "Source: docs/upper_computer_user_acceptance_script.md",
  },
  {
    kicker: "FAQ",
    title: "常见问题要按责任边界分流，不要把外部缺口算成 HMI 页面失败",
    subtitle: "Keep acceptance language precise under pressure.",
    body: [
      "页面打不开：查服务、端口、浏览器、证书和网络。",
      "实时数据为空：查 STM32/RS485、样本入口和传感器新鲜度。",
      "AI 不可用：查 StepFun、本地模型权重、adapter、训练脚本和 RK 推理服务。",
      "第三方任务失败：查平台、broker、账号、证书、topic 和任务格式。",
    ],
    proofTitle: "Closeout",
    proof: [
      ["Train", "签到"],
      ["Record", "问题闭环"],
      ["Accept", "执行 UAT"],
      ["Sign", "签字归档"],
    ],
    foot: "Source: docs/upper_computer_training_attendance_and_issues.md",
  },
];

async function importArtifactTool() {
  const home = process.env.HOME || process.env.USERPROFILE || "C:\\Users\\Abraham";
  const entry = path.join(
    home,
    ".cache",
    "codex-runtimes",
    "codex-primary-runtime",
    "dependencies",
    "node",
    "node_modules",
    "@oai",
    "artifact-tool",
    "dist",
    "artifact_tool.mjs",
  );
  return import(pathToFileURL(entry).href);
}

function addShape(slide, opts) {
  const { left, top, width, height, text, ...shapeOpts } = opts;
  const shape = slide.shapes.add(shapeOpts);
  if ([left, top, width, height].every((value) => Number.isFinite(value))) {
    shape.frame = { left, top, width, height };
  }
  if (text !== undefined) {
    shape.text.set(text);
  }
  return shape;
}

function addText(slide, text, frame, style = {}) {
  const shape = addShape(slide, {
    geometry: "rect",
    text,
    ...frame,
    fill: { color: style.fill ?? "transparent" },
    outline: style.outline ?? { color: "transparent", width: 0 },
  });
  shape.text.typeface = style.typeface ?? "Aptos";
  shape.text.fontSize = style.fontSize ?? 22;
  shape.text.color = style.color ?? C.ink;
  shape.text.bold = style.bold ?? false;
  shape.text.alignment = style.alignment ?? "left";
  shape.text.verticalAlignment = style.anchor ?? "top";
  shape.text.insets = style.insets ?? { left: 0, right: 0, top: 0, bottom: 0 };
  return shape;
}

async function addImage(slide, imagePath, frame, fit = "cover") {
  const bytes = await fs.readFile(imagePath);
  const image = slide.images.add({
    blob: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
    fit,
    alt: path.basename(imagePath),
  });
  image.position = frame;
  return image;
}

function addScrim(slide, frame, opacity = 0.72) {
  addShape(slide, {
    geometry: "rect",
    ...frame,
    fill: { color: `rgba(247, 245, 239, ${opacity})` },
    outline: { color: "transparent", width: 0 },
  });
}

function addFooter(slide, n, foot) {
  addText(slide, String(n).padStart(2, "0"), { left: 60, top: 676, width: 42, height: 20 }, {
    fontSize: 11,
    color: C.muted,
    bold: true,
  });
  addShape(slide, {
    geometry: "rect",
    left: 110,
    top: 684,
    width: 730,
    height: 1,
    fill: { color: C.line },
    outline: { color: C.line, width: 0 },
  });
  addText(slide, foot, { left: 860, top: 670, width: 360, height: 28 }, {
    fontSize: 10,
    color: C.muted,
    alignment: "right",
  });
}

function addKicker(slide, text) {
  addShape(slide, {
    geometry: "rect",
    left: 60,
    top: 44,
    width: 12,
    height: 12,
    fill: { color: C.red },
    outline: { color: C.red, width: 0 },
  });
  addText(slide, text, { left: 84, top: 38, width: 260, height: 28 }, {
    fontSize: 12,
    color: C.red,
    bold: true,
    anchor: "middle",
  });
}

function addBody(slide, body) {
  let y = 266;
  for (const item of body) {
    addShape(slide, {
      geometry: "ellipse",
      left: 70,
      top: y + 9,
      width: 7,
      height: 7,
      fill: { color: C.blue },
      outline: { color: C.blue, width: 0 },
    });
    addText(slide, item, { left: 92, top: y, width: 500, height: 58 }, {
      fontSize: 21,
      color: C.ink,
      insets: { left: 0, right: 0, top: 0, bottom: 0 },
    });
    y += 84;
  }
}

function addBodyColumn(slide, body, width = 520, startY = 266) {
  let y = startY;
  for (const item of body) {
    addShape(slide, {
      geometry: "ellipse",
      left: 70,
      top: y + 9,
      width: 7,
      height: 7,
      fill: { color: C.blue },
      outline: { color: C.blue, width: 0 },
    });
    addText(slide, item, { left: 92, top: y, width, height: 62 }, {
      fontSize: 20,
      color: C.ink,
      insets: { left: 0, right: 0, top: 0, bottom: 0 },
    });
    y += 86;
  }
}

function addProof(slide, slideData, index) {
  const x = 660;
  const y = 264;
  addText(slide, slideData.proofTitle ?? "Proof object", { left: x, top: y - 44, width: 470, height: 28 }, {
    fontSize: 14,
    color: C.muted,
    bold: true,
  });
  const items = slideData.proof ?? [];
  const colors = [C.paleBlue, C.paleGreen, C.paleAmber, "#ece6dd", "#e6e1eb"];
  items.forEach(([label, value], i) => {
    const top = y + i * 68;
    addShape(slide, {
      geometry: "rect",
      left: x,
      top,
      width: 500,
      height: 52,
      fill: { color: colors[i % colors.length] },
      outline: { color: C.line, width: 1 },
    });
    addText(slide, label, { left: x + 20, top: top + 9, width: 120, height: 20 }, {
      fontSize: 14,
      color: index % 2 === 0 ? C.blue : C.green,
      bold: true,
    });
    addText(slide, value, { left: x + 150, top: top + 8, width: 320, height: 24 }, {
      fontSize: 17,
      color: C.ink,
      bold: false,
    });
  });
}

function addProofColumn(slide, slideData, index) {
  const x = 92;
  const y = 526;
  addText(slide, slideData.proofTitle ?? "Proof object", { left: x, top: y - 28, width: 470, height: 20 }, {
    fontSize: 12,
    color: C.muted,
    bold: true,
  });
  const items = (slideData.proof ?? []).slice(0, 3);
  items.forEach(([label, value], i) => {
    const left = x + i * 170;
    addShape(slide, {
      geometry: "rect",
      left,
      top: y,
      width: 150,
      height: 58,
      fill: { color: i % 2 === 0 ? C.paleBlue : C.paleGreen },
      outline: { color: C.line, width: 1 },
    });
    addText(slide, label, { left: left + 10, top: y + 8, width: 130, height: 18 }, {
      fontSize: 11,
      color: index % 2 === 0 ? C.blue : C.green,
      bold: true,
    });
    addText(slide, value, { left: left + 10, top: y + 27, width: 130, height: 26 }, {
      fontSize: 12,
      color: C.ink,
    });
  });
}

function addFlow(slide, labels) {
  const startX = 650;
  const y = 338;
  const w = labels.length > 5 ? 84 : 100;
  const gap = labels.length > 5 ? 20 : 28;
  labels.forEach((label, i) => {
    const x = startX + i * (w + gap);
    addShape(slide, {
      geometry: "rect",
      left: x,
      top: y,
      width: w,
      height: 76,
      fill: { color: i % 2 === 0 ? C.paleBlue : C.paleGreen },
      outline: { color: C.line, width: 1 },
    });
    addText(slide, label, { left: x + 8, top: y + 14, width: w - 16, height: 46 }, {
      fontSize: labels.length > 5 ? 13 : 15,
      color: C.ink,
      bold: true,
      alignment: "center",
      anchor: "middle",
    });
    if (i < labels.length - 1) {
      addShape(slide, {
        geometry: "chevron",
        left: x + w + 4,
        top: y + 26,
        width: gap,
        height: 22,
        fill: { color: C.amber },
        outline: { color: C.amber, width: 0 },
      });
    }
  });
}

function addTable(slide, rows) {
  const x = 650;
  const y = 248;
  const widths = [132, 170, 198];
  rows.forEach((row, r) => {
    let left = x;
    row.forEach((cell, c) => {
      addShape(slide, {
        geometry: "rect",
        left,
        top: y + r * 62,
        width: widths[c],
        height: 54,
        fill: { color: r === 0 ? C.ink : r % 2 === 0 ? C.paleGreen : C.paleBlue },
        outline: { color: C.line, width: 1 },
      });
      addText(slide, cell, { left: left + 10, top: y + r * 62 + 9, width: widths[c] - 20, height: 34 }, {
        fontSize: r === 0 ? 13 : 14,
        color: r === 0 ? C.white : C.ink,
        bold: r === 0 || c === 0,
        anchor: "middle",
      });
      left += widths[c];
    });
  });
}

async function main() {
  process.env.HOME = process.env.HOME || "C:\\Users\\Abraham";
  const { Presentation, PresentationFile } = await importArtifactTool();
  const presentation = Presentation.create({ slideSize: size });
  const slideRefs = [];

  for (const [idx, data] of slides.entries()) {
    const slide = presentation.slides.add();
    slideRefs.push(slide);
    const imagePage = idx === 0 || [3, 4, 5, 9, 14, 15].includes(idx);
    addShape(slide, {
      geometry: "rect",
      left: 0,
      top: 0,
      width: size.width,
      height: size.height,
      fill: { color: C.bg },
      outline: { color: C.bg, width: 0 },
    });
    if (idx === 0) {
      await addImage(slide, imageAssets.overview, { left: 720, top: 0, width: 560, height: size.height });
      addScrim(slide, { left: 0, top: 0, width: 760, height: size.height }, 0.97);
    } else if (idx === 3) {
      await addImage(slide, imageAssets.safety, { left: 750, top: 0, width: 530, height: size.height });
      addScrim(slide, { left: 0, top: 0, width: 790, height: size.height }, 0.97);
    } else if (idx === 4) {
      await addImage(slide, imageAssets.interlock, { left: 750, top: 0, width: 530, height: size.height });
      addScrim(slide, { left: 0, top: 0, width: 790, height: size.height }, 0.97);
    } else if (idx === 5) {
      await addImage(slide, imageAssets.edgeAi, { left: 750, top: 0, width: 530, height: size.height });
      addScrim(slide, { left: 0, top: 0, width: 790, height: size.height }, 0.97);
    } else if (idx === 9) {
      await addImage(slide, imageAssets.interface, { left: 750, top: 0, width: 530, height: size.height });
      addScrim(slide, { left: 0, top: 0, width: 790, height: size.height }, 0.97);
    } else if ([14, 15].includes(idx)) {
      await addImage(slide, imageAssets.signoff, { left: 750, top: 0, width: 530, height: size.height });
      addScrim(slide, { left: 0, top: 0, width: 790, height: size.height }, 0.97);
    }
    addShape(slide, {
      geometry: "rect",
      left: 0,
      top: 0,
      width: 24,
      height: size.height,
      fill: { color: idx % 3 === 0 ? C.red : idx % 3 === 1 ? C.blue : C.green },
      outline: { color: "transparent", width: 0 },
    });
    addKicker(slide, data.kicker);
    const titleWidth = imagePage ? 640 : 1080;
    const titleLines = Math.max(1, Math.ceil(data.title.length / (imagePage ? 12 : 30)));
    const titleHeight = Math.min(144, Math.max(104, titleLines * 42));
    const titleFontSize = idx === 0 ? 43 : titleLines >= 3 ? 29 : 34;
    const isLongTitle = titleLines >= 3;
    const subtitleTop = isLongTitle ? 236 : 88 + titleHeight + 8;
    const bodyStartY = isLongTitle ? 304 : 266;
    addText(slide, data.title, { left: 60, top: 88, width: titleWidth, height: titleHeight }, {
      fontSize: titleFontSize,
      color: C.ink,
      bold: true,
      typeface: "Georgia",
      insets: { left: 0, right: 0, top: 0, bottom: 0 },
    });
    addText(slide, data.subtitle, { left: 60, top: idx === 0 ? 198 : subtitleTop, width: 560, height: 40 }, {
      fontSize: 18,
      color: C.muted,
    });
    if (imagePage) {
      addBodyColumn(slide, data.body, 550, bodyStartY);
    } else {
      addBody(slide, data.body);
    }
    if (idx === 15 || data.hideProof) {
      // Closing FAQ slide uses the generated signoff image as the proof object.
    } else if (data.table) {
      addTable(slide, data.table);
    } else if (data.flow) {
      addFlow(slide, data.flow);
    } else if (imagePage && idx !== 15 && !data.hideProof) {
      addProofColumn(slide, data, idx);
    } else {
      addProof(slide, data, idx);
    }
    addFooter(slide, idx + 1, data.foot);
  }

  await fs.mkdir(previewDir, { recursive: true });
  const previewPaths = [];
  for (let index = 0; index < slideRefs.length; index += 1) {
    const previewPath = path.join(previewDir, `slide-${String(index + 1).padStart(2, "0")}.png`);
    const preview = await presentation.export({ slide: slideRefs[index], format: "png", scale: 0.5 });
    await fs.writeFile(previewPath, Buffer.from(await preview.arrayBuffer()));
    previewPaths.push(previewPath);
  }

  await fs.mkdir(path.dirname(outputPptx), { recursive: true });
  const pptx = await PresentationFile.exportPptx(presentation);
  await pptx.save(outputPptx);
  const stat = await fs.stat(outputPptx);

  await fs.mkdir(path.dirname(manifestPath), { recursive: true });
  await fs.writeFile(
    manifestPath,
    `${JSON.stringify(
      {
        output: outputPptx,
        bytes: stat.size,
        slideCount: slides.length,
        slideSize: size,
        source: path.join(repoRoot, "docs", "upper_computer_training_deck.md"),
        imageAssets,
        previewDir,
        previewPaths,
        generatedAt: new Date().toISOString(),
      },
      null,
      2,
    )}\n`,
    "utf8",
  );

  console.log(JSON.stringify({ output: outputPptx, bytes: stat.size, slideCount: slides.length, manifest: manifestPath, previewDir }, null, 2));
}

main().catch((error) => {
  console.error(error.stack || error.message || String(error));
  process.exit(1);
});
