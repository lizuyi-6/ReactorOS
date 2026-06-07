<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, numberAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const batches = computed(() => arrayAt(store.batches, "batches"));
const outcomes = computed(() => arrayAt(store.batches, "outcomes"));
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
    label: `#${textAt(row, "id")} - ${textAt(row, "name")}`
  }))
);

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
        <p class="eyebrow">{{ store.tr("SQLite 历史 API", "SQLite History API") }}</p>
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
            <template #default="{ row }">{{ textAt(row, "started_at") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('结束', 'Finished')" min-width="160">
            <template #default="{ row }">{{ textAt(row, "finished_at") || "--" }}</template>
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
            <el-descriptions-item :label="store.tr('开始', 'Started')">{{ textAt(selectedBatch, "started_at") }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('结束', 'Finished')">{{ textAt(selectedBatch, "finished_at") || "--" }}</el-descriptions-item>
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
              :disabled="batch.id === null"
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
          <el-button type="primary" :disabled="!store.isAuthenticated || !productResultForm.batch_id" @click="saveProductResult">
            {{ store.tr("保存产物结果", "Save Product Result") }}
          </el-button>
          <span v-if="actionMessage" class="muted">{{ actionMessage }}</span>
        </div>
      </el-form>
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
