<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { ElMessage } from "element-plus";
import PageHeader from "../components/PageHeader.vue";
import EmptyState from "../components/EmptyState.vue";
import HmiButton from "../components/HmiButton.vue";
import { batchApi } from "../api";
import { errorMessage } from "../api/errors";
import { useAuthStore } from "../stores/auth";
import { usePlantStore } from "../stores/plant";
import { useLanguage } from "../i18n";
import { fixed, formatTimestamp, text } from "../utils/format";
import type { BatchItem, BatchSampleItem } from "../api/types";

const auth = useAuthStore();
const plant = usePlantStore();
const { tr } = useLanguage();

const loading = ref(false);
const submitting = ref(false);
const selectedBatch = ref<BatchItem | null>(null);
const samples = ref<BatchSampleItem[]>([]);

const batches = computed(() => plant.batches);

const productForm = reactive({
  product_mass_g: 0,
  product_concentration_percent: 0,
  product_ratio: 0,
  quality: "",
  note: ""
});

async function loadBatches(): Promise<void> {
  loading.value = true;
  try {
    await plant.loadBatches();
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    loading.value = false;
  }
}

async function selectBatch(batch: BatchItem): Promise<void> {
  selectedBatch.value = batch;
  try {
    const payload = await batchApi.samples(batch.id, 200);
    samples.value = payload?.samples ?? [];
  } catch (error) {
    ElMessage.error(errorMessage(error));
  }
}

async function submitProductResult(): Promise<void> {
  if (!selectedBatch.value) return;
  submitting.value = true;
  try {
    const body: Record<string, unknown> = {
      product_mass_g: productForm.product_mass_g,
      product_concentration_percent: productForm.product_concentration_percent,
      product_ratio: productForm.product_ratio
    };
    if (productForm.quality.trim()) body.quality = productForm.quality.trim();
    if (productForm.note.trim()) body.note = productForm.note.trim();
    await batchApi.recordResult(selectedBatch.value.id, body as never);
    ElMessage.success(tr("产物结果已录入", "Product result recorded"));
    await loadBatches();
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
  }
}

function batchStatusType(status: string): "success" | "warning" | "info" | "danger" {
  if (status === "completed") return "success";
  if (status === "running") return "warning";
  if (status === "failed" || status === "aborted") return "danger";
  return "info";
}

onMounted(loadBatches);
</script>

<template>
  <div class="page-stack">
    <PageHeader :title="tr('历史数据', 'History')" :subtitle="tr('批次、产物结果与样本时序', 'Batches, product results and sample time series')">
      <template #actions>
        <HmiButton type="manual" :disabled="loading" @click="loadBatches">
          {{ tr("刷新", "Refresh") }}
        </HmiButton>
      </template>
    </PageHeader>

    <div class="history-layout">
      <!-- 批次列表 -->
      <section class="hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("批次列表", "Batch list") }}</span>
          <span class="muted">{{ batches.length }}</span>
        </div>
        <div class="hmi-panel-body flush">
          <el-table v-if="batches.length > 0" v-loading="loading" :data="batches" size="small" @row-click="selectBatch">
            <el-table-column prop="id" label="ID" width="70" />
            <el-table-column prop="process_name" :label="tr('工艺', 'Process')" min-width="140">
              <template #default="{ row }">{{ text(row.process_name) }}</template>
            </el-table-column>
            <el-table-column :label="tr('状态', 'Status')" width="100">
              <template #default="{ row }">
                <el-tag size="small" :type="batchStatusType(String(row.status ?? ''))">{{ row.status }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="tr('开始时间', 'Start')" min-width="150">
              <template #default="{ row }">{{ formatTimestamp(row.started_at) }}</template>
            </el-table-column>
            <el-table-column :label="tr('结束时间', 'End')" min-width="150">
              <template #default="{ row }">{{ formatTimestamp(row.finished_at) }}</template>
            </el-table-column>
          </el-table>
          <EmptyState v-else icon="📦" :title="tr('暂无批次', 'No batches')" />
        </div>
      </section>

      <!-- 批次详情 -->
      <section v-if="selectedBatch" class="hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("批次详情", "Batch detail") }} #{{ selectedBatch.id }}</span>
          <el-tag size="small" :type="batchStatusType(String(selectedBatch.status ?? ''))">{{ selectedBatch.status }}</el-tag>
        </div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>{{ tr("工艺", "Process") }}</dt>
            <dd>{{ text(selectedBatch.process_name) }}</dd>
            <dt>{{ tr("开始时间", "Start") }}</dt>
            <dd>{{ formatTimestamp(selectedBatch.started_at) }}</dd>
            <dt>{{ tr("结束时间", "End") }}</dt>
            <dd>{{ formatTimestamp(selectedBatch.finished_at) }}</dd>
            <dt>{{ tr("产物质量", "Product mass") }}</dt>
            <dd>{{ fixed(selectedBatch.product_mass_g ?? null, 1) }} g</dd>
            <dt>{{ tr("产物浓度", "Concentration") }}</dt>
            <dd>{{ fixed(selectedBatch.product_concentration_percent ?? null, 1) }} %</dd>
            <dt>{{ tr("产物比率", "Ratio") }}</dt>
            <dd>{{ fixed(selectedBatch.product_ratio ?? null, 2) }}</dd>
          </dl>

          <!-- 样本时序 -->
          <div v-if="samples.length > 0" class="samples-section">
            <h4>{{ tr("样本时序", "Sample time series") }}</h4>
            <el-table :data="samples" size="small" max-height="240">
              <el-table-column prop="captured_at" :label="tr('时间', 'Time')" min-width="150">
                <template #default="{ row }">{{ formatTimestamp(row.captured_at) }}</template>
              </el-table-column>
              <el-table-column prop="temperature_c" :label="tr('温度', 'Temp')" width="90">
                <template #default="{ row }">{{ fixed(row.temperature_c ?? null, 1) }}°C</template>
              </el-table-column>
              <el-table-column prop="pressure_mpa" :label="tr('压力', 'Press')" width="90">
                <template #default="{ row }">{{ fixed(row.pressure_mpa ?? null, 2) }}MPa</template>
              </el-table-column>
              <el-table-column prop="stirrer_rpm" :label="tr('转速', 'RPM')" width="80">
                <template #default="{ row }">{{ fixed(row.stirrer_rpm ?? null, 0) }}</template>
              </el-table-column>
            </el-table>
          </div>

          <!-- 产物录入 -->
          <div v-if="auth.isAuthenticated && selectedBatch.status === 'completed'" class="product-form">
            <h4>{{ tr("录入产物结果", "Record product result") }}</h4>
            <el-form label-position="top">
              <el-form-item :label="tr('产物质量 (g)', 'Product mass (g)')">
                <el-input-number v-model="productForm.product_mass_g" :min="0" controls-position="right" class="full-width" />
              </el-form-item>
              <el-form-item :label="tr('产物浓度 (%)', 'Concentration (%)')">
                <el-input-number v-model="productForm.product_concentration_percent" :min="0" :max="100" controls-position="right" class="full-width" />
              </el-form-item>
              <el-form-item :label="tr('产物比率', 'Ratio')">
                <el-input-number v-model="productForm.product_ratio" :min="0" :max="1" :step="0.01" controls-position="right" class="full-width" />
              </el-form-item>
              <el-form-item :label="tr('质量等级', 'Quality')">
                <el-input v-model="productForm.quality" :placeholder="tr('如：优/良/合格', 'e.g. excellent/good/pass')" />
              </el-form-item>
              <el-form-item :label="tr('备注', 'Note')">
                <el-input v-model="productForm.note" type="textarea" :rows="2" />
              </el-form-item>
              <HmiButton type="start" :disabled="submitting" @click="submitProductResult">
                {{ tr("提交", "Submit") }}
              </HmiButton>
            </el-form>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.history-layout {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--spacing);
  align-items: start;
}

.samples-section {
  margin-top: var(--spacing);
}

.samples-section h4,
.product-form h4 {
  margin: 0 0 var(--spacing);
  font-size: var(--fs-md);
  color: var(--text-primary);
}

.product-form {
  margin-top: var(--spacing);
  padding-top: var(--spacing);
  border-top: 1px solid var(--border-glass);
}

.full-width {
  width: 100%;
}

@media (max-width: 1000px) {
  .history-layout {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
