<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, formatTimestamp, numberAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const batches = computed(() => arrayAt(store.batches, "batches"));
const outcomes = computed(() => arrayAt(store.batches, "outcomes"));
const runtime = computed(() => objectAt(store.live, "runtime") ?? store.runtimeFallback);
const liveAlarms = computed(() => arrayAt(store.live, "alarms"));
const liveUnavailable = computed(() => store.liveStatus !== "fresh");
const activeBatchId = computed(() => numberAt(runtime.value, "active_batch_id"));
const unfinishedBatchRecoveryAlarm = computed(
  () => liveAlarms.value.find((alarm) => textAt(alarm, "type", "") === "unfinished_batch_recovery") ?? null
);
const batchRecoveryBlocked = computed(() => Boolean(unfinishedBatchRecoveryAlarm.value));
const historyFilters = reactive({
  search: "",
  status: "all",
  ratioBand: "all"
});
const productResultForm = reactive({
  batch_id: "",
  yield_percent: 80,
  product_ratio: 0.8,
  notes: ""
});
type OptionalBatchNumber = number | null;
// Standalone batch start form (POST /api/batches/start) — requires at least one
// explicit target/duration field (backend rejects all-absent). process_id optional.
const startBatchForm = reactive({
  name: "",
  process_id: "",
  target_temperature_c: null as OptionalBatchNumber,
  target_stirrer_rpm: null as OptionalBatchNumber,
  heating_minutes: null as OptionalBatchNumber,
  stirring_minutes: null as OptionalBatchNumber
});
const startBatchDisabled = computed(() => {
  const has = (v: OptionalBatchNumber) => v !== null;
  return !(
    has(startBatchForm.target_temperature_c) ||
    has(startBatchForm.target_stirrer_rpm) ||
    has(startBatchForm.heating_minutes) ||
    has(startBatchForm.stirring_minutes)
  );
});
// Paged sample time-series history (GET /api/v1/reactor/:id/history)
const DEVICE_ID = "reactor_001";
const historySamples = ref<unknown[]>([]);
const historyLoading = ref(false);
const historyPage = ref(1);
const historyPageSize = ref(50);
// v1_history item shape: { timestamp, data: {current_temp, current_pressure,
// stir_speed, flow_rate, ph, product_concentration, ...}, batch_id }.
// Helper to pull a nested field out of item.data for the table cells.
function histField(row: unknown, field: string): string {
  const data = objectAt(row, "data");
  return textAt(data, field, "--");
}
// `loadBatchDetail` returns a wrapper object { batch, outcome, samples, events }
// (see src/api.rs `get_batch_detail`). The previous code stored the wrapper
// directly into `selectedBatch`, so the detail panel read fields like
// `id`/`name` off the wrapper and rendered blank. The split below keeps the
// raw response as `selectedBatchDetail` and exposes the inner `batch`,
// `outcome`, `samples`, and `events` as separate computeds.
const selectedBatchDetail = ref<Record<string, unknown> | null>(null);
const selectedBatch = computed(() => objectAt(selectedBatchDetail.value, "batch"));
const selectedOutcome = computed(() => objectAt(selectedBatchDetail.value, "outcome"));
const selectedSamples = computed(() => arrayAt(selectedBatchDetail.value, "samples"));
const selectedEvents = computed(() => arrayAt(selectedBatchDetail.value, "events"));
const reportBytes = ref<number | null>(null);
const actionMessage = ref("");

function triggerBlobDownload(blob: Blob, filename: string): string {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.style.display = "none";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  return url;
}

function uniqueSorted(values: string[]): string[] {
  return Array.from(new Set(values.filter(Boolean))).sort((a, b) => a.localeCompare(b));
}

function lower(value: string): string {
  return value.trim().toLocaleLowerCase();
}

function hasFinishedAt(row: unknown): boolean {
  if (!row || typeof row !== "object") return false;
  const value = (row as Record<string, unknown>).finished_at;
  return value !== null && value !== undefined && value !== "";
}

function outcomeForBatchId(batchId: number | null): Record<string, unknown> | null {
  if (batchId === null) return null;
  return outcomes.value.find((row) => numberAt(row, "batch_id") === batchId) ?? null;
}

const statusOptions = computed(() => uniqueSorted(batches.value.map((row) => textAt(row, "status", ""))));
const ratioBandOptions = [
  { value: "gte_090", zh: ">= 0.90", en: ">= 0.90" },
  { value: "075_090", zh: "0.75 - 0.90", en: "0.75 - 0.90" },
  { value: "lt_075", zh: "< 0.75", en: "< 0.75" }
];

const batchOptions = computed(() =>
  batches.value.map((row) => ({
    id: numberAt(row, "id"),
    label: `#${textAt(row, "id")} - ${textAt(row, "name")}`,
    finished: hasFinishedAt(row),
    active: activeBatchId.value !== null && numberAt(row, "id") === activeBatchId.value
  }))
);

const selectedProductBatch = computed(
  () => batches.value.find((row) => String(numberAt(row, "id")) === productResultForm.batch_id) ?? null
);
const selectedProductBatchFinished = computed(() => hasFinishedAt(selectedProductBatch.value));
const selectedProductBatchActive = computed(() => {
  const selectedId = numberAt(selectedProductBatch.value, "id");
  return selectedId !== null && activeBatchId.value === selectedId;
});
const productResultBlocked = computed(
  () =>
    !store.isAuthenticated ||
    !productResultForm.batch_id ||
    liveUnavailable.value ||
    batchRecoveryBlocked.value ||
    activeBatchId.value !== null ||
    !selectedProductBatchFinished.value ||
    selectedProductBatchActive.value
);
const productResultBlockReason = computed(() => {
  if (!store.isAuthenticated) return store.tr("需要登录后录入产物结果。", "Sign in before saving product results.");
  if (!productResultForm.batch_id) return store.tr("请选择一个已完成批次。", "Select a finished batch.");
  if (liveUnavailable.value) {
    return store.tr(
      "实时现场状态不可用，产物结果录入已锁定，避免把未知状态写入 AI 依据。",
      "Live field state is unavailable; product result entry is locked to avoid contaminating AI evidence."
    );
  }
  if (batchRecoveryBlocked.value) {
    const ids = textAt(unfinishedBatchRecoveryAlarm.value, "unfinished_batch_ids", "");
    return store.tr(
      `未完成批次恢复未决，先核对现场并修复批次记录。${ids}`,
      `Unfinished batch recovery is unresolved; verify the field and repair batch records first. ${ids}`
    );
  }
  if (activeBatchId.value !== null) {
    return store.tr(
      `当前仍有活动批次 #${activeBatchId.value}，先结束并确认生产状态。`,
      `Batch #${activeBatchId.value} is still active; finish and verify production first.`
    );
  }
  if (!selectedProductBatchFinished.value || selectedProductBatchActive.value) {
    return store.tr("只能给已完成且非活动的批次录入结果。", "Only finished, inactive batches can receive product results.");
  }
  return "";
});

function outcomeText(outcome: Record<string, unknown> | null, key: string, fallback = "--"): string {
  return outcome ? textAt(outcome, key, fallback) : fallback;
}

function ratioBandForOutcome(outcome: Record<string, unknown> | null): string {
  const ratio = numberAt(outcome, "product_ratio");
  if (ratio === null) return "none";
  if (ratio >= 0.9) return "gte_090";
  if (ratio >= 0.75) return "075_090";
  return "lt_075";
}

const filteredBatches = computed(() => {
  const query = lower(historyFilters.search);
  const status = historyFilters.status;
  const ratioBand = historyFilters.ratioBand;
  return batches.value.filter((row) => {
    const id = numberAt(row, "id");
    const outcome = outcomeForBatchId(id);
    const rowStatus = textAt(row, "status", "");
    const rowRatio = textAt(outcome, "product_ratio", "");
    const rowYield = textAt(outcome, "yield_percent", "");
    const searchable = [
      textAt(row, "id", ""),
      textAt(row, "name", ""),
      textAt(row, "process_id", ""),
      rowStatus,
      rowRatio,
      rowYield
    ].join(" ").toLocaleLowerCase();
    return (
      (!query || searchable.includes(query)) &&
      (status === "all" || rowStatus === status) &&
      (ratioBand === "all" || ratioBandForOutcome(outcome) === ratioBand)
    );
  });
});

const filteredBatchIds = computed(() => new Set(filteredBatches.value.map((row) => numberAt(row, "id")).filter((id): id is number => id !== null)));
const filteredOutcomes = computed(() => outcomes.value.filter((row) => filteredBatchIds.value.has(numberAt(row, "batch_id") ?? -1)));

function clearHistoryFilters(): void {
  historyFilters.search = "";
  historyFilters.status = "all";
  historyFilters.ratioBand = "all";
}

function statusTagType(status: string): "success" | "warning" | "info" | "danger" {
  if (status === "completed") return "success";
  if (status === "running") return "success";
  if (status === "failed" || status === "rejected") return "danger";
  if (status === "draft" || status === "pending") return "warning";
  return "info";
}

function processStatusText(status: string): string {
  if (store.isChinese) {
    if (status === "draft") return "草稿";
    if (status === "applied") return "已应用";
    if (status === "archived") return "已归档";
    if (status === "running") return "运行中";
    if (status === "completed") return "已完成";
    if (status === "failed") return "失败";
    return status || "未知";
  }
  if (status === "draft") return "Draft";
  if (status === "applied") return "Applied";
  if (status === "archived") return "Archived";
  if (status === "running") return "Running";
  if (status === "completed") return "Completed";
  if (status === "failed") return "Failed";
  return status || "Unknown";
}

async function refresh(): Promise<void> {
  try {
    await store.loadBatches();
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function selectBatch(id: number | null): Promise<void> {
  if (id === null) {
    selectedBatchDetail.value = null;
    reportBytes.value = null;
    return;
  }
  try {
    const detail = await store.loadBatchDetail(id);
    selectedBatchDetail.value = detail as Record<string, unknown>;
    productResultForm.batch_id = String(id);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function saveProductResult(): Promise<void> {
  const batchId = Number(productResultForm.batch_id);
  if (!Number.isFinite(batchId)) return;
  try {
    store.error = null;
    actionMessage.value = "";
    await store.saveProductResult({
      batch_id: batchId,
      yield_percent: productResultForm.yield_percent,
      product_ratio: productResultForm.product_ratio,
      notes: productResultForm.notes.trim() || undefined
    });
    actionMessage.value = store.tr("产物结果已保存，AI 推荐已重新生成。", "Product result saved and AI recommendation regenerated.");
    await selectBatch(batchId);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function startStandaloneBatch(): Promise<void> {
  try {
    store.error = null;
    actionMessage.value = "";
    const num = (v: OptionalBatchNumber): number | null => (v === null ? null : Number(v));
    const pid = startBatchForm.process_id.trim();
    await store.startBatch({
      name: startBatchForm.name.trim() || undefined,
      process_id: pid === "" ? null : Number(pid),
      target_temperature_c: num(startBatchForm.target_temperature_c),
      target_stirrer_rpm: num(startBatchForm.target_stirrer_rpm),
      heating_minutes: num(startBatchForm.heating_minutes),
      stirring_minutes: num(startBatchForm.stirring_minutes)
    });
    actionMessage.value = store.tr("批次已启动。", "Batch started.");
    startBatchForm.name = "";
    startBatchForm.target_temperature_c = null;
    startBatchForm.target_stirrer_rpm = null;
    startBatchForm.heating_minutes = null;
    startBatchForm.stirring_minutes = null;
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function finishActiveBatch(): Promise<void> {
  if (activeBatchId.value === null) return;
  try {
    store.error = null;
    actionMessage.value = "";
    await store.finishBatch(activeBatchId.value);
    actionMessage.value = store.tr(`批次 #${activeBatchId.value} 已结束。`, `Batch #${activeBatchId.value} finished.`);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function loadHistorySamples(): Promise<void> {
  historyLoading.value = true;
  try {
    const resp = await store.loadHistory(DEVICE_ID, {
      page: historyPage.value,
      pageSize: historyPageSize.value
    });
    // v1_history response shape: { records, items: [{timestamp, data:{current_temp,...}, batch_id}] }
    historySamples.value = arrayAt<object>(resp, "items");
    if (historySamples.value.length === 0) {
      actionMessage.value = store.tr("无样本历史数据（当前无管线样本入库）。", "No sample history (no pipeline samples persisted yet).");
    }
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
    historySamples.value = [];
  } finally {
    historyLoading.value = false;
  }
}

async function downloadBatchesCsv(): Promise<void> {
  try {
    store.error = null;
    actionMessage.value = "";
    const blob = await store.exportBatchesCsv();
    triggerBlobDownload(blob, `batches-${Date.now()}.csv`);
    actionMessage.value = store.tr("批次 CSV 已下载", "Batch CSV downloaded");
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function downloadBatchesXlsx(): Promise<void> {
  try {
    store.error = null;
    actionMessage.value = "";
    const blob = await store.exportBatchesXlsx();
    triggerBlobDownload(blob, `batches-${Date.now()}.xlsx`);
    actionMessage.value = store.tr("批次 XLSX 已下载", "Batch XLSX downloaded");
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function downloadReport(): Promise<void> {
  if (!selectedBatch.value) return;
  const id = numberAt(selectedBatch.value, "id");
  if (id === null) return;
  try {
    const blob = await store.exportBatchReport(id);
    triggerBlobDownload(blob, `batch-${id}-report.md`);
    reportBytes.value = blob.size;
    actionMessage.value = store.tr("报告已下载", "Report downloaded");
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

watch(
  batches,
  (rows) => {
    if (!productResultForm.batch_id && rows.length > 0) {
      const id = numberAt(rows[0], "id");
      if (id !== null) productResultForm.batch_id = String(id);
    }
  },
  { immediate: true }
);
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("批次与导出", "Batches & export") }}</p>
        <h1>{{ store.tr("历史数据", "History Data") }}</h1>
        <span>{{ store.tr("批次、产物结果、报告与 CSV/XLSX 导出", "Batches, outcomes, reports, and CSV/XLSX export") }}</span>
      </div>
      <div class="heading-actions">
        <el-tag>{{ filteredBatches.length }} / {{ batches.length }} {{ store.tr("批次", "batches") }}</el-tag>
        <el-button :disabled="!store.isAuthenticated" @click="downloadBatchesCsv">
          {{ store.tr("导出 CSV", "Export CSV") }}
        </el-button>
        <el-button :disabled="!store.isAuthenticated" @click="downloadBatchesXlsx">
          {{ store.tr("导出 XLSX", "Export XLSX") }}
        </el-button>
        <el-button :disabled="!store.isAuthenticated" @click="refresh">
          {{ store.tr("刷新", "Refresh") }}
        </el-button>
      </div>
    </div>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("批次生命周期", "Batch Lifecycle") }}</h2>
        <el-tag type="info">{{ store.tr("独立启动 / 结束活动批次", "Standalone start / finish active") }}</el-tag>
      </div>
      <p>{{ store.tr("启动独立批次至少需要一项目标或时长（后端拒绝全空）。结束将停机活动批次并关闭记录。", "Starting a standalone batch requires at least one target or duration field (backend rejects all-absent). Finishing stops the active batch and closes its record.") }}</p>
      <el-form label-position="top" class="product-result-form">
        <el-form-item :label="store.tr('批次名称', 'Batch name')">
          <el-input v-model="startBatchForm.name" :placeholder="store.tr('可选', 'optional')" />
        </el-form-item>
        <el-form-item :label="store.tr('目标温度 C', 'Target temp C')">
          <el-input-number v-model="startBatchForm.target_temperature_c" :min="0" :max="220" controls-position="right" :placeholder="store.tr('可选', 'optional')" />
        </el-form-item>
        <el-form-item :label="store.tr('搅拌 rpm', 'Stirrer rpm')">
          <el-input-number v-model="startBatchForm.target_stirrer_rpm" :min="0" :max="1800" controls-position="right" />
        </el-form-item>
        <el-form-item :label="store.tr('加热 min', 'Heating min')">
          <el-input-number v-model="startBatchForm.heating_minutes" :min="0" :max="600" controls-position="right" />
        </el-form-item>
        <el-form-item :label="store.tr('搅拌 min', 'Stirring min')">
          <el-input-number v-model="startBatchForm.stirring_minutes" :min="0" :max="600" controls-position="right" />
        </el-form-item>
        <el-form-item :label="store.tr('工艺 ID', 'Process ID')">
          <el-input v-model="startBatchForm.process_id" :placeholder="store.tr('可选', 'optional')" />
        </el-form-item>
        <div class="control-actions">
          <el-button
            type="primary"
            :disabled="!store.isAuthenticated || startBatchDisabled || batchRecoveryBlocked"
            @click="startStandaloneBatch"
          >
            {{ store.tr("启动批次", "Start Batch") }}
          </el-button>
          <el-button
            type="danger"
            plain
            :disabled="!store.isAuthenticated || activeBatchId === null || batchRecoveryBlocked"
            @click="finishActiveBatch"
          >
            {{ store.tr("结束活动批次", "Finish Active Batch") }}{{ activeBatchId !== null ? ` #${activeBatchId}` : "" }}
          </el-button>
        </div>
      </el-form>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("样本时序历史", "Sample Time-Series History") }}</h2>
        <el-tag type="info">GET /api/v1/reactor/:id/history</el-tag>
      </div>
      <p>{{ store.tr("从后端分页端点拉取已持久化的传感器样本（区别于本地批次过滤）。", "Fetch persisted sensor samples from the backend paged endpoint (distinct from local batch filtering).") }}</p>
      <div class="control-actions">
        <el-input-number v-model="historyPage" :min="1" :max="9999" controls-position="right" />
        <span class="muted">{{ store.tr("页", "page") }}</span>
        <el-input-number v-model="historyPageSize" :min="10" :max="500" controls-position="right" />
        <span class="muted">{{ store.tr("条/页", "per page") }}</span>
        <el-button :loading="historyLoading" @click="loadHistorySamples">
          {{ store.tr("拉取样本", "Fetch Samples") }}
        </el-button>
      </div>
      <el-table v-if="historySamples.length > 0" :data="historySamples" class="data-table" size="small" max-height="360">
        <el-table-column :label="store.tr('时间', 'Time')" min-width="170">
          <template #default="{ row }">{{ formatTimestamp(textAt(row, "timestamp")) }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('温度 C', 'Temp')" width="90">
          <template #default="{ row }">{{ histField(row, "current_temp") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('压力 MPa', 'Press')" width="110">
          <template #default="{ row }">{{ histField(row, "current_pressure") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('rpm', 'rpm')" width="80">
          <template #default="{ row }">{{ histField(row, "stir_speed") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('流量', 'Flow')" width="80">
          <template #default="{ row }">{{ histField(row, "flow_rate") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('浓度 %', 'Conc.')" width="90">
          <template #default="{ row }">{{ histField(row, "product_concentration") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('pH', 'pH')" width="70">
          <template #default="{ row }">{{ histField(row, "ph") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('批次', 'Batch')" width="80">
          <template #default="{ row }">{{ textAt(row, "batch_id") ? "#" + textAt(row, "batch_id") : "--" }}</template>
        </el-table-column>
      </el-table>
      <div v-else class="process-empty">
        {{ store.tr("尚未拉取样本历史。", "No sample history fetched yet.") }}
      </div>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("历史筛选", "History Filters") }}</h2>
        <el-tag>{{ store.tr("本地联动", "Local linked view") }}</el-tag>
      </div>
      <el-form label-position="top" class="history-filter-form">
        <el-form-item :label="store.tr('搜索', 'Search')">
          <el-input
            v-model="historyFilters.search"
            clearable
            :placeholder="store.tr('批次、工艺、状态、产率或产物比例', 'Batch, process, status, yield, or product ratio')"
          />
        </el-form-item>
        <el-form-item :label="store.tr('状态', 'Status')">
          <el-select v-model="historyFilters.status">
            <el-option :label="store.tr('全部状态', 'All statuses')" value="all" />
            <el-option v-for="status in statusOptions" :key="status" :label="processStatusText(status)" :value="status" />
          </el-select>
        </el-form-item>
        <el-form-item :label="store.tr('产物比例', 'Product ratio')">
          <el-select v-model="historyFilters.ratioBand">
            <el-option :label="store.tr('全部比例', 'All ratios')" value="all" />
            <el-option v-for="band in ratioBandOptions" :key="band.value" :label="store.tr(band.zh, band.en)" :value="band.value" />
          </el-select>
        </el-form-item>
        <div class="control-actions">
          <el-button @click="clearHistoryFilters">{{ store.tr("清空筛选", "Clear Filters") }}</el-button>
        </div>
      </el-form>
    </section>

    <section class="panel two-col">
      <div class="data-table-wrap">
        <el-table :data="filteredBatches" class="data-table" size="small" @row-click="(row) => selectBatch(numberAt(row, 'id'))">
          <el-table-column :label="store.tr('ID', 'ID')" width="64">
            <template #default="{ row }">{{ textAt(row, "id") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('批次', 'Batch')" min-width="160">
            <template #default="{ row }">{{ textAt(row, "name") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('工艺', 'Process')" width="80">
            <template #default="{ row }">{{ textAt(row, "process_id") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('状态', 'Status')" width="120">
            <template #default="{ row }">
              <el-tag :type="statusTagType(textAt(row, 'status'))" size="small">
                {{ processStatusText(textAt(row, 'status')) }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="store.tr('产物比例', 'Product ratio')" width="120">
            <template #default="{ row }">{{ outcomeText(outcomeForBatchId(numberAt(row, "id")), "product_ratio") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('产率 %', 'Yield %')" width="100">
            <template #default="{ row }">{{ outcomeText(outcomeForBatchId(numberAt(row, "id")), "yield_percent") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('开始', 'Started')" min-width="160">
            <template #default="{ row }">{{ formatTimestamp(textAt(row, "started_at", "")) }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('结束', 'Finished')" min-width="160">
            <template #default="{ row }">{{ formatTimestamp(textAt(row, "finished_at", "")) }}</template>
          </el-table-column>
        </el-table>
      </div>
      <div class="analysis-block">
        <h2>{{ store.tr("批次详情", "Batch Detail") }}</h2>
        <p v-if="!selectedBatch" class="muted">{{ store.tr("选择左侧批次查看详情和报告。", "Select a batch on the left to view its detail and report.") }}</p>
        <template v-else>
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item :label="store.tr('批次 ID', 'Batch ID')">{{ textAt(selectedBatch, "id") }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('名称', 'Name')">{{ textAt(selectedBatch, "name") }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('工艺', 'Process')">{{ textAt(selectedBatch, "process_id") }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('状态', 'Status')">
              <el-tag :type="statusTagType(textAt(selectedBatch, 'status'))" size="small">
                {{ processStatusText(textAt(selectedBatch, 'status')) }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="store.tr('开始', 'Started')">{{ formatTimestamp(textAt(selectedBatch, "started_at", "")) }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('结束', 'Finished')">{{ formatTimestamp(textAt(selectedBatch, "finished_at", "")) }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('产物比例', 'Product ratio')">{{ outcomeText(selectedOutcome, "product_ratio", store.tr("未填写", "n/a")) }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('产率', 'Yield')">{{ selectedOutcome ? textAt(selectedOutcome, "yield_percent", "--") : "--" }} %</el-descriptions-item>
            <el-descriptions-item :label="store.tr('目标温度', 'Target temperature')">{{ outcomeText(selectedOutcome, "target_temperature_c") }} C</el-descriptions-item>
            <el-descriptions-item :label="store.tr('目标搅拌', 'Target stirrer')">{{ outcomeText(selectedOutcome, "target_stirrer_rpm") }} rpm</el-descriptions-item>
            <el-descriptions-item :label="store.tr('加热时长', 'Heating minutes')">{{ outcomeText(selectedOutcome, "heating_minutes") }} min</el-descriptions-item>
            <el-descriptions-item :label="store.tr('搅拌时长', 'Stirring minutes')">{{ outcomeText(selectedOutcome, "stirring_minutes") }} min</el-descriptions-item>
            <el-descriptions-item :label="store.tr('样本数', 'Sample count')">{{ selectedSamples.length }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('事件数', 'Event count')">{{ selectedEvents.length }}</el-descriptions-item>
          </el-descriptions>
          <div class="control-actions">
            <el-button type="primary" :disabled="!store.isAuthenticated" @click="downloadReport">
              {{ store.tr("下载报告 (Markdown)", "Download Report (Markdown)") }}
            </el-button>
            <span v-if="reportBytes !== null" class="muted">{{ reportBytes }} B</span>
          </div>
          <p v-if="actionMessage" class="muted">{{ actionMessage }}</p>
        </template>
      </div>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("产物结果录入", "Product Result Entry") }}</h2>
        <el-tag>{{ store.tr("驱动 AI 推荐更新", "Updates AI recommendation") }}</el-tag>
      </div>
      <el-form label-position="top" class="product-result-form">
        <el-form-item :label="store.tr('批次', 'Batch')">
          <el-select v-model="productResultForm.batch_id" :placeholder="store.tr('选择批次', 'Select batch')">
            <el-option
              v-for="batch in batchOptions"
              :key="batch.id ?? batch.label"
              :label="batch.label"
              :value="String(batch.id)"
              :disabled="batch.id === null || !batch.finished || batch.active"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="store.tr('产率 %', 'Yield %')">
          <el-input-number v-model="productResultForm.yield_percent" :min="0" :max="100" :step="0.1" />
        </el-form-item>
        <el-form-item :label="store.tr('产物比例', 'Product ratio')">
          <el-input-number v-model="productResultForm.product_ratio" :min="0" :max="1" :step="0.01" />
        </el-form-item>
        <el-form-item class="notes-field" :label="store.tr('备注', 'Notes')">
          <el-input
            v-model="productResultForm.notes"
            type="textarea"
            :rows="2"
            :placeholder="store.tr('可填写实验现象、异常或取样说明', 'Optional observations, exceptions, or sampling notes')"
          />
        </el-form-item>
        <div class="control-actions">
          <el-button type="primary" :disabled="productResultBlocked" @click="saveProductResult">
            {{ store.tr("保存产物结果", "Save Product Result") }}
          </el-button>
          <span v-if="actionMessage" class="muted">{{ actionMessage }}</span>
        </div>
      </el-form>
      <el-alert
        v-if="productResultBlockReason"
        class="inline-alert"
        type="warning"
        :closable="false"
        show-icon
        :title="store.tr('产物结果录入已锁定', 'Product result entry locked')"
        :description="productResultBlockReason"
      />
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("产物结果", "Product Outcomes") }}</h2>
        <el-tag>{{ filteredOutcomes.length }} / {{ outcomes.length }} {{ store.tr("条", "rows") }}</el-tag>
      </div>
      <el-table
        :data="filteredOutcomes"
        class="data-table"
        size="small"
        :empty-text="
          outcomes.length > 0
            ? store.tr('当前筛选无产物结果。', 'No product outcomes match the current filters.')
            : store.tr('暂无产物结果。', 'No product outcomes yet.')
        "
      >
        <el-table-column :label="store.tr('批次', 'Batch')" width="80">
          <template #default="{ row }">{{ textAt(row, "batch_id") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('产率 %', 'Yield %')" width="100">
          <template #default="{ row }">{{ textAt(row, "yield_percent") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('产物比例', 'Product ratio')" width="120">
          <template #default="{ row }">{{ textAt(row, "product_ratio") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('目标温度', 'Target temperature')" width="120">
          <template #default="{ row }">{{ textAt(row, "target_temperature_c") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('目标搅拌', 'Target stirrer')" width="120">
          <template #default="{ row }">{{ textAt(row, "target_stirrer_rpm") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('加热时长', 'Heating minutes')" width="120">
          <template #default="{ row }">{{ textAt(row, "heating_minutes") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('搅拌时长', 'Stirring minutes')" width="120">
          <template #default="{ row }">{{ textAt(row, "stirring_minutes") }}</template>
        </el-table-column>
      </el-table>
    </section>
  </section>
</template>
