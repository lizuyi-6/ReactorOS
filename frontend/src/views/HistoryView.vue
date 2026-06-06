<script setup lang="ts">
import { computed, ref } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, numberAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const batches = computed(() => arrayAt(store.batches, "batches"));
const outcomes = computed(() => arrayAt(store.batches, "outcomes"));
const selectedBatch = ref<Record<string, unknown> | null>(null);
const selectedOutcome = computed(() => {
  if (!selectedBatch.value) return null;
  const id = numberAt(selectedBatch.value, "id");
  if (id === null) return null;
  return outcomes.value.find((row) => numberAt(row, "batch_id") === id) ?? null;
});
const reportUrl = ref<string | null>(null);
const reportBytes = ref<number | null>(null);
const actionMessage = ref("");

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
    selectedBatch.value = null;
    reportUrl.value = null;
    reportBytes.value = null;
    return;
  }
  try {
    selectedBatch.value = await store.loadBatchDetail(id);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function downloadBatchesCsv(): Promise<void> {
  try {
    store.error = null;
    actionMessage.value = "";
    const blob = await store.requestBlob("/api/batches/export.csv");
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `batches-${Date.now()}.csv`;
    anchor.click();
    URL.revokeObjectURL(url);
    actionMessage.value = store.tr("批次 CSV 已下载", "Batch CSV downloaded");
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
    if (reportUrl.value) URL.revokeObjectURL(reportUrl.value);
    reportUrl.value = URL.createObjectURL(blob);
    reportBytes.value = blob.size;
    const anchor = document.createElement("a");
    anchor.href = reportUrl.value;
    anchor.download = `batch-${id}-report.md`;
    anchor.click();
    actionMessage.value = store.tr("报告已下载", "Report downloaded");
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("SQLite 历史 API", "SQLite History API") }}</p>
        <h1>{{ store.tr("历史数据", "History Data") }}</h1>
        <span>{{ store.tr("批次、产物结果、报告与 CSV 导出", "Batches, outcomes, reports, and CSV export") }}</span>
      </div>
      <div class="heading-actions">
        <el-tag>{{ store.tr(`${batches.length} 个批次`, `${batches.length} batches`) }}</el-tag>
        <el-button :disabled="!store.isAuthenticated" @click="downloadBatchesCsv">
          {{ store.tr("导出 CSV", "Export CSV") }}
        </el-button>
        <el-button :disabled="!store.isAuthenticated" @click="refresh">
          {{ store.tr("刷新", "Refresh") }}
        </el-button>
      </div>
    </div>

    <section class="panel two-col">
      <div class="data-table-wrap">
        <el-table :data="batches" class="data-table" size="small" @row-click="(row) => selectBatch(numberAt(row, 'id'))">
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
            <el-descriptions-item :label="store.tr('产品', 'Product')">{{ textAt(selectedOutcome, "product", store.tr("未填写", "n/a")) }}</el-descriptions-item>
            <el-descriptions-item :label="store.tr('产率', 'Yield')">{{ textAt(selectedOutcome, "yield_percent", "--") }} %</el-descriptions-item>
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
        <h2>{{ store.tr("产物结果", "Product Outcomes") }}</h2>
        <el-tag>{{ outcomes.length }} {{ store.tr("条", "rows") }}</el-tag>
      </div>
      <el-table v-if="outcomes.length > 0" :data="outcomes" class="data-table" size="small">
        <el-table-column :label="store.tr('批次', 'Batch')" width="80">
          <template #default="{ row }">{{ textAt(row, "batch_id") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('产品', 'Product')" min-width="160">
          <template #default="{ row }">{{ textAt(row, "product") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('产率 %', 'Yield %')" width="100">
          <template #default="{ row }">{{ textAt(row, "yield_percent") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('记录于', 'Recorded')" min-width="160">
          <template #default="{ row }">{{ textAt(row, "recorded_at") }}</template>
        </el-table-column>
      </el-table>
      <p v-else class="muted">{{ store.tr("暂无产物结果。", "No product outcomes yet.") }}</p>
    </section>
  </section>
</template>
