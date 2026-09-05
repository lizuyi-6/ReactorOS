<script setup lang="ts">
// Modbus Debug 设备通信调试页（2025 重构版）
// 数据来源：
// - plant.modbus（/api/modbus/registers）：tcp/serial/slave_id + read/write/coils/discrete 寄存器映射
// - plant.deviceStatus + live.primaryDevice：设备在线状态与组件健康
// - live.recentSamples：温度趋势
// - modbusApi.read/write：读/写寄存器（写仅 admin，写前确认、写后审计刷新）
// - 前端实测 API 往返延迟（performance.now），真实测量而非编造
import { computed, onMounted, reactive, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import PanelCard from "../components/PanelCard.vue";
import SparkLine from "../components/SparkLine.vue";
import TrendChart, { type TrendSeries } from "../components/TrendChart.vue";
import AppIcon from "../components/AppIcon.vue";
import { modbusApi } from "../api";
import { errorMessage } from "../api/errors";
import { useAuthStore } from "../stores/auth";
import { useLiveStore } from "../stores/live";
import { usePlantStore } from "../stores/plant";
import { useLanguage } from "../i18n";
import { fixed, formatTime, text } from "../utils/format";
import type { ModbusRegisterItem } from "../api/types";

const auth = useAuthStore();
const live = useLiveStore();
const plant = usePlantStore();
const { tr, language } = useLanguage();

// ---------- 本地状态 ----------

interface OpRecord {
  time: string;
  type: "read" | "write";
  slave: string;
  address: string;
  value: string;
  result: "success" | "failed";
  operator: string;
}

const reading = ref(false);
const writing = ref(false);
/** 最近一次写入的本地记录（时间/操作员/结果）。 */
const lastWrite = ref<{ time: string; operator: string; ok: boolean } | null>(null);
/** 页面内最近读写操作（read/write 完成后 unshift，纯本地）。 */
const operations = ref<OpRecord[]>([]);
const showAllOps = ref(false);
/** 前端实测 API 往返延迟（ms，滚动窗口）。 */
const latencyHistory = ref<number[]>([]);
/** read() 刷新过的最新值覆盖层（register name -> value）。 */
const localValues = ref<Record<string, number | string | null>>({});
/** 最近一次读取的寄存器名（值网格高亮）。 */
const activeRegister = ref<string>("");

const readForm = reactive({ slaveId: "1", function: "03", address: "", quantity: "10" });
const writeForm = reactive({ slaveId: "1", function: "06", register: "", value: "", reason: "" });
const filters = reactive({ search: "", type: "", status: "" });

const READ_FUNCTIONS = [
  { value: "01", label: "01 Read Coils" },
  { value: "02", label: "02 Read Discrete Inputs" },
  { value: "03", label: "03 Read Holding" },
  { value: "04", label: "04 Read Input" }
];
const WRITE_FUNCTIONS = [
  { value: "05", label: "05 Write Single Coil" },
  { value: "06", label: "06 Write Single" },
  { value: "16", label: "16 Write Multiple" }
];

// ---------- 数据派生 ----------

const modbus = computed(() => plant.modbus);
const tcpConnected = computed(() => Boolean(modbus.value?.tcp?.listening));
const tcpBind = computed(() => text(modbus.value?.tcp?.bind));
const rtuActive = computed(() => modbus.value?.mode === "modbus");
const slaveId = computed(() => modbus.value?.slave_id ?? null);
const slaveLabel = computed(() => (slaveId.value === null ? "--" : String(slaveId.value)));
const lastPoll = computed(() => {
  const at = modbus.value?.tcp?.updated_at;
  return at ? formatTime(at) : "--";
});
const latestLatency = computed(() => {
  const arr = latencyHistory.value;
  return arr.length > 0 ? Math.round(arr[arr.length - 1]) : null;
});

/** 串口描述："COM3 (9600, 8N1)" 样式，字段缺失显示 "--"。 */
const serialLabel = computed(() => {
  const serial = (modbus.value?.serial ?? null) as Record<string, unknown> | null;
  if (!serial) return "--";
  const port = text(serial.port, "");
  const baud = text(serial.baudrate, "");
  const bits = text(serial.bytesize, "");
  const parity = String(serial.parity ?? "").trim().toUpperCase().charAt(0);
  const stop = text(serial.stopbits, "");
  if (!port && !baud) return "--";
  const frame = [bits, parity, stop].filter(Boolean).join("");
  return [port, baud ? `${baud}${frame ? ` (${frame})` : ""}` : ""].filter(Boolean).join(" ");
});

const readRegisters = computed<ModbusRegisterItem[]>(() => modbus.value?.read_registers ?? []);
const writeRegisters = computed<ModbusRegisterItem[]>(() => modbus.value?.write_registers ?? []);
const coils = computed<ModbusRegisterItem[]>(() => modbus.value?.coils ?? []);
const discreteInputs = computed<ModbusRegisterItem[]>(() => modbus.value?.discrete_inputs ?? []);

interface MapRow {
  name: string;
  label: string;
  address: number | null;
  access: string;
  value: number | string | boolean | null | undefined;
  unit: string;
  group: string;
  hasValue: boolean;
}

function toMapRow(item: ModbusRegisterItem, group: string): MapRow {
  const value = localValues.value[item.name] !== undefined ? localValues.value[item.name] : item.value;
  return {
    name: item.name,
    label: item.label ?? item.name,
    address: typeof item.address === "number" ? item.address : null,
    access: item.access ?? "",
    value,
    unit: item.unit ?? "",
    group,
    hasValue: value !== null && value !== undefined
  };
}

/** 寄存器映射合并行：read + write + coils + discrete_inputs，前端过滤。 */
const mapRows = computed<MapRow[]>(() => [
  ...readRegisters.value.map((r) => toMapRow(r, "read")),
  ...writeRegisters.value.map((r) => toMapRow(r, "write")),
  ...coils.value.map((r) => toMapRow(r, "coil")),
  ...discreteInputs.value.map((r) => toMapRow(r, "discrete"))
]);

const accessOptions = computed(() => {
  const unique = Array.from(new Set(mapRows.value.map((r) => r.access).filter(Boolean)));
  return unique.sort().map((a) => ({ value: a, label: accessLabel(a) }));
});

function accessLabel(access: string): string {
  if (access === "read") return "RO";
  if (access === "write") return "WO";
  if (access === "read_write") return "RW";
  return access.toUpperCase();
}

const filteredMapRows = computed(() => {
  const q = filters.search.trim().toLowerCase();
  return mapRows.value.filter((row) => {
    if (q && !row.name.toLowerCase().includes(q) && !row.label.toLowerCase().includes(q)) return false;
    if (filters.type && row.access !== filters.type) return false;
    if (filters.status === "good" && !row.hasValue) return false;
    if (filters.status === "nodata" && row.hasValue) return false;
    return true;
  });
});

/** Read 面板值网格：read_registers 前 10 项（地址小字 + 值大字）。 */
const valueCards = computed(() =>
  readRegisters.value.slice(0, 10).map((r) => ({
    name: r.name,
    address: typeof r.address === "number" ? r.address : null,
    value: localValues.value[r.name] !== undefined ? localValues.value[r.name] : r.value
  }))
);

function formatValue(value: unknown): string {
  if (value === null || value === undefined || value === "") return "--";
  if (typeof value === "boolean") return value ? "ON" : "OFF";
  if (typeof value === "number") return Number.isInteger(value) ? String(value) : value.toFixed(2);
  return String(value);
}

// ---------- 温度趋势（live.recentSamples） ----------

const tempSeries = computed<TrendSeries[]>(() => [
  {
    name: tr("温度", "Temp"),
    unit: "°C",
    color: "#2f9bff",
    smooth: true,
    data: live.recentSamples.map((s) => [
      new Date(s.captured_at ?? s.created_at ?? Date.now()).getTime(),
      typeof s.temperature_c === "number" ? s.temperature_c : null
    ])
  }
]);
const currentTemp = computed(() => live.latestSample?.temperature_c ?? null);

// ---------- 设备与组件健康 ----------

const deviceOnline = computed(() => {
  const device = live.primaryDevice ?? plant.deviceStatus?.devices?.[0] ?? null;
  return Boolean(device?.online);
});

interface ComponentRow {
  id: string;
  label: string;
  type: string;
  status: string;
  controllable: boolean;
}

const componentRows = computed<ComponentRow[]>(() => {
  const raw =
    live.primaryDevice?.components ?? plant.deviceStatus?.devices?.[0]?.components ?? [];
  return (raw as unknown[]).map((item) => {
    const c = (item ?? {}) as Record<string, unknown>;
    return {
      id: String(c.component_id ?? c.id ?? "--"),
      label: String(c.label ?? ""),
      type: String(c.component_type ?? "--"),
      status: String(c.status ?? c.state ?? "--"),
      controllable: Boolean(c.controllable)
    };
  });
});

function componentIcon(row: ComponentRow): string {
  const key = `${row.id} ${row.type}`.toLowerCase();
  if (key.includes("heater") || key.includes("relay")) return "heater";
  if (key.includes("stepper") || key.includes("motor")) return "motor";
  if (key.includes("temperature")) return "gauge";
  if (key.includes("valve")) return "valve";
  return "flask";
}

type Tone = "ok" | "warn" | "bad" | "info" | "";
function componentTone(status: string): Tone {
  if (status === "running" || status === "on") return "ok";
  if (status === "error") return "bad";
  if (status === "blocked" || status === "locked" || status === "unavailable") return "warn";
  if (status === "idle") return "info";
  return "";
}

function componentStatusLabel(status: string): string {
  switch (status) {
    case "running":
      return tr("运行中", "Running");
    case "on":
      return tr("开启", "On");
    case "idle":
      return tr("待机", "Idle");
    case "error":
      return tr("故障", "Error");
    case "blocked":
      return tr("阻断", "Blocked");
    case "locked":
      return tr("闭锁", "Locked");
    case "unavailable":
      return tr("不可用", "Unavailable");
    default:
      return status === "--" ? "--" : status;
  }
}

// ---------- 写入表单 ----------

const selectedWriteRegister = computed(
  () => writeRegisters.value.find((r) => r.name === writeForm.register) ?? null
);
const writeUnit = computed(() => selectedWriteRegister.value?.unit ?? "");
const writeValueNumber = computed(() => {
  const parsed = Number(writeForm.value);
  return writeForm.value.trim() !== "" && Number.isFinite(parsed) ? parsed : null;
});
const writeDisabled = computed(
  () =>
    !auth.isAdmin ||
    writing.value ||
    !writeForm.register ||
    writeValueNumber.value === null ||
    !writeForm.reason.trim()
);
const writeValueInput = computed(() => (auth.isAdmin ? "" : tr("写寄存器需要 admin 角色", "Writing requires the admin role")));

// 写入按钮禁用原因提示：禁用时让用户知道缺哪一项。
const writeDisabledReason = computed(() => {
  if (!writeDisabled.value || writing.value) return "";
  if (!auth.isAdmin) return tr("写寄存器需要 admin 角色", "Writing requires the admin role");
  if (!writeForm.register) return tr("请选择要写入的寄存器地址", "Select a writable register address");
  if (writeValueNumber.value === null) return tr("请输入有效数值", "Enter a valid numeric value");
  if (!writeForm.reason.trim()) return tr("请填写写入原因", "Enter the write reason");
  return "";
});

// ---------- 读写动作 ----------

/** 测量一次 API 往返延迟（真实前端实测值）。 */
async function measure<T>(task: () => Promise<T>): Promise<T> {
  const started = performance.now();
  try {
    return await task();
  } finally {
    const ms = performance.now() - started;
    latencyHistory.value = [...latencyHistory.value.slice(-29), ms];
  }
}

function recordOp(record: Omit<OpRecord, "time" | "operator">): void {
  operations.value.unshift({
    ...record,
    time: language.value === "zh" ? new Date().toLocaleTimeString("zh-CN") : new Date().toLocaleTimeString("en-GB"),
    operator: auth.user?.username ?? "--"
  });
  if (operations.value.length > 60) operations.value.length = 60;
}

/** 按起始地址解析寄存器名（read_registers 中地址匹配）。 */
function resolveRegisterByAddress(input: string): ModbusRegisterItem | null {
  const address = Number(input.trim());
  if (!Number.isFinite(address)) return null;
  return readRegisters.value.find((r) => r.address === address) ?? null;
}

async function doRead(): Promise<void> {
  const register = resolveRegisterByAddress(readForm.address);
  if (!register) {
    ElMessage.warning(tr("起始地址未匹配到读寄存器", "No read register at this start address"));
    return;
  }
  reading.value = true;
  try {
    const payload = await measure(() => modbusApi.read(register.name));
    const value = (payload as Record<string, unknown>)?.value ?? null;
    localValues.value = { ...localValues.value, [register.name]: value as number | string | null };
    activeRegister.value = register.name;
    recordOp({
      type: "read",
      slave: readForm.slaveId || "--",
      address: typeof register.address === "number" ? String(register.address) : "--",
      value: formatValue(value),
      result: "success"
    });
  } catch (error) {
    ElMessage.error(errorMessage(error, tr("读取失败", "Read failed")));
    recordOp({
      type: "read",
      slave: readForm.slaveId || "--",
      address: typeof register.address === "number" ? String(register.address) : "--",
      value: "--",
      result: "failed"
    });
  } finally {
    reading.value = false;
  }
}

async function doWrite(): Promise<void> {
  if (writeDisabled.value || writeValueNumber.value === null) return;
  const register = selectedWriteRegister.value;
  if (!register) return;
  const addressText = typeof register.address === "number" ? String(register.address) : "--";
  try {
    await ElMessageBox.confirm(
      tr(
        `将写入 ${register.name} @ ${addressText} = ${writeValueNumber.value}
原因：${writeForm.reason.trim()}`,
        `Write ${register.name} @ ${addressText} = ${writeValueNumber.value}\nReason: ${writeForm.reason.trim()}`
      ),
      tr("确认写入寄存器", "Confirm register write"),
      {
        type: "warning",
        confirmButtonText: tr("确认写入", "Write"),
        cancelButtonText: tr("取消", "Cancel")
      }
    );
  } catch {
    return; // 用户取消
  }
  writing.value = true;
  try {
    await measure(() => modbusApi.write(register.name, { value: writeValueNumber.value as number, reason: writeForm.reason.trim() }));
    lastWrite.value = { time: formatTime(new Date().toISOString()), operator: auth.user?.username ?? "--", ok: true };
    recordOp({
      type: "write",
      slave: writeForm.slaveId || "--",
      address: addressText,
      value: String(writeValueNumber.value),
      result: "success"
    });
    ElMessage.success(tr("写入成功", "Write succeeded"));
    await refreshRegisters();
  } catch (error) {
    lastWrite.value = { time: formatTime(new Date().toISOString()), operator: auth.user?.username ?? "--", ok: false };
    recordOp({
      type: "write",
      slave: writeForm.slaveId || "--",
      address: addressText,
      value: String(writeValueNumber.value),
      result: "failed"
    });
    ElMessage.error(errorMessage(error, tr("写入失败", "Write failed")));
  } finally {
    writing.value = false;
  }
}

async function refreshRegisters(): Promise<void> {
  try {
    await measure(() => plant.loadModbus());
  } catch (error) {
    ElMessage.error(errorMessage(error));
  }
}

function syncFormDefaults(): void {
  if (slaveId.value !== null) {
    readForm.slaveId = String(slaveId.value);
    writeForm.slaveId = String(slaveId.value);
  }
  if (readForm.address === "" && readRegisters.value.length > 0) {
    const first = readRegisters.value[0];
    readForm.address = typeof first.address === "number" ? String(first.address) : "";
  }
  if (writeForm.register === "" && writeRegisters.value.length > 0) {
    writeForm.register = writeRegisters.value[0].name;
  }
}

const loadingData = ref(false);
async function loadAll(): Promise<void> {
  loadingData.value = true;
  try {
    const [modbusResult] = await Promise.allSettled([
      measure(() => plant.loadModbus()),
      plant.loadDeviceStatus(),
      plant.loadAudit()
    ]);
    if (modbusResult.status === "fulfilled") syncFormDefaults();
    else if (modbusResult.reason) ElMessage.error(errorMessage(modbusResult.reason));
  } finally {
    loadingData.value = false;
  }
}

onMounted(loadAll);

const visibleOps = computed(() => (showAllOps.value ? operations.value : operations.value.slice(0, 8)));

// ---------- 通信时间线（静态示意 + 真实统计缺失显示 "--"） ----------

const timelineDots = [
  { tone: "ok" },
  { tone: "info" },
  { tone: "ok" },
  { tone: "info" },
  { tone: "bad" },
  { tone: "info" },
  { tone: "ok" },
  { tone: "info" },
  { tone: "ok" },
  { tone: "info" }
] as const;
</script>

<template>
  <div class="page-stack">
    <!-- 页头 -->
    <header class="page-header">
      <div>
        <h2 class="page-title">
          Modbus Debug<span class="zh">{{ tr("设备通信调试", "Modbus Debug") }}</span>
        </h2>
        <p class="page-subtitle">
          {{ tr("寄存器映射、读写调试与通道健康", "Register map, read/write debug and channel health") }}
        </p>
      </div>
      <div class="header-meta">
        <el-tag v-if="modbus?.mode" size="small" type="info" class="mono">{{ modbus.mode }}</el-tag>
        <el-tag size="small" :type="auth.isAdmin ? 'success' : 'warning'">
          {{ auth.role }}
        </el-tag>
        <el-button size="small" :loading="loadingData" @click="loadAll">
          <AppIcon name="reset" :size="13" style="margin-right: 5px" />{{ tr("刷新", "Refresh") }}
        </el-button>
      </div>
    </header>

    <!-- 0) 顶部 5 张状态卡 -->
    <div class="status-cards">
      <div class="panel stat-card">
        <div class="stat-head">
          <span class="stat-title">Modbus TCP<span class="zh">{{ tr("以太网", "Ethernet") }}</span></span>
          <span class="status-dot" :class="tcpConnected ? 'ok' : 'bad'"></span>
        </div>
        <div class="stat-value mono" :title="tcpBind">{{ tcpConnected ? tr("已连接", "Connected") : tr("未连接", "Disconnected") }}</div>
        <div class="stat-sub mono">{{ tcpBind }}</div>
        <div class="stat-footer">
          <SparkLine v-if="latencyHistory.length > 1" :points="latencyHistory" :height="20" color="#38c8f2" />
          <span class="mono stat-metric">
            {{ latestLatency === null ? "--" : `${latestLatency} ms` }} {{ tr("延迟", "latency") }}
          </span>
        </div>
      </div>

      <div class="panel stat-card">
        <div class="stat-head">
          <span class="stat-title">Modbus RTU<span class="zh">{{ tr("串口", "Serial") }}</span></span>
          <span class="status-dot" :class="rtuActive ? 'ok' : ''"></span>
        </div>
        <div class="stat-value">{{ rtuActive ? tr("已连接", "Connected") : tr("未启用", "Inactive") }}</div>
        <div class="stat-sub mono" :title="serialLabel">{{ serialLabel }}</div>
        <div class="stat-footer">
          <span class="stat-metric">{{ tr("串口配置", "Port config") }}</span>
        </div>
      </div>

      <div class="panel stat-card">
        <div class="stat-head">
          <span class="stat-title">Polling Interval<span class="zh">{{ tr("轮询间隔", "Polling") }}</span></span>
        </div>
        <div class="stat-value mono">--<span class="stat-unit">ms</span></div>
        <div class="stat-sub">&nbsp;</div>
        <div class="stat-footer">
          <span class="stat-metric">{{ tr("最近轮询", "Last poll") }}</span>
          <span class="mono stat-metric">{{ lastPoll }}</span>
        </div>
      </div>

      <div class="panel stat-card">
        <div class="stat-head">
          <span class="stat-title">Slave ID<span class="zh">{{ tr("从站地址", "Slave") }}</span></span>
        </div>
        <div class="stat-value mono">{{ slaveLabel }}</div>
        <div class="stat-sub">&nbsp;</div>
        <div class="stat-footer">
          <span class="stat-metric">{{ tr("广播 禁用", "Broadcast off") }}</span>
        </div>
      </div>

      <div class="panel stat-card">
        <div class="stat-head">
          <span class="stat-title">Device State<span class="zh">{{ tr("设备在线状态", "Device") }}</span></span>
        </div>
        <div class="stat-value" :class="deviceOnline ? 'tone-ok' : 'tone-bad'">
          {{ deviceOnline ? tr("在线", "Online") : tr("离线", "Offline") }}
        </div>
        <div class="stat-sub">&nbsp;</div>
        <div class="stat-footer">
          <span class="stat-metric">{{ tr("运行时间", "Uptime") }}</span>
          <span class="mono stat-metric">--</span>
        </div>
      </div>
    </div>

    <!-- 主体三列 -->
    <div class="main-grid">
      <!-- 1) 左列：读取寄存器 + 寄存器映射 -->
      <div class="col-left">
        <PanelCard en="Read Registers" :zh="tr('读取寄存器', 'Read Registers')" icon="modbus" class="read-panel">
          <template #actions>
            <span class="data-label mono">{{ tr("共", "") }}{{ readRegisters.length }}{{ tr("个读寄存器", "read registers") }}</span>
          </template>
          <div class="read-form">
            <label class="field">
              <span class="field-label">Slave ID</span>
              <el-select v-model="readForm.slaveId" size="small" filterable>
                <el-option :value="slaveLabel" :label="slaveLabel" />
              </el-select>
            </label>
            <label class="field">
              <span class="field-label">{{ tr("功能码", "Function Code") }}</span>
              <el-select v-model="readForm.function" size="small">
                <el-option v-for="fn in READ_FUNCTIONS" :key="fn.value" :value="fn.value" :label="fn.label" />
              </el-select>
            </label>
            <label class="field">
              <span class="field-label">{{ tr("起始地址", "Start Address") }}</span>
              <el-input v-model="readForm.address" size="small" class="mono" placeholder="0" />
            </label>
            <label class="field">
              <span class="field-label">{{ tr("数量", "Quantity") }}</span>
              <el-input v-model="readForm.quantity" size="small" class="mono" placeholder="10" />
            </label>
            <el-button type="primary" size="small" class="read-btn" :loading="reading" @click="doRead">
              {{ tr("读取", "Read") }}
            </el-button>
          </div>

          <div class="section-label">
            <span>{{ tr("值", "Values") }}</span>
            <span v-if="activeRegister" class="mono active-name">{{ activeRegister }}</span>
          </div>
          <div v-if="valueCards.length > 0" class="value-grid">
            <div
              v-for="card in valueCards"
              :key="card.name"
              class="value-chip"
              :class="{ active: card.name === activeRegister }"
            >
              <span class="chip-addr mono">{{ card.address === null ? "--" : card.address }}</span>
              <span class="chip-value mono">{{ formatValue(card.value) }}</span>
            </div>
          </div>
          <div v-else class="empty-state">
            <AppIcon name="modbus" :size="30" />
            <span>{{ tr("暂无读寄存器数据", "No read registers") }}</span>
          </div>

          <div class="section-label trend-head">
            <span>{{ tr("趋势", "Trend") }}</span>
            <span class="mono data-value">{{ fixed(currentTemp, 1, "°C") }}</span>
          </div>
          <div class="trend-box">
            <TrendChart v-if="tempSeries[0].data.length > 0" :series="tempSeries" :legend="false" height="100%" />
            <div v-else class="empty-state small-empty">
              <span>{{ tr("暂无实时样本", "No live samples") }}</span>
            </div>
          </div>
        </PanelCard>

        <PanelCard en="Register Map" :zh="tr('寄存器映射', 'Register Map')" icon="gauge" flush scrollable class="map-panel">
          <template #actions>
            <span class="data-label mono">{{ filteredMapRows.length }} / {{ mapRows.length }}</span>
          </template>
          <div class="map-filters">
            <el-input
              v-model="filters.search"
              size="small"
              clearable
              :placeholder="tr('搜索名称 / 标签', 'Filter name / label')"
              class="filter-search"
            >
              <template #prefix><AppIcon name="search" :size="12" /></template>
            </el-input>
            <el-select v-model="filters.type" size="small" clearable :placeholder="tr('全部类型', 'All Types')" class="filter-select">
              <el-option v-for="opt in accessOptions" :key="opt.value" :value="opt.value" :label="opt.label" />
            </el-select>
            <el-select v-model="filters.status" size="small" clearable :placeholder="tr('全部状态', 'All Status')" class="filter-select">
              <el-option value="good" :label="tr('有数据', 'Good')" />
              <el-option value="nodata" :label="tr('无数据', 'No Data')" />
            </el-select>
          </div>
          <el-table v-if="filteredMapRows.length > 0" :data="filteredMapRows" size="small" class="table-fill">
            <el-table-column :label="tr('地址', 'Address')" width="70">
              <template #default="{ row }"><span class="mono">{{ row.address === null ? "--" : row.address }}</span></template>
            </el-table-column>
            <el-table-column :label="tr('标签', 'Label')" min-width="190" show-overflow-tooltip>
              <template #default="{ row }">
                <div class="cell-label">{{ row.label }}</div>
                <div class="cell-sub mono">{{ row.name }}</div>
              </template>
            </el-table-column>
            <el-table-column :label="tr('类型', 'Type')" width="80">
              <template #default="{ row }">
                <span class="access-tag mono" :class="row.access">{{ accessLabel(row.access) }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="tr('当前值', 'Current Value')" width="110" align="right">
              <template #default="{ row }"><span class="mono">{{ formatValue(row.value) }}</span></template>
            </el-table-column>
            <el-table-column :label="tr('单位', 'Unit')" width="70">
              <template #default="{ row }"><span class="cell-sub">{{ row.unit || "--" }}</span></template>
            </el-table-column>
            <el-table-column :label="tr('状态', 'Status')" width="110">
              <template #default="{ row }">
                <span v-if="row.hasValue" class="status-inline ok">
                  <span class="status-dot ok"></span>{{ tr("良好", "Good") }}
                </span>
                <span v-else class="status-inline">
                  <span class="status-dot"></span>{{ tr("无数据", "No Data") }}
                </span>
              </template>
            </el-table-column>
          </el-table>
          <div v-else class="empty-state">
            <AppIcon name="search" :size="28" />
            <span>{{ tr("没有匹配的寄存器", "No matching registers") }}</span>
          </div>
        </PanelCard>
      </div>

      <!-- 2) 中列：组件健康 -->
      <PanelCard en="Component Health" :zh="tr('组件健康状态', 'Component Health')" icon="motor" scrollable class="col-mid">
        <template #actions>
          <span class="data-label mono">{{ componentRows.length }}</span>
        </template>
        <div v-if="componentRows.length > 0" class="component-list">
          <div v-for="row in componentRows" :key="row.id" class="component-item">
            <span class="component-icon" :class="componentTone(row.status)">
              <AppIcon :name="componentIcon(row)" :size="16" />
            </span>
            <div class="component-info">
              <div class="component-name mono">{{ row.id }}</div>
              <div class="cell-sub">{{ row.label || "--" }}</div>
              <div class="component-meta">
                <span class="cell-sub">{{ tr("类型", "Type") }}: <span class="mono">{{ row.type }}</span></span>
                <span class="cell-sub">{{ tr("地址", "Addr") }}: <span class="mono">--</span></span>
              </div>
              <div class="component-meta">
                <span class="status-inline" :class="componentTone(row.status)">
                  <span class="status-dot" :class="componentTone(row.status)"></span>{{ componentStatusLabel(row.status) }}
                </span>
                <span class="cell-sub">{{ tr("更新", "Update") }}: <span class="mono">--</span></span>
              </div>
            </div>
          </div>
        </div>
        <div v-else class="empty-state">
          <AppIcon name="motor" :size="30" />
          <span>{{ tr("当前设备模式无组件", "No components in this device mode") }}</span>
        </div>
      </PanelCard>

      <!-- 3) 右列：写入 + 最近操作 + 时间线 -->
      <div class="col-right">
        <PanelCard en="Write Register" :zh="tr('写入寄存器', 'Write Register')" icon="valve" class="write-panel">
          <div class="write-form">
            <label class="field">
              <span class="field-label">Slave ID</span>
              <el-select v-model="writeForm.slaveId" size="small" filterable>
                <el-option :value="slaveLabel" :label="slaveLabel" />
              </el-select>
            </label>
            <label class="field">
              <span class="field-label">{{ tr("功能码", "Function Code") }}</span>
              <el-select v-model="writeForm.function" size="small">
                <el-option v-for="fn in WRITE_FUNCTIONS" :key="fn.value" :value="fn.value" :label="fn.label" />
              </el-select>
            </label>
            <label class="field span-2">
              <span class="field-label">{{ tr("地址", "Address") }}</span>
              <el-select v-model="writeForm.register" size="small" filterable :placeholder="tr('选择可写寄存器', 'Writable register')">
                <el-option
                  v-for="reg in writeRegisters"
                  :key="reg.name"
                  :value="reg.name"
                  :label="`${reg.name}${typeof reg.address === 'number' ? ' @ ' + reg.address : ''}`"
                />
              </el-select>
            </label>
            <label class="field">
              <span class="field-label">{{ tr("值", "Value") }}</span>
              <el-input v-model="writeForm.value" size="small" class="mono" placeholder="0" />
            </label>
            <label class="field">
              <span class="field-label">{{ tr("单位", "Unit") }}</span>
              <el-input :model-value="writeUnit || '--'" size="small" disabled />
            </label>
            <label class="field span-2">
              <span class="field-label">{{ tr("权限验证", "Permission") }}</span>
              <el-select :model-value="auth.role" size="small" disabled>
                <el-option :value="auth.role" :label="auth.role" />
              </el-select>
            </label>
            <label class="field span-2">
              <span class="field-label">{{ tr("原因", "Reason") }}</span>
              <el-input
                v-model="writeForm.reason"
                size="small"
                maxlength="240"
                :placeholder="writeValueInput || tr('写入原因（必填）', 'Reason (required)')"
              />
            </label>
            <el-button type="primary" size="small" class="span-2 write-btn" :disabled="writeDisabled" :loading="writing" @click="doWrite">
              {{ tr("写入", "Write") }}
            </el-button>
            <div v-if="writeDisabledReason" class="span-2 write-hint">{{ writeDisabledReason }}</div>
          </div>
          <div v-if="lastWrite" class="last-write">
            <span class="status-dot" :class="lastWrite.ok ? 'ok' : 'bad'"></span>
            <span class="cell-sub">{{ tr("最近写入", "Last write") }}</span>
            <span class="mono cell-sub">{{ lastWrite.time }}</span>
            <span class="cell-sub">{{ tr("操作员", "Operator") }}</span>
            <span class="mono cell-sub">{{ lastWrite.operator }}</span>
          </div>
        </PanelCard>

        <PanelCard en="Recent Operations" :zh="tr('最近读写操作', 'Recent Operations')" icon="clock" flush scrollable class="ops-panel">
          <template #actions>
            <el-button
              v-if="operations.length > 8"
              link
              size="small"
              type="primary"
              @click="showAllOps = !showAllOps"
            >
              {{ showAllOps ? tr("收起", "Collapse") : tr("查看全部", "View All") }}
            </el-button>
          </template>
          <el-table v-if="visibleOps.length > 0" :data="visibleOps" size="small" class="table-fill">
            <el-table-column label="{{ tr('时间', 'Time') }}" width="76">
              <template #default="{ row }"><span class="mono cell-sub">{{ row.time }}</span></template>
            </el-table-column>
            <el-table-column label="{{ tr('类型', 'Type') }}" width="64">
              <template #default="{ row }">
                <span :class="row.type === 'write' ? 'op-write' : 'op-read'">
                  {{ row.type === "write" ? tr("写", "Write") : tr("读", "Read") }}
                </span>
              </template>
            </el-table-column>
            <el-table-column prop="slave" label="Slave" width="56">
              <template #default="{ row }"><span class="mono cell-sub">{{ row.slave }}</span></template>
            </el-table-column>
            <el-table-column label="{{ tr('地址', 'Addr') }}" width="52">
              <template #default="{ row }"><span class="mono cell-sub">{{ row.address }}</span></template>
            </el-table-column>
            <el-table-column label="{{ tr('值', 'Value') }}" width="70" align="right">
              <template #default="{ row }"><span class="mono cell-sub">{{ row.value }}</span></template>
            </el-table-column>
            <el-table-column label="{{ tr('结果', 'Result') }}" width="72">
              <template #default="{ row }">
                <span v-if="row.result === 'success'" class="status-inline ok">
                  <span class="status-dot ok"></span>{{ tr("成功", "Success") }}
                </span>
                <span v-else class="status-inline bad">
                  <span class="status-dot bad"></span>{{ tr("失败", "Failed") }}
                </span>
              </template>
            </el-table-column>
            <el-table-column :label="tr('操作员', 'Operator')" min-width="80" show-overflow-tooltip>
              <template #default="{ row }"><span class="cell-sub">{{ row.operator }}</span></template>
            </el-table-column>
          </el-table>
          <div v-else class="empty-state">
            <AppIcon name="clock" :size="28" />
            <span>{{ tr("暂无读写操作", "No operations yet") }}</span>
          </div>
        </PanelCard>

        <PanelCard en="Communications Timeline" :zh="tr('通信时间线', 'Comms Timeline')" icon="live" class="timeline-panel">
          <div class="timeline-strip">
            <span v-for="(dot, i) in timelineDots" :key="i" class="tl-dot" :class="dot.tone"></span>
          </div>
          <div class="timeline-legend">
            <span class="legend-item"><span class="status-dot ok"></span>{{ tr("请求", "Request") }}</span>
            <span class="legend-item"><span class="status-dot info"></span>{{ tr("响应", "Response") }}</span>
            <span class="legend-item"><span class="status-dot bad"></span>{{ tr("错误", "Error") }}</span>
          </div>
          <div class="timeline-stats">
            <div class="tl-stat">
              <span class="tl-num mono">--</span>
              <span class="tl-label">{{ tr("总报文", "Total Packets") }}</span>
            </div>
            <div class="tl-stat">
              <span class="tl-num mono">--</span>
              <span class="tl-label">{{ tr("成功率", "Successful") }}</span>
            </div>
            <div class="tl-stat">
              <span class="tl-num mono">--</span>
              <span class="tl-label">{{ tr("错误", "Errors") }}</span>
            </div>
            <div class="tl-stat">
              <span class="tl-num mono">--</span>
              <span class="tl-label">{{ tr("超时", "Timeouts") }}</span>
            </div>
          </div>
        </PanelCard>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ===== 页头 ===== */
.header-meta {
  display: flex;
  align-items: center;
  gap: 8px;
}

/* ===== 顶部状态卡 ===== */
.status-cards {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: var(--spacing);
  flex: none;
  min-height: 0;
}
.stat-card {
  padding: 0;
}
.stat-card :deep(.panel-body) {
  padding: 10px 12px;
  gap: 4px;
}
.stat-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}
.stat-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.stat-title .zh {
  margin-left: 6px;
  font-weight: 400;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.stat-value {
  font-family: var(--font-data);
  font-variant-numeric: tabular-nums;
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.stat-value.tone-ok { color: var(--ind-green); }
.stat-value.tone-bad { color: var(--ind-red); }
.stat-unit {
  margin-left: 4px;
  font-size: var(--fs-sm);
  font-weight: 400;
  color: var(--text-tertiary);
}
.stat-sub {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-height: 14px;
}
.stat-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 2px;
  min-height: 20px;
}
.stat-footer .sparkline {
  flex: 1;
  min-width: 0;
}
.stat-metric {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

/* ===== 主体三列 ===== */
.main-grid {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 55fr) minmax(0, 17fr) minmax(0, 28fr);
  gap: var(--spacing);
}
.col-left {
  display: grid;
  grid-template-rows: minmax(0, 46fr) minmax(0, 54fr);
  gap: var(--spacing);
  min-height: 0;
  min-width: 0;
}
.col-right {
  display: grid;
  grid-template-rows: minmax(0, auto) minmax(0, 1fr) auto;
  gap: var(--spacing);
  min-height: 0;
  min-width: 0;
}

/* ===== 表单字段 ===== */
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}
.field-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.field.span-2 {
  grid-column: span 2;
}
.read-form {
  display: grid;
  grid-template-columns: minmax(0, 0.8fr) minmax(0, 1.3fr) minmax(0, 1fr) minmax(0, 0.8fr) auto;
  gap: 10px;
  align-items: end;
  flex: none;
}
.read-btn {
  height: 24px;
}
.write-form {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}
.write-btn {
  margin-top: 2px;
}
.write-hint {
  font-size: 11px;
  color: var(--ind-amber);
  line-height: 1.3;
}

/* ===== Read 面板 ===== */
.read-panel :deep(.panel-body) {
  gap: 8px;
}
.section-label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  font-weight: 600;
  flex: none;
}
.active-name {
  color: var(--accent-strong);
}
.value-grid {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 8px;
  flex: none;
}
.value-chip {
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 6px 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.value-chip.active {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px rgba(47, 155, 255, 0.35);
}
.chip-addr {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.chip-value {
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.trend-head .data-value {
  color: var(--accent-strong);
  font-size: var(--fs-sm);
}
.trend-box {
  flex: 1;
  min-height: 70px;
}
.small-empty {
  padding: 10px;
  font-size: var(--fs-sm);
}

/* ===== Register Map ===== */
.map-panel {
  min-height: 0;
}
.map-filters {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 108px 108px;
  gap: 8px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-glass);
  flex: none;
}
.table-fill {
  flex: 1;
  min-height: 0;
}
.table-fill :deep(.el-table__inner-wrapper) {
  height: 100%;
}
.cell-label {
  color: var(--text-primary);
  font-size: var(--fs-sm);
}
.cell-sub {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.access-tag {
  display: inline-block;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-xs);
  border: 1px solid var(--border-glass);
  color: var(--text-secondary);
  background: var(--bg-inset);
}
.access-tag.read { color: var(--ind-green); border-color: rgba(47, 212, 123, 0.4); }
.access-tag.write { color: var(--ind-amber); border-color: rgba(245, 166, 35, 0.4); }
.access-tag.read_write { color: var(--accent-strong); border-color: rgba(47, 155, 255, 0.4); }
.status-inline {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-secondary);
}
.status-inline.ok { color: var(--ind-green); }
.status-inline.bad { color: var(--ind-red); }
.status-inline.warn { color: var(--ind-amber); }

/* ===== 组件健康 ===== */
.col-mid {
  min-height: 0;
}
.component-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.component-item {
  display: flex;
  gap: 10px;
  padding: 8px 10px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
}
.component-icon {
  width: 30px;
  height: 30px;
  border-radius: var(--radius-sm);
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
  background: rgba(47, 155, 255, 0.1);
  color: var(--accent);
}
.component-icon.ok { color: var(--ind-green); background: rgba(47, 212, 123, 0.12); }
.component-icon.warn { color: var(--ind-amber); background: rgba(245, 166, 35, 0.12); }
.component-icon.bad { color: var(--ind-red); background: rgba(255, 82, 82, 0.12); }
.component-icon.info { color: var(--accent-cyan); background: rgba(56, 200, 242, 0.12); }
.component-info {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.component-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  overflow-wrap: anywhere;
}
.component-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

/* ===== 写入面板 ===== */
.write-panel :deep(.panel-body) {
  gap: 10px;
}
.last-write {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 8px 10px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
}

/* ===== 最近操作 ===== */
.ops-panel {
  min-height: 0;
}
.op-read { color: var(--accent-cyan); font-size: var(--fs-xs); font-weight: 600; }
.op-write { color: var(--ind-amber); font-size: var(--fs-xs); font-weight: 600; }
.ops-panel .map-filters {
  display: none;
}

/* ===== 时间线 ===== */
.timeline-strip {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 6px 10px;
  position: relative;
  flex: none;
}
.timeline-strip::before {
  content: "";
  position: absolute;
  left: 8px;
  right: 8px;
  top: 50%;
  height: 1px;
  background: var(--border-glass);
}
.tl-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--ind-gray);
  position: relative;
  z-index: 1;
}
.tl-dot.ok { background: var(--ind-green); box-shadow: 0 0 6px rgba(47, 212, 123, 0.5); }
.tl-dot.info { background: var(--accent); box-shadow: 0 0 6px rgba(47, 155, 255, 0.5); }
.tl-dot.bad { background: var(--ind-red); box-shadow: 0 0 6px rgba(255, 82, 82, 0.5); }
.timeline-legend {
  display: flex;
  justify-content: center;
  gap: 14px;
  flex: none;
}
.legend-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.timeline-stats {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
  margin-top: 10px;
  flex: none;
}
.tl-stat {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 8px 4px;
}
.tl-num {
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-primary);
}
.tl-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  text-align: center;
}

/* ===== 窄屏降级：三列改两列 ===== */
@media (max-width: 1380px) {
  .main-grid {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    grid-template-rows: minmax(0, auto) minmax(0, 1fr);
  }
  .col-left {
    grid-column: 1 / -1;
    grid-template-rows: minmax(0, 260px) minmax(0, 1fr);
  }
  .col-mid {
    grid-row: 2;
  }
  .col-right {
    grid-row: 2;
  }
  .value-grid {
    grid-template-columns: repeat(5, minmax(0, 1fr));
  }
}
@media (max-width: 1000px) {
  .status-cards {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

/* V32：移动端单列堆叠、整页可滚动 */
@media (max-width: 900px) {
  .main-grid { display: flex; flex-direction: column; }
  .main-grid > * { flex: none; }
  .col-left { display: flex; flex-direction: column; }
  .status-cards { grid-template-columns: 1fr 1fr; }
}
</style>
