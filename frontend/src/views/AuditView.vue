<script setup lang="ts">
// Audit Trail / 审计追踪 — 参考稿 4 号页面完整重写。
// 数据源：auditApi.logs({page,pageSize,eventType}) → AuditLogsResponse{events, total, chain}。
// actor/role 由后端 control_events 提供（system 表示控制环/内部事件）；severity/IP 后端暂无，不编造。
// Export Report 无后端端点：用当前页真实事件 + 链状态在客户端生成 Markdown 报告。
import { computed, onMounted, reactive, ref, watch } from "vue";
import { ElMessage } from "element-plus";
import PanelCard from "../components/PanelCard.vue";
import AppIcon from "../components/AppIcon.vue";
import { auditApi } from "../api";
import { downloadBlob } from "../api/http";
import { errorMessage } from "../api/errors";
import { useLanguage } from "../i18n";
import { formatTimestamp } from "../utils/format";
import type { AuditChainStatus, ControlEvent } from "../api/types";

const { tr, language } = useLanguage();

// ---------- 状态 ----------
const loading = ref(false);
const exporting = ref(false);
const events = ref<ControlEvent[]>([]);
const chain = ref<AuditChainStatus | null>(null);
const total = ref(0);
const page = ref(1);
const pageSize = ref(50);
const selected = ref<ControlEvent | null>(null);
const chainDialog = ref(false);
const appliedEventType = ref("");
const eventTypeOptions = ref<string[]>([]);

const filters = reactive({
  eventType: "",
  dateRange: null as string[] | null,
  batchId: ""
});

// ---------- 事件类型双语映射（取自后端 src/api.rs / main.rs 全部 record 点） ----------
const ACTION_LABELS: Record<string, [string, string]> = {
  update_targets: ["Update Setpoint", "更新设定值"],
  operator_targets_updated: ["Update Setpoint", "更新设定值"],
  ai_targets_updated: ["AI Setpoint Update", "AI 设定值更新"],
  ainas_targets_updated: ["AINAS Setpoint Update", "AINAS 设定值更新"],
  ai_master_decision: ["AI Master Decision", "AI 主控决策"],
  auto_enabled: ["Auto Control Change", "自动控制切换"],
  automatic_final_interlock_blocked: ["Interlock Blocked", "联锁拦截"],
  batch_started: ["Batch Start", "批次开始"],
  batch_finished: ["Batch Finish", "批次结束"],
  batch_finish_recovery_missing_batch: ["Batch Recovery", "批次恢复"],
  process_start_failed: ["Process Start Failed", "工艺启动失败"],
  process_stop_recovery_missing_batch: ["Process Stop Recovery", "工艺停止恢复"],
  unfinished_batch_recovery_auto_blocked: ["Recovery Auto-Block", "恢复自动闭锁"],
  v1_control_accepted: ["V1 Control Accepted", "V1 控制受理"],
  v1_process_loaded: ["V1 Process Loaded", "V1 工艺载入"],
  device_write: ["Device Write", "设备写入"],
  device_write_failed: ["Device Write Failed", "设备写入失败"],
  downstream_command_fault: ["Downstream Fault", "下游指令故障"],
  control_fault_auto_disabled: ["Fault Auto-Disable", "故障自动停用"],
  control_fault_reset: ["Fault Reset", "故障复位"],
  control_loop_terminated: ["Control Loop Stop", "控制环终止"],
  field_input_fault_auto_disabled: ["Field Input Fault", "现场输入故障"],
  high_sensor_alarm_auto_disabled: ["Alarm Auto-Disable", "告警自动停用"],
  emergency_stop: ["Emergency Stop", "紧急停止"],
  emergency_stop_reset: ["Emergency Stop Reset", "紧急停止复位"],
  manual_lock_on: ["Manual Lock", "人工上锁"],
  manual_lock_off: ["Manual Lock Release", "人工解锁"],
  manual_unlock_refused: ["Unlock Refused", "解锁被拒"],
  modbus_register_write: ["Modbus Write", "Modbus 写入"],
  demo_seed_applied: ["Demo Seed", "演示数据种子"]
};

function actionDual(eventType: string): string {
  const pair = ACTION_LABELS[eventType];
  if (!pair) return eventType;
  // V19：中文模式 "English 中文" 双语并列；英文模式仅英文（不再残留中文）
  return language.value === "zh" ? `${pair[0]} ${pair[1]}` : pair[0];
}

// 事件本身无 result 字段；按 event_type 语义推导展示（失败/拦截类事件在类型名中自带信号）。
function resultTone(eventType: string): "success" | "warning" | "danger" {
  if (/(failed|fault)/.test(eventType)) return "danger";
  if (/(refused|blocked)/.test(eventType)) return "warning";
  return "success";
}

function resultLabel(eventType: string): string {
  const tone = resultTone(eventType);
  if (tone === "danger") return tr("失败", "Failed");
  if (tone === "warning") return tr("已拦截", "Blocked");
  return tr("成功", "Success");
}

function targetText(event: ControlEvent): string {
  const parts: string[] = [];
  if (event.target_temperature_c !== null && event.target_temperature_c !== undefined) {
    parts.push(`${event.target_temperature_c.toFixed(1)} °C`);
  }
  if (event.target_stirrer_rpm !== null && event.target_stirrer_rpm !== undefined) {
    parts.push(`${event.target_stirrer_rpm} RPM`);
  }
  if (event.target_shake_speed_cpm !== null && event.target_shake_speed_cpm !== undefined) {
    parts.push(`${event.target_shake_speed_cpm} CPM`);
  }
  return parts.length ? parts.join(" · ") : "--";
}

function targetObject(event: ControlEvent): string {
  if (event.batch_id !== null && event.batch_id !== undefined) return `Batch #${event.batch_id}`;
  return tr("反应釜", "Reactor");
}

function hashShort(hash: string | null | undefined): string {
  if (!hash) return "--";
  if (hash.length <= 20) return hash;
  return `${hash.slice(0, 10)}…${hash.slice(-8)}`;
}

function blockId(id: number): string {
  return `#${String(id).padStart(6, "0")}`;
}

// ---------- 加载 ----------
function mergeEventTypes(rows: ControlEvent[]): void {
  const seen = new Set(eventTypeOptions.value);
  for (const row of rows) {
    if (row.event_type && !seen.has(row.event_type)) {
      seen.add(row.event_type);
      eventTypeOptions.value.push(row.event_type);
    }
  }
  eventTypeOptions.value.sort((a, b) => a.localeCompare(b));
}

async function load(): Promise<void> {
  loading.value = true;
  try {
    const payload = await auditApi.logs({
      page: page.value,
      pageSize: pageSize.value,
      eventType: filters.eventType
    });
    events.value = payload?.events ?? [];
    total.value = payload?.total ?? 0;
    chain.value = payload?.chain ?? null;
    mergeEventTypes(events.value);
    selected.value = null;
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    loading.value = false;
  }
}

watch([page, pageSize], () => {
  void load();
});

function applyFilters(): void {
  appliedEventType.value = filters.eventType.trim();
  const decorative =
    filters.batchId.trim() !== "" || (Array.isArray(filters.dateRange) && filters.dateRange.length === 2);
  if (!appliedEventType.value && decorative) {
    ElMessage.info(tr("当前仅事件类型筛选生效", "Only the event type filter is applied"));
  }
  // page 已是 1 时 watch 不会触发，需手动加载；否则由 watch([page,pageSize]) 接管，避免双请求。
  if (page.value === 1) void load();
  else page.value = 1;
}

function resetFilters(): void {
  filters.eventType = "";
  filters.dateRange = null;
  filters.batchId = "";
  appliedEventType.value = "";
  if (page.value === 1) void load();
  else page.value = 1;
}

function selectEvent(row: ControlEvent): void {
  selected.value = row;
}

// ---------- 统计卡 ----------
const sinceText = computed(() => {
  if (!events.value.length || total.value > events.value.length) return "--";
  const stamps = events.value
    .map((event) => event.created_at)
    .filter((value): value is string => Boolean(value))
    .sort();
  return stamps.length ? formatTimestamp(stamps[0]).slice(0, 10) : "--";
});

const chainHealth = computed(() => {
  const hashed = chain.value?.total_hashed_events ?? 0;
  const chained = chain.value?.chained_events ?? 0;
  if (hashed <= 0) return null;
  return {
    pct: Math.round((chained / hashed) * 100),
    hashed,
    chained,
    broken: chain.value?.broken_events ?? 0
  };
});

const verification = computed<{ tone: "good" | "bad" | "unknown"; en: string; zh: string }>(() => {
  if (!chain.value) return { tone: "unknown", en: "--", zh: "" };
  const valid = chain.value.window_valid ?? chain.value.valid;
  if (valid === true) return { tone: "good", en: "Fully Verified", zh: "完全已验证" };
  return { tone: "bad", en: "Broken", zh: "存在断链" };
});

const windowRange = computed(() => {
  const from = chain.value?.checked_from_event_id;
  const to = chain.value?.checked_to_event_id;
  if (from === null || from === undefined || to === null || to === undefined) return "--";
  return `#${from} → #${to}`;
});

const rangeText = computed(() => {
  if (!events.value.length) {
    return language.value === "zh" ? `共 ${total.value} 条` : `0 of ${total.value}`;
  }
  const start = (page.value - 1) * pageSize.value + 1;
  const end = start + events.value.length - 1;
  return language.value === "zh"
    ? `${start}-${end} / 共 ${total.value} 条`
    : `${start}-${end} of ${total.value}`;
});

const chainCubeIds = computed<number[]>(() => {
  const id = selected.value?.id;
  if (!id) return [];
  return [id - 2, id - 1, id, id + 1, id + 2].filter((value) => value > 0);
});

// ---------- 导出 ----------
async function exportCsv(): Promise<void> {
  exporting.value = true;
  try {
    const blob = await auditApi.exportCsv(appliedEventType.value);
    const date = new Date().toISOString().slice(0, 10);
    downloadBlob(blob, `audit-${date}.csv`);
    ElMessage.success(tr("CSV 已导出", "CSV exported"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    exporting.value = false;
  }
}

// 后端无审计报告端点：用当前已加载的真实事件 + 链校验状态生成 Markdown 报告。
function exportReport(): void {
  try {
    const zh = language.value === "zh";
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, "0");
    const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
    const lines: string[] = [];
    lines.push("# Audit Trail Report / 审计追踪报告");
    lines.push("");
    lines.push(`- ${zh ? "生成时间 Generated" : "Generated"}: ${formatTimestamp(now.toISOString())}`);
    if (appliedEventType.value) {
      lines.push(`- ${zh ? "事件类型筛选 Event type" : "Event type filter"}: ${appliedEventType.value}`);
    }
    const status = chain.value;
    if (status) {
      lines.push(
        `- ${zh ? "链上事件 Chained" : "Chained events"}: ${status.chained_events ?? 0} / ${status.total_hashed_events ?? 0}`
      );
      lines.push(
        `- ${zh ? "校验窗口 Window" : "Verification window"}: #${status.checked_from_event_id ?? "--"} → #${status.checked_to_event_id ?? "--"} (${status.window_valid === true ? (zh ? "有效 valid" : "valid") : zh ? "异常 broken" : "broken"})`
      );
    }
    lines.push("");
    lines.push(
      `| # | ${zh ? "时间 Timestamp" : "Timestamp"} | ${zh ? "类型 Type" : "Type"} | ${zh ? "原因 Reason" : "Reason"} | °C | RPM | CPM | ${zh ? "哈希 Hash" : "Hash"} |`
    );
    lines.push("|---|---|---|---|---|---|---|---|");
    for (const event of events.value) {
      const reason = String(event.reason ?? "").replace(/\|/g, "/").replace(/\r?\n/g, " ");
      lines.push(
        `| ${event.id} | ${formatTimestamp(event.created_at)} | ${event.event_type} | ${reason} | ${event.target_temperature_c ?? "-"} | ${event.target_stirrer_rpm ?? "-"} | ${event.target_shake_speed_cpm ?? "-"} | ${event.event_hash ?? "-"} |`
      );
    }
    lines.push("");
    const blob = new Blob([lines.join("\n")], { type: "text/markdown;charset=utf-8" });
    downloadBlob(blob, `audit-report-${stamp}.md`);
    ElMessage.success(tr("报告已导出", "Report exported"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  }
}

// ---------- 复制 ----------
async function copyValue(value: string): Promise<void> {
  if (!value) return;
  try {
    await navigator.clipboard.writeText(value);
    ElMessage.success(tr("已复制", "Copied"));
  } catch {
    try {
      const area = document.createElement("textarea");
      area.value = value;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(area);
      if (ok) ElMessage.success(tr("已复制", "Copied"));
      else ElMessage.error(tr("复制失败", "Copy failed"));
    } catch {
      ElMessage.error(tr("复制失败", "Copy failed"));
    }
  }
}

onMounted(() => {
  void load();
});
</script>

<template>
  <div class="audit-page">
    <!-- 0) 页头 -->
    <header class="page-header">
      <div class="head-left">
        <h1 class="page-title">
          <AppIcon name="audit" :size="20" />
          Audit Trail <span class="zh">审计追踪</span>
        </h1>
        <p class="page-subtitle">
          Immutable audit log for traceability and compliance
          <span class="sub-zh">不可篡改的审计日志，确保可追溯与合规</span>
        </p>
      </div>
      <div class="head-meta">
        <el-tag size="small" :type="verification.tone === 'good' ? 'success' : verification.tone === 'bad' ? 'danger' : 'info'">
          <template v-if="verification.tone === 'good'">Chain Verified <span class="zh">链已验证</span></template>
          <template v-else-if="verification.tone === 'bad'">Chain Broken <span class="zh">链断开</span></template>
          <template v-else>--</template>
        </el-tag>
      </div>
    </header>

    <!-- 1) 统计卡 -->
    <div class="stats-row">
      <div class="stat-card">
        <div class="stat-top">
          <AppIcon name="report" :size="14" />
          <span class="stat-label">Total Events <i class="zh">事件总数</i></span>
        </div>
        <div class="stat-value mono">{{ total }}</div>
        <div class="stat-sub">{{ tr("自", "Since") }} <span class="mono">{{ sinceText }}</span></div>
      </div>

      <div class="stat-card">
        <div class="stat-top">
          <AppIcon name="shield" :size="14" />
          <span class="stat-label">Chain Health <i class="zh">链路健康度</i></span>
        </div>
        <div class="stat-value mono" :class="chainHealth && chainHealth.pct >= 100 ? 'good' : chainHealth ? 'warn' : ''">
          {{ chainHealth ? chainHealth.pct + "%" : "--" }}
        </div>
        <div v-if="chainHealth" class="stat-sub" :class="chainHealth.pct >= 100 ? 'good' : 'bad'">
          <AppIcon v-if="chainHealth.pct >= 100" name="check" :size="12" />
          <template v-if="chainHealth.pct >= 100">All blocks verified <span class="zh">全部区块已验证</span></template>
          <template v-else>{{ chainHealth.broken + " " + tr("个断链块", "broken blocks") }}</template>
        </div>
        <div v-else class="stat-sub">--</div>
      </div>

      <div class="stat-card">
        <div class="stat-top">
          <AppIcon name="check" :size="14" />
          <span class="stat-label">Verification Status <i class="zh">校验状态</i></span>
        </div>
        <div class="stat-value stat-text" :class="verification.tone">
          <span v-if="verification.tone !== 'unknown'" class="status-dot" :class="verification.tone === 'good' ? 'ok' : 'bad'" />
          <template v-if="verification.tone === 'unknown'">--</template>
          <template v-else>{{ verification.en }} <span class="zh">{{ verification.zh }}</span></template>
        </div>
        <div class="stat-sub">Window <span class="mono">{{ windowRange }}</span></div>
      </div>

      <div class="stat-card">
        <div class="stat-top">
          <AppIcon name="export" :size="14" />
          <span class="stat-label">Exports (30 Days) <i class="zh">导出次数</i></span>
        </div>
        <div class="stat-value mono">--</div>
        <div class="stat-sub">CSV / Report</div>
      </div>
    </div>

    <!-- 2) 筛选行 -->
    <div class="filter-bar">
      <div class="f-field">
        <span class="f-label">Event Type <i class="zh">事件类型</i></span>
        <el-select
          v-model="filters.eventType"
          class="f-control"
          filterable
          clearable
          :placeholder="tr('全部类型', 'All types')"
        >
          <el-option v-for="t in eventTypeOptions" :key="t" :value="t" :label="actionDual(t)" />
        </el-select>
      </div>

      <div class="f-field">
        <span class="f-label">User Role <i class="zh">用户角色</i></span>
        <el-select class="f-control" disabled placeholder="--" />
      </div>

      <div class="f-field f-wide">
        <span class="f-label">Date Range <i class="zh">时间范围</i></span>
        <el-date-picker
          v-model="filters.dateRange"
          type="daterange"
          class="f-control"
          range-separator="→"
          :start-placeholder="tr('开始日期', 'Start')"
          :end-placeholder="tr('结束日期', 'End')"
          value-format="YYYY-MM-DD"
        />
      </div>

      <div class="f-field">
        <span class="f-label">Severity <i class="zh">严重程度</i></span>
        <el-select class="f-control" disabled placeholder="--" />
      </div>

      <div class="f-field">
        <span class="f-label">Batch ID <i class="zh">批次号</i></span>
        <el-input
          v-model="filters.batchId"
          class="f-control"
          clearable
          :placeholder="tr('输入批次号', 'Enter batch ID')"
        />
      </div>

      <div class="f-actions">
        <el-button size="small" plain @click="resetFilters">
          <AppIcon name="reset" :size="13" class="btn-icon" />
          {{ tr("重置", "Reset") }}
        </el-button>
        <el-button size="small" type="primary" :loading="loading" @click="applyFilters">
          <AppIcon name="search" :size="13" class="btn-icon" />
          {{ tr("应用", "Apply") }}
        </el-button>
      </div>
    </div>

    <!-- 3) 主区：左日志表 + 右详情 -->
    <div class="main-area">
      <PanelCard en="Audit Log" zh="审计日志" icon="audit" flush class="log-panel">
        <template #actions>
          <div class="panel-actions">
            <el-button size="small" plain :loading="exporting" @click="exportCsv">
              <AppIcon name="export" :size="13" class="btn-icon" />
              {{ tr("导出 CSV", "Export CSV") }}
            </el-button>
            <el-button size="small" plain @click="exportReport">
              <AppIcon name="report" :size="13" class="btn-icon" />
              {{ tr("导出报告", "Export Report") }}
            </el-button>
          </div>
        </template>

        <div v-loading="loading" class="table-wrap overflow-auto">
          <el-table
            :data="events"
            height="100%"
            size="small"
            highlight-current-row
            class="audit-table"
            @row-click="selectEvent"
          >
            <el-table-column width="64">
              <template #header><div class="th"><span>ID</span><span class="th-zh">编号</span></div></template>
              <template #default="{ row }"><span class="mono dim">#{{ row.id }}</span></template>
            </el-table-column>

            <el-table-column width="148">
              <template #header><div class="th"><span>Timestamp</span><span class="th-zh">时间戳</span></div></template>
              <template #default="{ row }"><span class="mono">{{ formatTimestamp(row.created_at) }}</span></template>
            </el-table-column>

            <el-table-column width="86">
              <template #header><div class="th"><span>Actor</span><span class="th-zh">执行者</span></div></template>
              <template #default="{ row }"><span class="mono">{{ row.actor || "--" }}</span></template>
            </el-table-column>

            <el-table-column width="76">
              <template #header><div class="th"><span>Role</span><span class="th-zh">角色</span></div></template>
              <template #default="{ row }"><span class="dim">{{ row.role || "--" }}</span></template>
            </el-table-column>

            <el-table-column min-width="180" show-overflow-tooltip>
              <template #header><div class="th"><span>Action</span><span class="th-zh">操作</span></div></template>
              <template #default="{ row }">{{ actionDual(row.event_type) }}</template>
            </el-table-column>

            <el-table-column width="104">
              <template #header><div class="th"><span>Target</span><span class="th-zh">目标对象</span></div></template>
              <template #default="{ row }"><span class="mono">{{ targetObject(row) }}</span></template>
            </el-table-column>

            <el-table-column width="140">
              <template #header><div class="th"><span>Target Value</span><span class="th-zh">目标值</span></div></template>
              <template #default="{ row }"><span class="mono">{{ targetText(row) }}</span></template>
            </el-table-column>

            <el-table-column min-width="150" show-overflow-tooltip>
              <template #header><div class="th"><span>Reason</span><span class="th-zh">原因</span></div></template>
              <template #default="{ row }"><span class="dim-soft">{{ row.reason || "--" }}</span></template>
            </el-table-column>

            <el-table-column width="100">
              <template #header><div class="th"><span>Result</span><span class="th-zh">结果</span></div></template>
              <template #default="{ row }">
                <el-tag size="small" :type="resultTone(row.event_type)">{{ resultLabel(row.event_type) }}</el-tag>
              </template>
            </el-table-column>

            <el-table-column width="92">
              <template #header><div class="th"><span>Hash</span><span class="th-zh">哈希状态</span></div></template>
              <template #default="{ row }">
                <span v-if="row.event_hash" class="hash-ok"><AppIcon name="check" :size="12" /> Verified</span>
                <span v-else class="dim">--</span>
              </template>
            </el-table-column>

            <template #empty>
              <div class="empty-state">
                <AppIcon name="audit" :size="34" />
                <span>{{ tr("暂无审计事件", "No audit events") }}</span>
              </div>
            </template>
          </el-table>
        </div>

        <div class="table-footer">
          <span class="range mono">{{ rangeText }}</span>
          <el-pagination
            v-model:current-page="page"
            v-model:page-size="pageSize"
            :total="total"
            :page-sizes="[20, 50, 100, 200]"
            :pager-count="5"
            background
            layout="total, sizes, prev, pager, next"
          />
        </div>
      </PanelCard>

      <PanelCard en="Event Details" zh="事件详情" icon="report" scrollable class="detail-panel">
        <div v-if="selected" class="detail-content">
          <div class="d-group">{{ tr("事件信息", "Event") }}</div>
          <div class="d-row">
            <span class="d-k">Event ID <i class="zh">事件ID</i></span>
            <span class="d-v mono">
              {{ blockId(selected.id) }}
              <button class="copy-btn" :title="tr('复制', 'Copy')" @click="copyValue(String(selected.id))">{{ tr("复制", "Copy") }}</button>
            </span>
          </div>
          <div class="d-row">
            <span class="d-k">Timestamp <i class="zh">时间戳</i></span>
            <span class="d-v mono">{{ formatTimestamp(selected.created_at) }}</span>
          </div>
          <div class="d-row">
            <span class="d-k">Actor <i class="zh">执行者</i></span>
            <span class="d-v mono">{{ selected.actor || "--" }}</span>
          </div>
          <div class="d-row">
            <span class="d-k">Role <i class="zh">角色</i></span>
            <span class="d-v">{{ selected.role || "--" }}</span>
          </div>
          <div class="d-row">
            <span class="d-k">Action <i class="zh">操作</i></span>
            <span class="d-v">{{ actionDual(selected.event_type) }}</span>
          </div>
          <div class="d-row">
            <span class="d-k">Target <i class="zh">目标对象</i></span>
            <span class="d-v mono">{{ targetObject(selected) }}</span>
          </div>

          <div class="d-group">{{ tr("变更内容", "Changes") }}</div>
          <div class="d-row">
            <span class="d-k">Old Value <i class="zh">旧值</i></span>
            <span class="d-v mono dim">--</span>
          </div>
          <div class="d-row">
            <span class="d-k">New Value <i class="zh">新值</i></span>
            <span class="d-v mono">{{ targetText(selected) }}</span>
          </div>
          <div class="d-row">
            <span class="d-k">Reason <i class="zh">原因</i></span>
            <span class="d-v reason">{{ selected.reason || "--" }}</span>
          </div>
          <div class="d-row">
            <span class="d-k">Result <i class="zh">结果</i></span>
            <span class="d-v">
              <span class="status-dot" :class="resultTone(selected.event_type) === 'success' ? 'ok' : resultTone(selected.event_type) === 'danger' ? 'bad' : 'warn'" />
              {{ resultLabel(selected.event_type) }}
            </span>
          </div>

          <div class="d-group">{{ tr("来源与链证", "Provenance") }}</div>
          <div class="d-row">
            <span class="d-k">Source IP <i class="zh">来源IP</i></span>
            <span class="d-v mono dim">--</span>
          </div>
          <div class="d-row">
            <span class="d-k">Client <i class="zh">客户端</i></span>
            <span class="d-v mono">Web Console</span>
          </div>
          <div class="d-row">
            <span class="d-k">Notes <i class="zh">备注</i></span>
            <span class="d-v dim">--</span>
          </div>
          <div class="d-row">
            <span class="d-k">Hash <i class="zh">哈希</i></span>
            <span class="d-v mono">
              <span v-if="selected.event_hash" :title="selected.event_hash">{{ hashShort(selected.event_hash) }}</span>
              <span v-else class="dim">--</span>
              <button v-if="selected.event_hash" class="copy-btn" :title="tr('复制', 'Copy')" @click="copyValue(selected.event_hash ?? '')">{{ tr("复制", "Copy") }}</button>
            </span>
          </div>
          <div class="d-row">
            <span class="d-k">Block <i class="zh">区块</i></span>
            <span class="d-v mono">{{ blockId(selected.id) }}</span>
          </div>
          <div class="d-row">
            <span class="d-k">Confirmations <i class="zh">确认数</i></span>
            <span class="d-v mono dim">--</span>
          </div>
          <div class="d-row">
            <span class="d-k">Verified <i class="zh">校验</i></span>
            <span class="d-v">
              <span v-if="selected.event_hash" class="status-dot ok" />
              <span v-else class="status-dot" />
              {{ selected.event_hash ? tr("已验证", "Verified") : "--" }}
            </span>
          </div>

          <!-- 链路校验可视化 -->
          <div class="chain-block">
            <div class="chain-head">
              <span class="chain-title">Chain Verification <i class="zh">链路校验</i></span>
              <a class="chain-link" @click.prevent="chainDialog = true">View Full Chain {{ tr("完整链路", "") }} →</a>
            </div>
            <div class="chain-cubes">
              <template v-for="(cid, idx) in chainCubeIds" :key="cid">
                <span v-if="idx > 0" class="chain-arrow">→</span>
                <div class="cube" :class="{ current: selected && cid === selected.id }">
                  <span class="cube-num">#{{ cid }}</span>
                  <span class="cube-tag">{{ cid === selected?.id ? tr("当前", "CURRENT") : tr("区块", "BLOCK") }}</span>
                </div>
              </template>
            </div>
          </div>

          <div class="detail-actions">
            <el-button size="small" plain :loading="exporting" @click="exportCsv">
              <AppIcon name="export" :size="13" class="btn-icon" />
              {{ tr("导出 CSV", "Export CSV") }}
            </el-button>
            <el-button size="small" plain @click="exportReport">
              <AppIcon name="report" :size="13" class="btn-icon" />
              {{ tr("导出报告", "Export Report") }}
            </el-button>
          </div>
        </div>

        <div v-else class="empty-state">
          <AppIcon name="audit" :size="34" />
          <span class="empty-title">{{ tr("未选择事件", "No Event Selected") }}</span>
          <span class="empty-hint">{{ tr("点击左侧日志行查看事件详情", "Click a log row to view event details") }}</span>
        </div>
      </PanelCard>
    </div>

    <!-- 完整链路对话框（真实 chain 状态字段） -->
    <el-dialog v-model="chainDialog" width="480px" :title="'Hash Chain ' + tr('哈希链状态', '')">
      <dl v-if="chain" class="kv-list">
        <dt>{{ tr("哈希事件总数", "Total hashed") }}</dt>
        <dd class="mono">{{ chain.total_hashed_events ?? "--" }}</dd>
        <dt>{{ tr("已校验事件", "Checked") }}</dt>
        <dd class="mono">{{ chain.checked_events ?? "--" }}</dd>
        <dt>{{ tr("链上事件", "Chained") }}</dt>
        <dd class="mono">{{ chain.chained_events ?? "--" }}</dd>
        <dt>{{ tr("断链事件", "Broken") }}</dt>
        <dd class="mono">{{ chain.broken_events ?? "--" }}</dd>
        <dt>{{ tr("窗口校验", "Window valid") }}</dt>
        <dd class="mono" :class="(chain.window_valid ?? chain.valid) === true ? 'good' : 'bad'">
          {{ (chain.window_valid ?? chain.valid) === true ? tr("有效", "valid") : tr("异常", "broken") }}
        </dd>
        <dt>{{ tr("校验范围", "Checked range") }}</dt>
        <dd class="mono">{{ windowRange }}</dd>
        <dt>{{ tr("最新事件哈希", "Last hash") }}</dt>
        <dd class="mono" :title="chain.last_event_hash ?? ''">{{ hashShort(chain.last_event_hash) }}</dd>
        <dt>{{ tr("校验上限", "Verification limit") }}</dt>
        <dd class="mono">{{ chain.verification_limit ?? "--" }}</dd>
      </dl>
      <div v-else class="empty-state">{{ tr("链状态未加载", "Chain status not loaded") }}</div>
    </el-dialog>
  </div>
</template>

<style scoped>
/* ===== 页面骨架：禁止整页滚动 ===== */
.audit-page {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

.head-left {
  min-width: 0;
}

.head-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}

.sub-zh {
  display: block;
  margin-top: 2px;
  color: var(--text-tertiary);
}

/* ===== 统计卡 ===== */
.stats-row {
  flex: none;
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--spacing);
}

.stat-card {
  background: var(--bg-panel);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-lg);
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
}

.stat-top {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--accent);
}

.stat-label {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  font-weight: 600;
  /* V30：窄屏允许换行 */
  white-space: normal;
  overflow-wrap: anywhere;
}

.stat-label i {
  font-style: normal;
  font-weight: 400;
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
  margin-left: 3px;
}

.stat-value {
  font-size: var(--fs-xl);
  font-weight: 700;
  line-height: 1.1;
}

.stat-value.stat-text {
  font-family: var(--font-ui);
  font-size: var(--fs-md);
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
}

.stat-sub {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  display: flex;
  align-items: center;
  gap: 4px;
  /* V30：长窗口范围允许换行 */
  white-space: normal;
  flex-wrap: wrap;
  overflow-wrap: anywhere;
}

.good { color: var(--ind-green); }
.bad { color: var(--ind-red); }
.warn { color: var(--ind-amber); }

/* ===== 筛选行 ===== */
.filter-bar {
  flex: none;
  display: flex;
  align-items: flex-end;
  gap: 12px;
  flex-wrap: wrap;
  min-width: 0;
  background: var(--bg-panel);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-lg);
  padding: 10px 14px;
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
}

.f-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.f-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  font-weight: 600;
  white-space: nowrap;
}

.f-label i {
  font-style: normal;
  font-weight: 400;
  margin-left: 3px;
}

.f-control {
  width: 158px;
  max-width: 100%;
}

.f-wide { max-width: 100%; min-width: 0; }
.f-wide .f-control {
  width: 232px;
  max-width: 100%;
}
/* V32：日期范围选择器内部最小宽度在 393px 溢出——深度收敛 */
.f-wide :deep(.el-date-editor) { max-width: 100%; min-width: 0; }

/* V32：移动端分页瘦身（total/sizes 挤爆 393px 页脚） */
@media (max-width: 900px) {
  .table-footer { justify-content: center; }
  .table-footer :deep(.el-pagination__total),
  .table-footer :deep(.el-pagination__sizes),
  .table-footer .range { display: none; }
}

.f-actions {
  display: flex;
  gap: 8px;
  margin-left: auto;
  align-self: flex-end;
}

/* ===== 主区左右布局 ===== */
.main-area {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 380px;
  gap: var(--spacing);
}

.log-panel {
  min-width: 0;
}

.panel-actions {
  display: flex;
  gap: 8px;
}

.btn-icon {
  margin-right: 5px;
  vertical-align: -2px;
}

.table-wrap {
  flex: 1;
  min-height: 0;
  position: relative;
  /* V30/V32：移动端表格横向滚动（多列审计表在 393px 必然超宽） */
  overflow-x: auto;
}

.audit-table :deep(.el-table__row) {
  cursor: pointer;
}

.th {
  display: flex;
  flex-direction: column;
  gap: 1px;
  line-height: 1.25;
}

.th span:first-child {
  color: var(--text-secondary);
  font-size: var(--fs-sm);
  font-weight: 600;
}

.th .th-zh {
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
  font-weight: 400;
}

.dim {
  color: var(--text-tertiary);
}

.dim-soft {
  color: var(--text-secondary);
}

.hash-ok {
  color: var(--ind-green);
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-weight: 600;
  white-space: nowrap;
}

.table-footer {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 8px 14px;
  border-top: 1px solid var(--border-glass);
  flex-wrap: wrap;
}

.range {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

/* ===== 右侧详情 ===== */
.detail-panel {
  min-width: 0;
}

.detail-content {
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.d-group {
  font-size: var(--fs-xs);
  letter-spacing: 0.8px;
  text-transform: uppercase;
  color: var(--accent);
  font-weight: 700;
  margin: 6px 0 2px;
  padding-bottom: 4px;
  border-bottom: 1px solid var(--border-glass);
}

.d-group:first-child {
  margin-top: 0;
}

.d-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}

.d-k {
  color: var(--text-tertiary);
  font-size: var(--fs-sm);
  white-space: nowrap;
  flex: none;
}

.d-k i {
  font-style: normal;
  font-size: var(--fs-xs);
  opacity: 0.75;
  margin-left: 2px;
}

.d-v {
  font-size: var(--fs-sm);
  color: var(--text-primary);
  text-align: right;
  overflow-wrap: anywhere;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  justify-content: flex-end;
  min-width: 0;
}

.d-v.reason {
  font-family: var(--font-ui);
  display: block;
  text-align: right;
  white-space: normal;
}

.copy-btn {
  flex: none;
  border: 1px solid var(--border-glass);
  background: var(--bg-inset);
  border-radius: var(--radius-sm);
  color: var(--text-tertiary);
  font-size: 10px;
  padding: 1px 6px;
  cursor: pointer;
  line-height: 1.5;
}

.copy-btn:hover {
  border-color: var(--accent);
  color: var(--accent);
}

/* ===== 链路校验立方体 ===== */
.chain-block {
  margin-top: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--border-glass);
}

.chain-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 4px;
}

.chain-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
}

.chain-title i {
  font-style: normal;
  font-weight: 400;
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
  margin-left: 3px;
}

.chain-link {
  font-size: var(--fs-xs);
  color: var(--accent);
  cursor: pointer;
  text-decoration: none;
  white-space: nowrap;
}

.chain-link:hover {
  color: var(--accent-strong);
  text-decoration: underline;
}

.chain-cubes {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 10px 0 4px;
  flex-wrap: nowrap;
}

.cube {
  width: 52px;
  height: 46px;
  flex: none;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-inset);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
}

.cube-num {
  font-family: var(--font-data);
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

.cube-tag {
  font-size: 9px;
  letter-spacing: 0.4px;
  color: var(--text-tertiary);
}

.cube.current {
  border-color: var(--accent);
  background: var(--accent-dim);
  box-shadow: 0 0 10px rgba(47, 155, 255, 0.35);
}

.cube.current .cube-num {
  color: var(--accent-strong);
  font-weight: 700;
}

.cube.current .cube-tag {
  color: var(--accent);
}

.chain-arrow {
  color: var(--text-tertiary);
  font-family: var(--font-data);
  flex: none;
}

.detail-actions {
  display: flex;
  gap: 10px;
  margin-top: 12px;
}

.detail-actions .el-button {
  flex: 1;
}

/* ===== 空态 ===== */
.empty-state {
  flex: 1;
}

.empty-state svg {
  color: var(--text-tertiary);
  opacity: 0.55;
}

.empty-title {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-secondary);
}

.empty-hint {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

/* ===== V32：移动端单列堆叠、整页可滚动 ===== */
@media (max-width: 900px) {
  .audit-page { height: auto; overflow: visible; }
  .main-area { display: flex; flex-direction: column; }
  .main-area > * { flex: none; }
  .stats-row { grid-template-columns: 1fr 1fr; }
}

/* ===== 响应式 ===== */
@media (max-width: 1500px) {
  .main-area {
    grid-template-columns: minmax(0, 1fr) 340px;
  }
}

@media (max-width: 1150px) {
  .main-area {
    grid-template-columns: minmax(0, 1fr) 300px;
  }

  .stats-row {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
