<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import PageHeader from "../components/PageHeader.vue";
import EmptyState from "../components/EmptyState.vue";
import HmiButton from "../components/HmiButton.vue";
import { auditApi } from "../api";
import { errorMessage } from "../api/errors";
import { useAuthStore } from "../stores/auth";
import { useLanguage } from "../i18n";
import { formatTimestamp, text } from "../utils/format";
import type { AuditEventItem, AuditChainStatus } from "../api/types";

const auth = useAuthStore();
const { tr } = useLanguage();

const loading = ref(false);
const exporting = ref(false);
const events = ref<AuditEventItem[]>([]);
const chainStatus = ref<AuditChainStatus | null>(null);
const total = ref(0);
const page = ref(1);
const pageSize = ref(20);

const filters = reactive({
  event_type: "",
  username: "",
  result: "",
  start_time: "",
  end_time: ""
});

const chainOk = computed(() => chainStatus.value?.valid === true);

async function load(): Promise<void> {
  loading.value = true;
  try {
    const [eventsPayload, chainPayload] = await Promise.all([
      auditApi.list({
        page: page.value,
        page_size: pageSize.value,
        event_type: filters.event_type || undefined,
        username: filters.username || undefined,
        result: filters.result || undefined,
        start_time: filters.start_time || undefined,
        end_time: filters.end_time || undefined
      }),
      auditApi.chainStatus()
    ]);
    events.value = eventsPayload?.events ?? [];
    total.value = eventsPayload?.total ?? 0;
    chainStatus.value = chainPayload ?? null;
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    loading.value = false;
  }
}

async function exportCsv(): Promise<void> {
  exporting.value = true;
  try {
    const blob = await auditApi.exportCsv({
      event_type: filters.event_type || undefined,
      username: filters.username || undefined,
      result: filters.result || undefined,
      start_time: filters.start_time || undefined,
      end_time: filters.end_time || undefined
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `audit-${new Date().toISOString().slice(0, 10)}.csv`;
    a.click();
    URL.revokeObjectURL(url);
    ElMessage.success(tr("导出成功", "Export succeeded"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    exporting.value = false;
  }
}

function resultType(result: string): "success" | "warning" | "info" | "danger" {
  if (result === "success") return "success";
  if (result === "failure" || result === "error") return "danger";
  if (result === "blocked") return "warning";
  return "info";
}

onMounted(load);
</script>

<template>
  <div class="page-stack">
    <PageHeader :title="tr('审计日志', 'Audit Log')" :subtitle="tr('事件追踪、哈希链验证与导出', 'Event tracking, hash chain verification and export')">
      <template #actions>
        <HmiButton type="manual" :disabled="exporting" @click="exportCsv">
          {{ tr("导出 CSV", "Export CSV") }}
        </HmiButton>
      </template>
    </PageHeader>

    <!-- 哈希链状态 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("哈希链状态", "Hash chain status") }}</span>
        <el-tag size="small" :type="chainOk ? 'success' : 'danger'">{{ chainOk ? tr("完整", "Valid") : tr("异常", "Invalid") }}</el-tag>
      </div>
      <div class="hmi-panel-body">
        <dl class="kv-list">
          <dt>{{ tr("总事件数", "Total events") }}</dt>
          <dd>{{ chainStatus?.total_events ?? 0 }}</dd>
          <dt>{{ tr("链起始哈希", "Genesis hash") }}</dt>
          <dd class="mono">{{ text(chainStatus?.genesis_hash) }}</dd>
          <dt>{{ tr("最新哈希", "Latest hash") }}</dt>
          <dd class="mono">{{ text(chainStatus?.latest_hash) }}</dd>
          <dt>{{ tr("验证时间", "Verified at") }}</dt>
          <dd>{{ formatTimestamp(chainStatus?.verified_at) }}</dd>
        </dl>
      </div>
    </section>

    <!-- 筛选器 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">{{ tr("筛选", "Filters") }}</div>
      <div class="hmi-panel-body">
        <el-form :inline="true" class="filter-form">
          <el-form-item :label="tr('事件类型', 'Event type')">
            <el-select v-model="filters.event_type" clearable :placeholder="tr('全部', 'All')">
              <el-option value="auth" :label="tr('认证', 'Auth')" />
              <el-option value="control" :label="tr('控制', 'Control')" />
              <el-option value="process" :label="tr('工艺', 'Process')" />
              <el-option value="batch" :label="tr('批次', 'Batch')" />
              <el-option value="config" :label="tr('配置', 'Config')" />
              <el-option value="integration" :label="tr('集成', 'Integration')" />
            </el-select>
          </el-form-item>
          <el-form-item :label="tr('用户', 'User')">
            <el-input v-model="filters.username" clearable :placeholder="tr('用户名', 'Username')" />
          </el-form-item>
          <el-form-item :label="tr('结果', 'Result')">
            <el-select v-model="filters.result" clearable :placeholder="tr('全部', 'All')">
              <el-option value="success" :label="tr('成功', 'Success')" />
              <el-option value="failure" :label="tr('失败', 'Failure')" />
              <el-option value="blocked" :label="tr('已阻止', 'Blocked')" />
            </el-select>
          </el-form-item>
          <el-form-item>
            <HmiButton type="manual" :disabled="loading" @click="load">{{ tr("查询", "Query") }}</HmiButton>
          </el-form-item>
        </el-form>
      </div>
    </section>

    <!-- 事件表格 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("事件列表", "Events") }}</span>
        <span class="muted">{{ total }}</span>
      </div>
      <div class="hmi-panel-body flush">
        <el-table v-if="events.length > 0" v-loading="loading" :data="events" size="small">
          <el-table-column prop="id" label="ID" width="70" />
          <el-table-column prop="created_at" :label="tr('时间', 'Time')" min-width="150">
            <template #default="{ row }">{{ formatTimestamp(row.created_at) }}</template>
          </el-table-column>
          <el-table-column prop="event_type" :label="tr('类型', 'Type')" width="100">
            <template #default="{ row }">
              <el-tag size="small" type="info">{{ row.event_type }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="username" :label="tr('用户', 'User')" width="100" />
          <el-table-column prop="action" :label="tr('动作', 'Action')" min-width="140" />
          <el-table-column prop="result" :label="tr('结果', 'Result')" width="90">
            <template #default="{ row }">
              <el-tag size="small" :type="resultType(String(row.result ?? ''))">{{ row.result }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="detail" :label="tr('详情', 'Detail')" min-width="200">
            <template #default="{ row }">{{ text(row.detail) }}</template>
          </el-table-column>
        </el-table>
        <EmptyState v-else icon="📋" :title="tr('暂无事件', 'No events')" />
      </div>
      <div class="hmi-panel-body" style="border-top: 1px solid var(--border-glass);">
        <el-pagination
          v-model:current-page="page"
          v-model:page-size="pageSize"
          :total="total"
          :page-sizes="[10, 20, 50, 100]"
          layout="total, sizes, prev, pager, next"
          @current-change="load"
          @size-change="load"
        />
      </div>
    </section>
  </div>
</template>

<style scoped>
.filter-form {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing);
}

.filter-form :deep(.el-form-item) {
  margin-bottom: 0;
  margin-right: 0;
}
</style>
