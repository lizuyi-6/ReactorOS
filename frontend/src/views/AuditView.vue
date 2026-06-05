<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { ElMessage } from "element-plus";
import { usePlantStore } from "../stores/plant";
import { arrayAt, numberAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const exporting = ref(false);
const loadingAudit = ref(false);
const auditFilters = reactive({
  eventType: "",
  pageSize: 20
});

const events = computed(() => arrayAt(store.audit, "events"));
const chain = computed(() => objectAt(store.audit, "chain"));
const total = computed(() => numberAt(store.audit, "total") ?? events.value.length);
const page = computed(() => numberAt(store.audit, "page") ?? 1);
const pageSize = computed(() => numberAt(store.audit, "page_size") ?? auditFilters.pageSize);
const chainValid = computed(() => textAt(chain.value, "valid", "false") === "true");
const windowValid = computed(() => textAt(chain.value, "window_valid", "false") === "true");
const truncated = computed(() => textAt(chain.value, "verification_truncated", "false") === "true");

const eventTypeOptions = [
  { labelZh: "全部事件", labelEn: "All events", value: "" },
  { labelZh: "目标写入", labelEn: "Target writes", value: "operator_targets_updated" },
  { labelZh: "Modbus 写入", labelEn: "Modbus writes", value: "modbus_register_write" },
  { labelZh: "AI 决策", labelEn: "AI decisions", value: "ai_master_decision" },
  { labelZh: "工艺启动", labelEn: "Process started", value: "process_started" },
  { labelZh: "工艺停止", labelEn: "Process stopped", value: "process_stopped" },
  { labelZh: "批次完成", labelEn: "Batch finished", value: "batch_finished" }
];

const chainMetrics = computed(() => [
  {
    label: store.tr("哈希事件总数", "Hashed events"),
    value: textAt(chain.value, "total_hashed_events", "0"),
    helper: store.tr("已写入防篡改链的事件数", "Events persisted in the tamper-evident chain")
  },
  {
    label: store.tr("本次校验", "Checked window"),
    value: textAt(chain.value, "checked_events", "0"),
    helper: `${textAt(chain.value, "checked_from_event_id")} -> ${textAt(chain.value, "checked_to_event_id")}`
  },
  {
    label: store.tr("链式事件", "Chained events"),
    value: textAt(chain.value, "chained_events", "0"),
    helper: store.tr("具有 previous_hash 的连续事件", "Continuous events carrying previous_hash")
  },
  {
    label: store.tr("断链事件", "Broken events"),
    value: textAt(chain.value, "broken_events", "0"),
    helper: store.tr("校验窗口内发现的断链数量", "Broken links found in the verification window")
  }
]);

async function loadAudit(pageNumber = 1): Promise<void> {
  loadingAudit.value = true;
  store.error = null;
  try {
    await store.loadAudit({
      page: pageNumber,
      pageSize: auditFilters.pageSize,
      eventType: auditFilters.eventType
    });
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  } finally {
    loadingAudit.value = false;
  }
}

async function exportCsv(): Promise<void> {
  exporting.value = true;
  store.error = null;
  try {
    const blob = await store.exportAuditCsv(auditFilters.eventType);
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "reactor-audit-log.csv";
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);
    ElMessage.success(store.tr("审计 CSV 已导出", "Audit CSV exported"));
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  } finally {
    exporting.value = false;
  }
}
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("防篡改链", "Tamper-evident Chain") }}</p>
        <h1>{{ store.tr("审计日志", "Audit Log") }}</h1>
        <span>{{ store.tr("操作事件、哈希链校验和 CSV 导出权限", "Operation events, hash-chain verification, and CSV export permissions") }}</span>
      </div>
      <div class="heading-actions">
        <el-tag :type="chainValid ? 'success' : 'danger'">
          {{ chainValid ? store.tr("链校验通过", "Chain valid") : store.tr("链校验异常", "Chain warning") }}
        </el-tag>
        <el-tag :type="windowValid ? 'success' : 'warning'">
          {{ windowValid ? store.tr("窗口通过", "Window valid") : store.tr("窗口异常", "Window warning") }}
        </el-tag>
      </div>
    </div>

    <section class="metric-grid">
      <div v-for="metric in chainMetrics" :key="metric.label" class="metric">
        <span>{{ metric.label }}</span>
        <strong>{{ metric.value }}</strong>
        <small>{{ metric.helper }}</small>
      </div>
    </section>

    <section class="panel audit-toolbar">
      <el-form label-position="top" class="audit-filter-form">
        <el-form-item :label="store.tr('事件类型', 'Event type')">
          <el-select v-model="auditFilters.eventType" filterable :placeholder="store.tr('全部事件', 'All events')">
            <el-option
              v-for="option in eventTypeOptions"
              :key="option.value || 'all'"
              :label="store.tr(option.labelZh, option.labelEn)"
              :value="option.value"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="store.tr('每页条数', 'Page size')">
          <el-input-number v-model="auditFilters.pageSize" :min="5" :max="100" :step="5" controls-position="right" />
        </el-form-item>
        <div class="control-actions">
          <el-button :loading="loadingAudit" :disabled="!store.isAuthenticated" @click="loadAudit(1)">
            {{ store.tr("查询审计", "Query Audit") }}
          </el-button>
          <el-button type="primary" :loading="exporting" :disabled="!store.isAuthenticated" @click="exportCsv">
            {{ store.tr("导出 CSV", "Export CSV") }}
          </el-button>
        </div>
      </el-form>
      <div class="audit-window-note">
        <strong>{{ store.tr("校验窗口", "Verification window") }}</strong>
        <span>
          {{ store.tr("上限", "Limit") }} {{ textAt(chain, "verification_limit", "0") }}
          · {{ store.tr("已截断", "Truncated") }} {{ truncated ? store.tr("是", "yes") : store.tr("否", "no") }}
        </span>
        <small>{{ store.tr("最后事件哈希", "Last event hash") }} {{ textAt(chain, "last_event_hash") }}</small>
      </div>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("事件列表", "Event List") }}</h2>
        <span>{{ store.tr(`第 ${page} 页，共 ${total} 条`, `Page ${page}, ${total} total`) }}</span>
      </div>
      <el-table :data="events" class="data-table">
        <el-table-column prop="id" label="#" width="80" />
        <el-table-column prop="created_at" :label="store.tr('时间', 'Time')" width="190" />
        <el-table-column prop="event_type" :label="store.tr('事件', 'Event')" width="190" />
        <el-table-column :label="store.tr('目标', 'Target')" width="170">
          <template #default="{ row }">
            {{ textAt(row, "target_temperature_c") }} C /
            {{ textAt(row, "target_stirrer_rpm") }} rpm
          </template>
        </el-table-column>
        <el-table-column prop="reason" :label="store.tr('原因', 'Reason')" />
        <el-table-column :label="store.tr('哈希', 'Hash')" width="150">
          <template #default="{ row }">
            <span class="hash-cell">{{ textAt(row, "event_hash") }}</span>
          </template>
        </el-table-column>
      </el-table>
      <div class="table-footer">
        <el-button :disabled="!store.isAuthenticated || page <= 1" @click="loadAudit(page - 1)">
          {{ store.tr("上一页", "Previous") }}
        </el-button>
        <el-button :disabled="!store.isAuthenticated || page * pageSize >= total" @click="loadAudit(page + 1)">
          {{ store.tr("下一页", "Next") }}
        </el-button>
      </div>
    </section>
  </section>
</template>
