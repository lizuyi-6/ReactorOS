// 后端 DTO 类型定义（依据 src/api.rs / api_auth.rs / db.rs 手写，与后端真实结构对齐）。
// 原则：信封 {code,message,data} 由 http 层统一拆开，这里只描述 data 载荷。

export type Role = "operator" | "engineer" | "admin" | "guest";

export interface AuthUser {
  username: string;
  role: Role;
  permissions: string[];
}

export interface LoginResponse {
  token: string;
  user: AuthUser;
  expires_at: string;
}

export interface PermissionRoleItem {
  role: string;
  label: string;
  can: string[];
  blocked: string[];
}

export interface PermissionRolesResponse {
  mode: string;
  authentication: string;
  session_ttl_hours: number;
  default_users: string[];
  roles: PermissionRoleItem[];
}

export interface HealthResponse {
  ok: boolean;
  service: string;
}

export interface SensorSample {
  id?: number;
  batch_id?: number | null;
  temperature_c?: number | null;
  pressure_mpa?: number | null;
  stirrer_rpm?: number | null;
  shake_speed_cpm?: number | null;
  tilt_state?: number | null;
  tilt_angle_deg?: number | null;
  flow_rate_l_min?: number | null;
  product_concentration_percent?: number | null;
  ph?: number | null;
  captured_at?: string;
  created_at?: string;
}

export interface ControlTargets {
  temperature_c?: number | null;
  heat_time_s?: number | null;
  hold_time_s?: number | null;
  cool_time_s?: number | null;
  stirrer_rpm?: number | null;
  shake_speed_cpm?: number | null;
  target_pressure_mpa?: number | null;
  cooling_mode?: string | null;
}

export interface RuntimeState {
  targets?: ControlTargets;
  latest_sample?: SensorSample | null;
  active_batch_id?: number | null;
  active_process_id?: number | null;
  active_process_name?: string | null;
  auto_enabled?: boolean;
  manual_lock?: boolean;
  emergency_stop?: boolean;
  control_loop_terminated?: boolean;
  last_sensor_error?: string | null;
  last_control_error?: string | null;
  [key: string]: unknown;
}

export interface DeviceStatusItem {
  device_id: string;
  device_role?: string;
  online?: boolean;
  status?: "idle" | "running" | "stale" | "offline" | "error" | string;
  auto_enabled?: boolean;
  manual_lock?: boolean;
  last_seen_at?: string | null;
  last_seen_age_ms?: number | null;
  stale_after_ms?: number | null;
  active_batch_id?: number | null;
  emergency_stop?: boolean;
  last_sensor_error?: string | null;
  last_control_error?: string | null;
  unfinished_batch_ids?: number[];
  unexpected_unfinished_batch_ids?: number[];
  last_command_request_id?: string | null;
  last_command_ok?: boolean | null;
  last_command_error?: string | null;
  sensors?: unknown[];
  components?: DeviceComponentItem[];
  [key: string]: unknown;
}

export interface DeviceComponentItem {
  component_id?: string;
  id?: string;
  label?: string;
  state?: string;
  actions?: ComponentAction[];
  [key: string]: unknown;
}

export interface ComponentAction {
  action: string;
  label: string;
  value_type: "none" | "number" | string;
  min?: number;
  max?: number;
  unit?: string;
}

export interface DeviceStatusSummary {
  total_count: number;
  online_count: number;
  devices: DeviceStatusItem[];
  sensors?: unknown[];
  components?: DeviceComponentItem[];
}

export interface DeviceCapabilitiesDevice {
  device_id: string;
  device_role?: string;
  mode?: string;
  online?: boolean;
  status?: string;
  sensors?: unknown[];
  components?: DeviceComponentItem[];
}

export interface DeviceCapabilitiesResponse {
  total_count: number;
  online_count: number;
  devices: DeviceCapabilitiesDevice[];
}

export interface Alarm {
  type?: string;
  code?: string;
  level?: string;
  severity?: string;
  message?: string;
  suggestion?: string;
  current_value?: number | string;
  limit_value?: number | string;
  [key: string]: unknown;
}

export interface AiProviderInfo {
  mode?: string;
  model?: string;
  fallback_reason?: string | null;
  [key: string]: unknown;
}

export interface AiRecommendationEnvelope {
  based_on_batch_count?: number;
  target_temperature_c?: number | null;
  target_stirrer_rpm?: number | null;
  heating_minutes?: number | null;
  stirring_minutes?: number | null;
  expected_score?: number | null;
  rationale?: string;
  provider?: AiProviderInfo | string | null;
  [key: string]: unknown;
}

export interface ProcessDefinition {
  id: number;
  name?: string | null;
  description?: string | null;
  status?: "draft" | "applied" | "archived" | string;
  version?: number;
  step_count?: number;
  created_at?: string;
  updated_at?: string;
  applied_at?: string | null;
}

export interface ProcessStep {
  id: number;
  process_id: number;
  step_index: number;
  name?: string | null;
  target_temperature_c?: number | null;
  ramp_rate_c_min?: number | null;
  duration_minutes?: number | null;
  target_stirrer_rpm?: number | null;
  target_shake_speed_cpm?: number | null;
  target_pressure_mpa?: number | null;
  cooling_mode?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface ProcessDetail {
  process: ProcessDefinition;
  steps: ProcessStep[];
}

export interface Batch {
  id: number;
  process_id?: number | null;
  name?: string | null;
  started_at?: string;
  finished_at?: string | null;
  target_temperature_c?: number | null;
  target_stirrer_rpm?: number | null;
  heating_minutes?: number | null;
  stirring_minutes?: number | null;
}

export interface BatchOutcome extends Batch {
  yield_percent?: number | null;
  product_ratio?: number | null;
  notes?: string | null;
}

export interface BatchListResponse {
  batches: Batch[];
  outcomes: BatchOutcome[];
}

export interface ControlEvent {
  id: number;
  batch_id?: number | null;
  event_type: string;
  /** 发起者用户名；系统/控制环事件为 "system"（V21 后端已提供） */
  actor?: string;
  /** 发起者角色；系统事件为 "system" */
  role?: string;
  target_temperature_c?: number | null;
  target_stirrer_rpm?: number | null;
  target_shake_speed_cpm?: number | null;
  reason?: string | null;
  created_at?: string;
  previous_hash?: string | null;
  event_hash?: string | null;
}

export interface BatchDetail {
  batch: Batch;
  outcome?: BatchOutcome | null;
  samples: SensorSample[];
  events: ControlEvent[];
}

export interface LiveResponse {
  runtime?: RuntimeState;
  device_status?: DeviceStatusSummary;
  latest_recommendation?: AiRecommendationEnvelope | null;
  ai_provider?: AiProviderInfo;
  processes?: ProcessDefinition[];
  recent_samples?: SensorSample[];
  recent_batches?: Batch[];
  recent_outcomes?: BatchOutcome[];
  recent_events?: ControlEvent[];
  alarms?: Alarm[];
  ai_memory?: unknown;
  field_scenario?: ScenarioInfo;
  production_line?: ScenarioInfo;
  [key: string]: unknown;
}

export interface ScenarioInfo {
  kind?: string;
  label?: string;
  source?: string;
  device_mode?: string;
  site_label?: string;
  confidence?: number | string;
  signals?: string[];
  actions?: string[];
  notes?: string[];
  requires_operator_inquiry?: boolean | string;
  production_adaptation_blocked?: boolean | string;
  special_handling_required?: boolean | string;
  [key: string]: unknown;
}

export interface AuditChainStatus {
  total_hashed_events?: number;
  checked_events?: number;
  chained_events?: number;
  broken_events?: number;
  window_valid?: boolean;
  valid?: boolean;
  last_event_hash?: string | null;
  checked_from_event_id?: number | null;
  checked_to_event_id?: number | null;
  verification_limit?: number;
  verification_truncated?: boolean;
}

export interface AuditLogsResponse {
  page: number;
  page_size: number;
  total: number;
  events: ControlEvent[];
  chain: AuditChainStatus;
}

export interface AiControlAction {
  action_type: string;
  target?: string;
  status: "planned" | "executed" | "skipped" | "blocked" | string;
  message?: string;
  result?: unknown;
}

export interface AiControlResponse {
  mode?: string;
  dry_run?: boolean;
  decision?: string;
  rationale?: string;
  recommended_targets?: ControlTargets | null;
  safety?: Record<string, unknown>;
  actions?: AiControlAction[];
  [key: string]: unknown;
}

export interface ExperimentPlanStep {
  step_no: number;
  name?: string;
  target_temperature_c?: number | null;
  target_stirrer_rpm?: number | null;
  target_shake_speed_cpm?: number | null;
  target_pressure_mpa?: number | null;
  duration_minutes?: number | null;
  operator_action?: string;
  safety_check?: string;
}

export interface ExperimentPlanResponse {
  plan_id?: string;
  title?: string;
  status?: string;
  source?: string;
  recommendation?: AiRecommendationEnvelope;
  objective?: string;
  sop_summary?: string;
  steps?: ExperimentPlanStep[];
  acceptance_criteria?: string[];
  safety_notes?: string[];
  model_boundary?: string[];
  next_actions?: string[];
  [key: string]: unknown;
}

export interface HistoryRecord {
  device_id?: string;
  batch_id?: number | null;
  timestamp?: string;
  data?: {
    current_temp?: number | null;
    current_pressure?: number | null;
    stir_speed?: number | null;
    shake_speed?: number | null;
    tilt_state?: number | null;
    tilt_angle?: number | null;
    tilt_angle_source?: string;
    flow_rate?: number | null;
    product_concentration?: number | null;
    ph?: number | null;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

export interface HistoryResponse {
  device_id?: string;
  page?: number;
  page_size?: number;
  interval?: string | null;
  start_time?: string;
  end_time?: string;
  items?: HistoryRecord[];
  records?: HistoryRecord[];
}

export interface RealtimePayload {
  runtime?: RuntimeState;
  device_id: string;
  timestamp: string;
  status: string;
  device_online: boolean;
  device_status?: DeviceStatusItem;
  data?: {
    current_temp?: number | null;
    current_pressure?: number | null;
    stir_speed?: number | null;
    shake_speed?: number | null;
    tilt_state?: number | null;
    tilt_angle?: number | null;
    tilt_angle_source?: string;
    flow_rate?: number | null;
    product_concentration_percent?: number | null;
    ph?: number | null;
    phase?: string;
    progress?: number | null;
  };
  alarms?: Alarm[];
}

export interface ModbusRegisterItem {
  name: string;
  label?: string;
  address?: number;
  access?: string;
  value?: number | string | null;
  raw?: number | string | null;
  scale?: number;
  offset?: number;
  source?: string;
  unit?: string;
}

export interface ModbusRegistersResponse {
  device_id?: string;
  mode?: string;
  slave_id?: number;
  serial?: Record<string, unknown>;
  tcp?: {
    listening?: boolean;
    bind?: string;
    unit_id?: number;
    tls?: boolean;
    updated_at?: string;
    [key: string]: unknown;
  };
  read_registers?: ModbusRegisterItem[];
  write_registers?: ModbusRegisterItem[];
  coils?: ModbusRegisterItem[];
  discrete_inputs?: ModbusRegisterItem[];
  [key: string]: unknown;
}

export interface IntegrationTask {
  id: number;
  external_task_id?: string | null;
  source?: string;
  action?: string;
  status?: "received" | "executing" | "executed" | "failed" | "rejected" | string;
  request?: unknown;
  response?: unknown;
  created_at?: string;
  updated_at?: string;
}

export interface ConfigSummary {
  device_mode?: string;
  device?: Record<string, unknown>;
  safety?: {
    control?: Record<string, unknown>;
    temperature?: { min_c?: number; max_c?: number; max_step_c?: number; default_target_c?: number };
    stirrer?: { min_rpm?: number; max_rpm?: number; max_step_rpm?: number; default_target_rpm?: number };
    optimizer?: Record<string, unknown>;
    forbidden_control_zones?: ForbiddenZone[];
    [key: string]: unknown;
  };
  field_scenario?: ScenarioInfo;
  production_line?: ScenarioInfo;
  ai_memory?: Record<string, unknown>;
  ai_provider?: AiProviderInfo;
  local_ai?: {
    mode?: string;
    missing?: string[];
    [key: string]: unknown;
  };
  permissions?: Record<string, unknown>;
  data_security?: {
    storage_encryption?: {
      enabled?: boolean;
      algorithm?: string;
      key_source?: string;
      encrypted_fields?: string[];
    };
    [key: string]: unknown;
  };
  integrations?: {
    rest_api?: unknown;
    cli?: unknown;
    mqtt?: Record<string, unknown>;
    mqtt_status?: Record<string, unknown>;
    ainas_ready?: unknown;
    ainas_task_api?: unknown;
    modbus_rtu?: unknown;
    modbus_tcp?: Record<string, unknown>;
    modbus_tcp_status?: Record<string, unknown>;
    json_bridge?: unknown;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

export interface ForbiddenZone {
  name?: string;
  description?: string;
  min_temperature_c?: number;
  max_temperature_c?: number;
  min_stirrer_rpm?: number;
  max_stirrer_rpm?: number;
  [key: string]: unknown;
}

export interface DemoContext {
  demo?: boolean;
  sensor_data_policy?: string;
  latest_recommendation?: AiRecommendationEnvelope | null;
  ai_provider?: AiProviderInfo;
  processes?: ProcessDefinition[];
  recent_batches?: Batch[];
  recent_outcomes?: BatchOutcome[];
  recent_events?: ControlEvent[];
  demo_alarms?: Alarm[];
  ai_memory?: unknown;
  [key: string]: unknown;
}

export interface ProcessApplyResponse {
  process?: ProcessDefinition;
  batch?: Batch;
  applied_targets?: ControlTargets;
  status?: string;
}

export interface ProcessStopResponse {
  stopped_batch_id?: number | null;
  process_id?: number | null;
  batch?: Batch | null;
  recovery?: unknown;
  active_batch_id?: number | null;
  auto_enabled?: boolean;
  stopped_targets?: ControlTargets;
}
