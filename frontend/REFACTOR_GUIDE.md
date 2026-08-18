# ReactorOS 前端重构指引（子代理必读）

> 工作区：X:\tianhks（Windows，路径用反斜杠）。分支 refactor/frontend-rebuild。
> 目标：按深海军蓝工业风参考稿重做 frontend/src/views 下全部页面。地基已完成，你只负责指定页面文件。

## 设计系统（已实现，直接使用）

### CSS 变量（src/design/tokens.css）
- 背景：--bg-app #08111f；面板 --bg-panel（半透明 #101f33）；--bg-panel-solid；--bg-inset（更深的内嵌底）；--bg-hover
- 边框：--border-glass（淡蓝透明）；--border-strong
- 强调：--accent #2f9bff；--accent-strong #57b4ff；--accent-dim；--accent-cyan #38c8f2
- 语义：--ind-green #2fd47b；--ind-red #ff5252；--ind-amber #f5a623；--ind-purple #b068f0；--ind-gray #5a7396
- 文本：--text-primary #e8f1fb；--text-secondary #9db4cf；--text-tertiary #5f7a9c
- 字号：--fs-xs 11 / --fs-sm 12 / --fs-base 13 / --fs-md 14 / --fs-lg 16 / --fs-xl 20 / --fs-2xl 28 / --fs-3xl 36
- 圆角：--radius-sm 6 / --radius-md 8 / --radius-lg 12；间距 --spacing 14px
- 字体：--font-ui；--font-data（等宽，数值一律用它）

### 全局类（src/design/base.css）
- .panel / .panel-header / .panel-title（内含 .zh 副标题）/ .panel-body（.flush 无内边距 / .scrollable 可滚）
- .page-stack（页面纵向堆叠，height:100%）/ .page-header / .page-title（含 .zh）/ .page-subtitle
- .grid .cols-2/3/4/5；.kv-list（dt/dd 键值表）；.data-value / .data-label / .mono
- .status-dot .ok/.warn/.bad/.info；.empty-state；.form-grid / .form-actions
- **严禁页面级纵向滚动条**：页面根用 .page-stack 或 grid 撑满 100% 高，内部区域各自 min-height:0 + 必要处 overflow:hidden/auto
- Element Plus 组件已全局按需自动注册（el-table/el-select/el-button/el-tag/el-pagination/el-switch/el-slider/el-input-number/el-date-picker/ElMessage/ElMessageBox 等直接用，无需 import 组件，ElMessage/ElMessageBox 需 import from "element-plus"）

### 共享组件（src/components/，需显式 import）
- **PanelCard.vue**：`<PanelCard en="Live Trend" zh="实时趋势">`；props: en, zh, icon?, flush?, scrollable?；插槽：默认 + #actions（标题右侧按钮区）
- **SparkLine.vue**：props: points: number[], color?, height?（默认30）。无轴迷你面积线
- **TrendChart.vue**：props: series: TrendSeries[]（{name, data:[tsMs, value|null][], color?, unit?, yAxisIndex?, smooth?, dashed?}），legend?, height?, markTime?（画竖虚线）。暗色坐标轴/图例/tooltip 已内置
- **EmergencyStopPanel.vue**：完整急停面板（按住1.5s触发，急停态点击复位确认）。直接 `<EmergencyStopPanel />` 使用，无 props
- **AppIcon.vue**：`<AppIcon name="monitor|control|ai|history|audit|modbus|settings|live|alarm|shield|batch|operator|clock|export|report|search|reset|play|pause|stop|check|flask|valve|heater|motor|gauge" :size="16" />`
- 旧组件 AlarmList.vue / EmptyState.vue / HmiButton.vue / PageHeader.vue 仍存在，可不用

### i18n 规则（强制）
`import { useLanguage } from "../i18n"` → `const { tr, language } = useLanguage()`；模板里所有用户可见文案必须 `tr("中文", "English")`。参考稿风格是 EN 主标签 + 中文副标签并排/上下（如 "Live Trend 实时趋势"），建议标题用 PanelCard 的 en+zh 双 props。时间格式化按 language.value === "zh" 分支。

## 数据层（已实现，直接调用）

### stores/live.ts（useLiveStore）— 实时数据
- live: LiveResponse|null；runtime（含 targets/latest_sample/active_batch_id/auto_enabled/manual_lock/emergency_stop/last_sensor_error 等）
- latestSample: SensorSample|null（temperature_c/pressure_mpa/stirrer_rpm/shake_speed_cpm/flow_rate_l_min/product_concentration_percent/ph/captured_at）
- recentSamples: SensorSample[]（滚动窗口，画趋势图）
- alarms: Alarm[]（level/severity/message/suggestion/current_value/limit_value）
- recommendation: AiRecommendationEnvelope|null（target_temperature_c/target_stirrer_rpm/expected_score/rationale/provider）
- primaryDevice: DeviceStatusItem|null（online/status/components[]）
- liveStatus: "fresh"|"unavailable"；refreshLive() 手动刷新
- **注意**：/api/live 无数据时 503 → liveStatus="unavailable"，页面要显示空值 "--" 而不是报错
- 注意单位：pressure_mpa（MPa，参考稿用 bar，1 MPa = 10 bar，显示时乘 10 并标注 bar）

### stores/plant.ts（usePlantStore）— 业务数据
- config: ConfigSummary|null（safety.temperature/stirrer 上下限、integrations、permissions、ai_provider、local_ai、device_mode 等）
- processes / selectedProcess(ProcessDetail 含 steps: ProcessStep[]，step_index/name/target_temperature_c/duration_minutes/target_stirrer_rpm/target_shake_speed_cpm/target_pressure_mpa)
- batches: BatchListResponse|null（batches[] + outcomes[] 含 yield_percent/product_ratio/notes）
- audit: AuditLogsResponse|null（events: ControlEvent[] + chain: AuditChainStatus）
- modbus: ModbusRegistersResponse|null（read_registers/write_registers/coils/discrete_inputs: ModbusRegisterItem[] {name,label,address,access,value,raw,unit}；tcp{listening,bind,tls}；serial；mode/slave_id）
- deviceStatus / deviceCapabilities / permissionRoles / ainasTasks / demoContext / recommendation
- 加载方法：loadConfig/loadProcesses/loadProcessDetail(id)/loadBatches/loadAudit({page,pageSize,eventType})/loadModbus/loadDeviceStatus/loadDeviceCapabilities/loadPermissionRoles/loadAinasTasks/loadDemoContext

### stores/auth.ts（useAuthStore）
- user{username,role,permissions}/token/isAuthenticated/role/isEngineerOrAdmin/isAdmin/hasPermission(p)/login/logout

### api/index.ts — 写操作
- controlApi: updateTargets({temperature_c,stirrer_rpm,shake_speed_cpm?})/setAuto(bool)/setManualLock(bool)/resetFault()/emergencyStop()/resetEmergencyStop()
- processApi: list()/detail(id)/create/update/addStep/updateStep/apply(id)/start(id)/stop(id,reason?)/stopCurrent(reason?)
- batchApi: list()/detail(id)→BatchDetail{batch,outcome,samples,events}/start({name,process_id,...})/finish(id)/saveProductResult({batch_id,yield_percent,product_ratio,notes})/exportCsv()/exportXlsx()/exportReport(id)
- auditApi: logs({page,pageSize,eventType})/exportCsv(eventType)
- aiApi: latestRecommendation()/regenerateRecommendation()(POST 触发云端)/control({intent?,dry_run,...})→AiControlResponse{decision,rationale,recommended_targets,actions[]}/experimentPlan()→ExperimentPlanResponse{plan_id,title,status,steps[{step_no,name,target_temperature_c,duration_minutes,operator_action,safety_check}],sop_summary,acceptance_criteria,safety_notes,next_actions}
- modbusApi: registers()/read(name)/write(name,{value,reason})
- deviceApi: controlComponent(deviceId, componentId, {action, value?, reason?})
- realtimeApi: history(deviceId,{startTime,endTime,page,pageSize})→HistoryResponse{items/records: HistoryRecord[]{timestamp,data{current_temp,current_pressure,stir_speed,shake_speed,flow_rate,product_concentration,ph}}}
- ainasApi: list/detail/create
- downloadBlob(blob, filename) from api/http 用于导出
- 写操作需要登录（auth store）；未登录时调用会 401，用 ElMessage 提示

### utils/format.ts
fixed(v,digits,suffix) → "--"|"25.3"；text(v,fallback)；formatTimestamp(v)→"YYYY-MM-DD HH:mm:ss"；formatTime(v)→"HH:mm:ss"；boolText(v)

## 参考稿视觉语言（所有页面统一）
- 页面顶部：页面标题行（EN 大标题 + 中文副标题，可用 .page-title/.page-subtitle 或 PanelCard 风格）
- 面板 = 深蓝玻璃卡片，面板标题 "EN 中文" 双语并排
- 数值大、等宽字体、带单位小字；状态用彩色点/胶囊/tag
- 表格行高紧凑、表头小字双语（"Batch ID / 批次号" 这种上下两行表头可用两个 span 堆叠）
- 图表主色：蓝 #2f9bff / 绿 #2fd47b / 橙 #f5a623 / 红 #ff5252 / 紫 #b068f0 / 青 #38c8f2
- 数据缺失显示 "--"，整块无数据用 .empty-state 或提示行，不要留空白框

## 构建验证
改完页面后你可以自行验证语法（可选）：在 X:\tianhks 运行 `npm run frontend:build`（需 node_modules 已装，已装）。
最终由主控统一构建。你只写自己负责的 view 文件，不要改共享文件（tokens/base/App/组件/stores/api/router）。
