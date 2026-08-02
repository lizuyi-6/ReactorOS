<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { ElMessage } from "element-plus";
import PageHeader from "../components/PageHeader.vue";
import EmptyState from "../components/EmptyState.vue";
import HmiButton from "../components/HmiButton.vue";
import { modbusApi } from "../api";
import { errorMessage } from "../api/errors";
import { useAuthStore } from "../stores/auth";
import { useLiveStore } from "../stores/live";
import { useLanguage } from "../i18n";
import { text } from "../utils/format";
import type { ModbusRegisterItem } from "../api/types";

const auth = useAuthStore();
const live = useLiveStore();
const { tr } = useLanguage();

const loading = ref(false);
const submitting = ref(false);
const registers = ref<ModbusRegisterItem[]>([]);
const writeRegisters = ref<ModbusRegisterItem[]>([]);
const coils = ref<ModbusRegisterItem[]>([]);
const discreteInputs = ref<ModbusRegisterItem[]>([]);
const tcpStatus = ref<Record<string, unknown> | null>(null);
const meta = ref<{ device_id?: string; mode?: string; slave_id?: number }>({});

const form = reactive({ readRegister: "", writeRegister: "", value: 0, reason: "" });
const readResult = ref<Record<string, unknown> | null>(null);

const allRegisterNames = computed(() => [...registers.value, ...writeRegisters.value].map((r) => r.name));
const writeRegisterNames = computed(() => writeRegisters.value.map((r) => r.name));

const writeDisabled = computed(
  () =>
    !auth.isAdmin ||
    submitting.value ||
    !form.writeRegister ||
    !form.reason.trim() ||
    live.liveStatus !== "fresh"
);

async function load(): Promise<void> {
  loading.value = true;
  try {
    const payload = await modbusApi.registers();
    registers.value = payload?.read_registers ?? [];
    writeRegisters.value = payload?.write_registers ?? [];
    coils.value = payload?.coils ?? [];
    discreteInputs.value = payload?.discrete_inputs ?? [];
    tcpStatus.value = (payload?.tcp ?? null) as Record<string, unknown> | null;
    meta.value = { device_id: payload?.device_id, mode: payload?.mode, slave_id: payload?.slave_id };
    if (!form.readRegister && allRegisterNames.value.length > 0) form.readRegister = allRegisterNames.value[0];
    if (!form.writeRegister && writeRegisterNames.value.length > 0) form.writeRegister = writeRegisterNames.value[0];
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    loading.value = false;
  }
}

async function readRegister(): Promise<void> {
  if (!form.readRegister) return;
  submitting.value = true;
  try {
    readResult.value = await modbusApi.read(form.readRegister);
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
  }
}

async function writeRegister(): Promise<void> {
  submitting.value = true;
  try {
    await modbusApi.write(form.writeRegister, { value: form.value, reason: form.reason.trim() });
    ElMessage.success(tr("写入成功", "Write succeeded"));
    await load();
    form.readRegister = form.writeRegister;
    await readRegister();
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page-stack">
    <PageHeader :title="tr('Modbus 调试', 'Modbus Debug')" :subtitle="tr('寄存器映射、读写与通道状态', 'Register map, read/write and channel status')">
      <template #actions>
        <el-tag :type="auth.isAdmin ? 'success' : 'info'" size="small">
          {{ auth.isAdmin ? tr("管理员可写", "Admin write") : tr("只读（写需 admin）", "Read-only (admin to write)") }}
        </el-tag>
      </template>
    </PageHeader>

    <div class="modbus-grid">
      <!-- 调试面板 -->
      <section class="hmi-panel">
        <div class="hmi-panel-header">{{ tr("寄存器读写", "Read / write") }}</div>
        <div class="hmi-panel-body debug-panel">
          <div class="debug-row">
            <el-select v-model="form.readRegister" size="default" :placeholder="tr('选择寄存器', 'Select register')" class="reg-select" filterable>
              <el-option v-for="name in allRegisterNames" :key="name" :value="name" :label="name" />
            </el-select>
            <HmiButton type="manual" :disabled="!form.readRegister" @click="readRegister">
              {{ tr("读取", "Read") }}
            </HmiButton>
          </div>

          <div v-if="readResult" class="read-result">
            <dl class="kv-list">
              <dt>{{ tr("寄存器", "Register") }}</dt>
              <dd>{{ text(readResult.register) }}</dd>
              <dt>{{ tr("地址", "Address") }}</dt>
              <dd>{{ text(readResult.address) }}</dd>
              <dt>{{ tr("值", "Value") }}</dt>
              <dd>{{ text(readResult.value) }}</dd>
              <dt>{{ tr("原始值", "Raw") }}</dt>
              <dd>{{ text(readResult.raw) }}</dd>
              <dt>{{ tr("来源", "Source") }}</dt>
              <dd>{{ text(readResult.source) }}</dd>
            </dl>
          </div>

          <el-divider />

          <el-alert
            v-if="!auth.isAdmin"
            type="info"
            :closable="false"
            show-icon
            :title="tr('写寄存器需要 admin 角色', 'Writing registers requires the admin role')"
            class="write-alert"
          />
          <div class="debug-row">
            <el-select v-model="form.writeRegister" size="default" :placeholder="tr('可写寄存器', 'Writable register')" class="reg-select" filterable>
              <el-option v-for="name in writeRegisterNames" :key="name" :value="name" :label="name" />
            </el-select>
            <el-input-number v-model="form.value" :step="1" controls-position="right" class="value-input" />
          </div>
          <el-input v-model="form.reason" :placeholder="tr('请输入写入原因（必填）', 'Enter write reason (required)')" maxlength="120" />
          <HmiButton type="start" :disabled="writeDisabled" @click="writeRegister">
            {{ tr("写入并读回", "Write & read back") }}
          </HmiButton>
        </div>
      </section>

      <!-- 通道状态 -->
      <section class="hmi-panel">
        <div class="hmi-panel-header">{{ tr("通道状态", "Channel status") }}</div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>{{ tr("设备", "Device") }}</dt>
            <dd>{{ text(meta.device_id) }}</dd>
            <dt>{{ tr("模式", "Mode") }}</dt>
            <dd>{{ text(meta.mode) }}</dd>
            <dt>Slave ID</dt>
            <dd>{{ text(meta.slave_id) }}</dd>
            <dt>TCP</dt>
            <dd>{{ tcpStatus?.listening ? tr("监听中", "Listening") : tr("未启用", "Disabled") }}</dd>
            <dt>{{ tr("绑定", "Bind") }}</dt>
            <dd>{{ text(tcpStatus?.bind) }}</dd>
            <dt>TLS</dt>
            <dd>{{ tcpStatus?.tls ? "on" : "off" }}</dd>
          </dl>
        </div>
      </section>
    </div>

    <!-- 寄存器表 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("读寄存器", "Read registers") }}</span>
        <span class="muted">{{ registers.length }}</span>
      </div>
      <div class="hmi-panel-body flush">
        <el-table v-if="registers.length > 0" v-loading="loading" :data="registers" size="small">
          <el-table-column prop="address" :label="tr('地址', 'Addr')" width="80" />
          <el-table-column prop="name" :label="tr('名称', 'Name')" min-width="180">
            <template #default="{ row }"><span class="mono">{{ row.name }}</span></template>
          </el-table-column>
          <el-table-column prop="access" :label="tr('访问', 'Access')" width="90" />
          <el-table-column :label="tr('当前值', 'Value')" width="120">
            <template #default="{ row }"><span class="mono">{{ text(row.value) }}</span></template>
          </el-table-column>
          <el-table-column :label="tr('原始值', 'Raw')" width="100">
            <template #default="{ row }"><span class="mono">{{ text(row.raw) }}</span></template>
          </el-table-column>
          <el-table-column prop="source" :label="tr('来源', 'Source')" min-width="120">
            <template #default="{ row }">{{ text(row.source) }}</template>
          </el-table-column>
        </el-table>
        <EmptyState v-else icon="MB" :title="tr('无读寄存器', 'No read registers')" />
      </div>
    </section>

    <div class="modbus-grid">
      <section class="hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("线圈", "Coils") }}</span>
          <span class="muted">{{ coils.length }}</span>
        </div>
        <div class="hmi-panel-body flush">
          <el-table v-if="coils.length > 0" :data="coils" size="small">
            <el-table-column prop="address" :label="tr('地址', 'Addr')" width="80" />
            <el-table-column prop="name" :label="tr('名称', 'Name')" min-width="160">
              <template #default="{ row }"><span class="mono">{{ row.name }}</span></template>
            </el-table-column>
            <el-table-column :label="tr('值', 'Value')" width="100">
              <template #default="{ row }">
                <el-tag size="small" :type="row.value ? 'success' : 'info'">{{ row.value ? "ON" : "OFF" }}</el-tag>
              </template>
            </el-table-column>
          </el-table>
          <EmptyState v-else icon="◌" :title="tr('无线圈', 'No coils')" />
        </div>
      </section>

      <section class="hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("离散输入", "Discrete inputs") }}</span>
          <span class="muted">{{ discreteInputs.length }}</span>
        </div>
        <div class="hmi-panel-body flush">
          <el-table v-if="discreteInputs.length > 0" :data="discreteInputs" size="small">
            <el-table-column prop="address" :label="tr('地址', 'Addr')" width="80" />
            <el-table-column prop="name" :label="tr('名称', 'Name')" min-width="160">
              <template #default="{ row }"><span class="mono">{{ row.name }}</span></template>
            </el-table-column>
            <el-table-column :label="tr('值', 'Value')" width="100">
              <template #default="{ row }">
                <el-tag size="small" :type="row.value ? 'success' : 'info'">{{ row.value ? "ON" : "OFF" }}</el-tag>
              </template>
            </el-table-column>
          </el-table>
          <EmptyState v-else icon="◌" :title="tr('无离散输入', 'No discrete inputs')" />
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.modbus-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--spacing);
  align-items: start;
}

.debug-panel {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
}

.debug-row {
  display: flex;
  gap: var(--spacing);
}

.reg-select {
  flex: 1;
}

.value-input {
  width: 160px;
}

.read-result {
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: var(--spacing);
}

.write-alert {
  margin-bottom: var(--spacing);
}

@media (max-width: 1000px) {
  .modbus-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
