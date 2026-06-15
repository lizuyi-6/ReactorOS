use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{any, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use tokio::time::MissedTickBehavior;
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    ai_provider::{
        local_envelope, stale_local_envelope, stepfun_envelope, AiProvider,
        AiRecommendationEnvelope, AiRecommendationProvider,
    },
    config::{DeviceConfig, DeviceMode, SafetyConfig},
    control::{clamp_operator_targets, forbidden_control_zone, SafeCommand},
    db::{
        Batch, BatchOutcome, ControlEvent, Db, DemoAlarm, NewProcessStep, ProcessDefinition,
        ProcessDetail, ProcessStep, ProductResult, SensorSampleRecord,
    },
    device::{ComponentControlCommand, ComponentControlOutcome, SharedDevice},
    field_scenario::{detect_field_scenario, FieldScenarioContext, FieldScenarioProfile},
    local_ai::LocalAiStatus,
    memory::{AiMemory, AiMemorySummary, LimitLevel, SensorLimit},
    number::round2,
    optimizer::{recommend_with_memory, Recommendation},
    reports::{
        build_audit_csv, build_batch_report_markdown, build_batches_csv, build_batches_xlsx,
    },
    state::{
        device_status_field_fault_reason, downstream_command_fault_reason, fit_tilt_angle_deg,
        timestamp_age_ms, timestamp_is_fresh, validate_sensor_snapshot, validate_sensor_tilt_state,
        ControlTargets, RuntimeState, SensorRange, SensorSnapshot, SharedState,
        SENSOR_FLOW_RATE_L_MIN_RANGE, SENSOR_PH_RANGE, SENSOR_PRESSURE_MPA_RANGE,
        SENSOR_PRODUCT_CONCENTRATION_PERCENT_RANGE, SENSOR_SHAKE_SPEED_CPM_RANGE,
        SENSOR_STIRRER_RPM_RANGE, SENSOR_TEMPERATURE_C_RANGE,
    },
};

#[path = "api_integrations.rs"]
mod api_integrations;
pub(crate) use api_integrations::execute_integration_task;
pub use api_integrations::AinasTaskRequest;
use api_integrations::{create_ainas_task, get_ainas_task, list_ainas_tasks};

#[path = "api_response.rs"]
mod api_response;
pub(crate) use api_response::{success, ApiJson, AppError, V1Envelope};

#[path = "api_auth.rs"]
mod api_auth;
use api_auth::{
    authenticated_user, login_response, permission_policy, require_admin, require_permission,
    AuthUser, LoginRequest, LoginResponse, Permission,
};

#[path = "modbus_registers.rs"]
mod modbus_register_map;
pub(crate) use modbus_register_map::apply_modbus_register_write;
pub(crate) use modbus_register_map::TargetUpdateInterlockMode;

#[derive(Debug, Clone)]
pub(crate) struct UnfinishedBatchStatus {
    pub unfinished_batch_ids: Vec<i64>,
    pub unexpected_batch_ids: Vec<i64>,
    pub runtime_active_batch_missing: bool,
}

impl UnfinishedBatchStatus {
    pub(crate) fn is_consistent(&self) -> bool {
        self.unexpected_batch_ids.is_empty() && !self.runtime_active_batch_missing
    }

    pub(crate) fn has_unfinished_batch(&self, runtime: &RuntimeState) -> bool {
        runtime.active_batch_id.is_some() || !self.unfinished_batch_ids.is_empty()
    }

    pub(crate) fn recovery_required(&self) -> bool {
        !self.is_consistent()
    }

    pub(crate) fn reason(&self, runtime: &RuntimeState) -> String {
        format!(
            "database has unfinished batch records {:?} while runtime active batch is {:?}",
            self.unfinished_batch_ids, runtime.active_batch_id
        )
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub runtime: SharedState,
    pub device: SharedDevice,
    pub device_mode: DeviceMode,
    pub device_config: Arc<DeviceConfig>,
    pub safety: Arc<SafetyConfig>,
    pub ai_memory: Arc<AiMemory>,
    pub ai_provider: Option<Arc<AiProvider>>,
    pub test_reset_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct HttpTlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub service: &'static str,
}

#[derive(Debug, Serialize)]
pub struct LiveResponse {
    pub runtime: RuntimeState,
    pub device_status: DeviceStatusSummary,
    pub latest_recommendation: Option<Recommendation>,
    pub ai_provider: AiRecommendationProvider,
    pub processes: Vec<ProcessDefinition>,
    pub recent_samples: Vec<SensorSampleRecord>,
    pub recent_batches: Vec<Batch>,
    pub recent_outcomes: Vec<BatchOutcome>,
    pub recent_events: Vec<ControlEvent>,
    pub alarms: Vec<Value>,
    pub ai_memory: AiMemorySummary,
    pub field_scenario: FieldScenarioProfile,
}

#[derive(Debug, Serialize)]
pub struct DemoContextResponse {
    pub demo: bool,
    pub sensor_data_policy: &'static str,
    pub latest_recommendation: Option<Recommendation>,
    pub ai_provider: AiRecommendationProvider,
    pub processes: Vec<ProcessDefinition>,
    pub recent_batches: Vec<Batch>,
    pub recent_outcomes: Vec<BatchOutcome>,
    pub recent_events: Vec<ControlEvent>,
    pub demo_alarms: Vec<DemoAlarm>,
    pub ai_memory: AiMemorySummary,
}

#[derive(Debug, Serialize)]
pub struct DeviceStatusSummary {
    pub total_count: usize,
    pub online_count: usize,
    pub devices: Vec<DeviceStatusItem>,
    pub sensors: Vec<DeviceSensorItem>,
    pub components: Vec<DeviceComponentItem>,
}

#[derive(Debug, Serialize)]
pub struct DeviceStatusItem {
    pub device_id: String,
    pub device_role: String,
    pub online: bool,
    pub status: String,
    pub auto_enabled: bool,
    pub manual_lock: bool,
    pub last_seen_at: Option<String>,
    pub last_seen_age_ms: Option<i64>,
    pub stale_after_ms: i64,
    pub active_batch_id: Option<i64>,
    pub emergency_stop: bool,
    pub last_sensor_error: Option<String>,
    pub last_control_error: Option<String>,
    pub unfinished_batch_ids: Vec<i64>,
    pub unexpected_unfinished_batch_ids: Vec<i64>,
    pub relay: Option<u8>,
    pub motor: Option<u8>,
    pub tilt: Option<u8>,
    pub speed_delay_us: Option<u64>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    pub last_command_request_id: Option<String>,
    pub last_command_ok: Option<bool>,
    pub last_command_error: Option<String>,
    pub sensors: Vec<DeviceSensorItem>,
    pub components: Vec<DeviceComponentItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceSensorItem {
    pub sensor_id: String,
    pub label: String,
    pub unit: String,
    pub status: String,
    pub value: Option<f64>,
    pub target: Option<f64>,
    pub source: String,
    pub component_id: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceComponentItem {
    pub component_id: String,
    pub component_type: String,
    pub label: String,
    pub controllable: bool,
    pub status: String,
    pub state: Value,
    pub actions: Vec<ComponentActionItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentActionItem {
    pub action: String,
    pub label: String,
    pub value_type: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub unit: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceCapabilitiesResponse {
    pub total_count: usize,
    pub online_count: usize,
    pub devices: Vec<DeviceCapabilityDevice>,
}

#[derive(Debug, Serialize)]
pub struct DeviceCapabilityDevice {
    pub device_id: String,
    pub device_role: String,
    pub mode: String,
    pub online: bool,
    pub status: String,
    pub sensors: Vec<DeviceSensorItem>,
    pub components: Vec<DeviceComponentItem>,
}

#[derive(Debug, Deserialize)]
pub struct ComponentControlRequest {
    pub action: String,
    pub value: Option<Value>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable_string_field"
    )]
    pub reason: Option<Option<String>>,
}

#[derive(Debug, Serialize)]
pub struct ComponentControlResponse {
    pub device_id: String,
    pub component: DeviceComponentItem,
    pub outcome: Option<ComponentControlOutcome>,
}

#[derive(Debug, Deserialize)]
pub struct StartBatchRequest {
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i64_field")]
    pub process_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub target_temperature_c: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub target_stirrer_rpm: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub target_shake_speed_cpm: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub heating_minutes: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub stirring_minutes: Option<Option<f64>>,
}

#[derive(Debug, Deserialize)]
pub struct ProductResultRequest {
    pub batch_id: i64,
    pub yield_percent: f64,
    pub product_ratio: f64,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProcessRequest {
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProcessRequest {
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProcessStepRequest {
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub name: Option<String>,
    pub target_temperature_c: f64,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub ramp_rate_c_min: Option<Option<f64>>,
    pub duration_minutes: f64,
    pub target_stirrer_rpm: f64,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub target_shake_speed_cpm: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub target_pressure_mpa: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub cooling_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProcessApplyResponse {
    pub process: ProcessDefinition,
    pub batch: Batch,
    pub applied_targets: ControlTargets,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ProcessStopResponse {
    pub stopped_batch_id: i64,
    pub process_id: Option<i64>,
    pub batch: Option<Batch>,
    pub recovery: Option<String>,
    pub active_batch_id: Option<i64>,
    pub auto_enabled: bool,
    pub stopped_targets: ControlTargets,
}

#[derive(Debug, Serialize)]
pub struct BatchListResponse {
    pub batches: Vec<Batch>,
    pub outcomes: Vec<BatchOutcome>,
}

#[derive(Debug, Serialize)]
pub struct BatchDetailResponse {
    pub batch: Batch,
    pub outcome: Option<BatchOutcome>,
    pub samples: Vec<SensorSampleRecord>,
    pub events: Vec<ControlEvent>,
}

#[derive(Debug, Deserialize)]
pub struct AutoRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct StopProcessRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ManualLockRequest {
    pub locked: bool,
}

#[derive(Debug, Deserialize)]
pub struct TargetRequest {
    pub temperature_c: f64,
    pub stirrer_rpm: f64,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub shake_speed_cpm: Option<Option<f64>>,
}

#[derive(Debug, Deserialize)]
pub struct V1ControlRequest {
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub command_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub timestamp: Option<String>,
    pub params: V1ControlParams,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub priority: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_bool_field")]
    pub auto_start: Option<Option<bool>>,
}

#[derive(Debug, Deserialize)]
pub struct V1ControlParams {
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub heat_time: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub hold_time: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub cool_time: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub stir_speed: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub shake_speed: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub target_temp: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_optional_number_field")]
    pub target_pressure: Option<Option<f64>>,
}

#[derive(Debug, Deserialize)]
pub struct LiveQuery {
    pub sample_limit: Option<usize>,
    pub include_processes: Option<bool>,
    pub include_batches: Option<bool>,
    pub include_events: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct V1HistoryQuery {
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub interval: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub event_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditLogResponse {
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub events: Vec<ControlEvent>,
    pub chain: crate::db::AuditChainStatus,
}

#[derive(Debug, Deserialize)]
pub struct ModbusWriteRequest {
    pub value: f64,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct V1ProcessRequest {
    pub process_id: String,
    pub name: String,
    pub phases: Vec<V1ProcessPhase>,
}

#[derive(Debug, Deserialize)]
pub struct V1ProcessPhase {
    pub phase: String,
    pub params: Value,
}

#[derive(Debug, Deserialize)]
pub struct PipelineSampleRequest {
    pub temperature_c: f64,
    pub pressure_mpa: f64,
    pub stirrer_rpm: f64,
    pub shake_speed_cpm: f64,
    pub tilt_state: u8,
    pub flow_rate_l_min: f64,
    pub product_concentration_percent: f64,
    pub ph: f64,
}

#[derive(Debug, Deserialize)]
pub struct AiControlRequest {
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub intent: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string_field")]
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_bool_field")]
    pub dry_run: Option<Option<bool>>,
    #[serde(default, deserialize_with = "deserialize_optional_bool_field")]
    pub allow_process_start: Option<Option<bool>>,
    #[serde(default, deserialize_with = "deserialize_optional_bool_field")]
    pub allow_process_stop: Option<Option<bool>>,
    #[serde(default, deserialize_with = "deserialize_optional_bool_field")]
    pub allow_component_control: Option<Option<bool>>,
    #[serde(default, deserialize_with = "deserialize_optional_bool_field")]
    pub allow_target_adjustment: Option<Option<bool>>,
    #[serde(default, deserialize_with = "deserialize_optional_i64_field")]
    pub preferred_process_id: Option<Option<i64>>,
}

#[derive(Debug, Serialize)]
pub struct AiControlResponse {
    pub mode: String,
    pub dry_run: bool,
    pub decision: String,
    pub rationale: String,
    pub recommended_targets: Option<ControlTargets>,
    pub safety: AiControlSafety,
    pub actions: Vec<AiControlAction>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiControlSafety {
    pub fresh_sample_required: bool,
    pub sensor_fresh: bool,
    pub emergency_stop: bool,
    pub manual_lock: bool,
    pub device_online: bool,
    pub active_batch_id: Option<i64>,
    pub batch_recovery_required: bool,
    pub unfinished_batch_ids: Vec<i64>,
    pub unexpected_batch_ids: Vec<i64>,
    pub high_alarm_count: usize,
    pub warning_alarm_count: usize,
    pub stop_product_concentration_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiControlAction {
    pub action_type: String,
    pub target: String,
    pub status: String,
    pub message: String,
    pub result: Option<Value>,
}

#[derive(Debug, Serialize)]
struct AiControlAuditReason {
    decision: String,
    rationale: String,
    actions: Vec<AiControlAuditAction>,
}

#[derive(Debug, Serialize)]
struct AiControlAuditAction {
    action_type: String,
    target: String,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentPlanStep {
    pub step_no: usize,
    pub name: String,
    pub target_temperature_c: f64,
    pub target_stirrer_rpm: f64,
    pub target_shake_speed_cpm: f64,
    pub target_pressure_mpa: f64,
    pub duration_minutes: f64,
    pub operator_action: String,
    pub safety_check: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentPlanResponse {
    pub plan_id: String,
    pub title: String,
    pub status: String,
    pub source: String,
    pub recommendation: Recommendation,
    pub objective: String,
    pub sop_summary: String,
    pub steps: Vec<ExperimentPlanStep>,
    pub acceptance_criteria: Vec<String>,
    pub safety_notes: Vec<String>,
    pub model_boundary: Vec<String>,
    pub next_actions: Vec<String>,
}

pub fn router(state: AppState, assets: PathBuf) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/live", get(live))
        .route("/api/demo/context", get(demo_context))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/me", get(auth_me))
        .route("/api/devices/status", get(devices_status))
        .route("/api/v1/devices/status", get(devices_status))
        .route("/api/devices/capabilities", get(devices_capabilities))
        .route("/api/v1/devices/capabilities", get(devices_capabilities))
        .route(
            "/api/devices/:device_id/components/:component_id/control",
            post(control_component),
        )
        .route(
            "/api/v1/devices/:device_id/components/:component_id/control",
            post(control_component),
        )
        .route("/api/v1/reactor/:device_id/control", post(v1_control))
        .route("/api/ai/control", post(ai_control))
        .route("/api/v1/ai/control", post(ai_control))
        .route("/api/ai/experiment-plan", get(ai_experiment_plan))
        .route("/api/v1/ai/experiment-plan", get(ai_experiment_plan))
        .route(
            "/api/v1/reactor/:device_id/samples",
            post(v1_pipeline_sample),
        )
        .route("/api/v1/reactor/:device_id/realtime", get(v1_realtime))
        .route("/api/v1/reactor/:device_id/history", get(v1_history))
        .route("/api/v1/reactor/:device_id/process", post(v1_process))
        .route("/ws/v1/reactor/:device_id/realtime", get(v1_realtime_ws))
        .route("/api/audit/logs", get(audit_logs))
        .route("/api/audit/export.csv", get(audit_export_csv))
        .route("/api/config/summary", get(config_summary))
        .route("/api/permissions/roles", get(permission_roles))
        .route(
            "/api/integrations/ainas/tasks",
            get(list_ainas_tasks).post(create_ainas_task),
        )
        .route("/api/integrations/ainas/tasks/:id", get(get_ainas_task))
        .route(
            "/api/v1/integrations/ainas/tasks",
            get(list_ainas_tasks).post(create_ainas_task),
        )
        .route("/api/v1/integrations/ainas/tasks/:id", get(get_ainas_task))
        .route("/api/modbus/registers", get(modbus_registers))
        .route(
            "/api/modbus/registers/:register/read",
            get(modbus_register_read),
        )
        .route(
            "/api/modbus/registers/:register/write",
            post(modbus_register_write),
        )
        .route("/api/processes", get(list_processes).post(create_process))
        .route("/api/processes/:id", get(get_process).put(update_process))
        .route("/api/processes/:id/steps", post(add_process_step))
        .route(
            "/api/processes/:id/steps/:step_id",
            put(update_process_step),
        )
        .route("/api/processes/:id/apply", post(apply_process))
        .route("/api/processes/:id/start", post(start_process))
        .route("/api/processes/:id/stop", post(stop_process_by_id))
        .route("/api/processes/current/stop", post(stop_current_process))
        .route("/api/v1/processes/:id/apply", post(apply_process))
        .route("/api/v1/processes/:id/start", post(start_process))
        .route("/api/v1/processes/:id/stop", post(stop_process_by_id))
        .route("/api/v1/processes/current/stop", post(stop_current_process))
        .route("/api/batches", get(list_batches))
        .route("/api/batches/export.csv", get(batches_export_csv))
        .route("/api/batches/export.xlsx", get(batches_export_xlsx))
        .route("/api/batches/start", post(start_batch))
        .route("/api/batches/:id", get(get_batch_detail))
        .route("/api/batches/:id/report.md", get(batch_report_markdown))
        .route("/api/batches/:id/finish", post(finish_batch))
        .route("/api/product-results", post(product_results))
        .route("/api/control/auto", post(set_auto))
        .route("/api/control/manual-lock", post(set_manual_lock))
        .route("/api/control/fault/reset", post(reset_control_fault))
        .route("/api/control/targets", post(set_targets))
        .route("/api/control/emergency-stop", post(emergency_stop))
        .route(
            "/api/control/emergency-stop/reset",
            post(reset_emergency_stop),
        )
        .route("/api/test/reset", post(test_reset))
        .route("/api/test/pipeline-sample", post(test_pipeline_sample))
        .route(
            "/api/recommendations/latest",
            get(latest_recommendation).post(generate_latest_recommendation),
        )
        .route("/api/*path", any(api_not_found))
        .nest_service(
            "/",
            ServeDir::new(&assets).not_found_service(ServeFile::new(assets.join("index.html"))),
        )
        .with_state(state)
}

pub async fn serve(
    state: AppState,
    assets: PathBuf,
    bind: SocketAddr,
    tls: Option<HttpTlsConfig>,
) -> Result<()> {
    let router = router(state, assets);
    if let Some(tls) = tls {
        crate::tls::install_rustls_provider();
        let tls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(tls.cert, tls.key).await?;
        tracing::info!("listening on https://{bind}");
        axum_server::bind_rustls(bind, tls_config)
            .serve(router.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        tracing::info!("listening on http://{bind}");
        axum::serve(listener, router).await?;
    }
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "reactor-edge-daemon",
    })
}

async fn live(
    State(state): State<AppState>,
    Query(query): Query<LiveQuery>,
) -> Result<Json<LiveResponse>, AppError> {
    let runtime = state.runtime.read().await.clone();
    ensure_fresh_sample(&state, &runtime)?;
    let sample_limit = query.sample_limit.unwrap_or(480).clamp(1, 480);
    let recent_samples = state.db.recent_sample_records_sqlx(sample_limit).await?;
    let processes = if query.include_processes.unwrap_or(true) {
        state.db.list_processes_sqlx().await?
    } else {
        Vec::new()
    };
    let (recent_batches, recent_outcomes) = if query.include_batches.unwrap_or(true) {
        (
            state.db.recent_batches_sqlx(20).await?,
            state.db.recent_batch_outcomes_sqlx(20).await?,
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let recent_events = if query.include_events.unwrap_or(true) {
        state.db.recent_control_events_sqlx(100).await?
    } else {
        Vec::new()
    };
    let ai_memory = AiMemorySummary::from(state.ai_memory.as_ref());
    let recommendation = state
        .db
        .latest_recommendation_sqlx()
        .await?
        .filter(|recommendation| provider_allows_recommendation(&state, recommendation))
        .filter(|recommendation| recommendation.based_on_batch_count > 0);
    let ai_provider = local_provider_for(&state);
    let device_status = device_status_summary_with_db(&state, &runtime).await?;
    let alarms = live_alarms_for(
        &state,
        state.safety.as_ref(),
        &runtime,
        runtime.latest_sample.as_ref(),
        state.ai_memory.as_ref(),
    )
    .await?;
    let field_scenario = detect_field_scenario(FieldScenarioContext {
        device_mode: &state.device_mode,
        runtime: Some(&runtime),
        include_runtime_signals: true,
        memory: state.ai_memory.as_ref(),
        processes: &processes,
        recent_batches: &recent_batches,
        recent_outcomes: &recent_outcomes,
    });
    Ok(Json(LiveResponse {
        runtime,
        device_status,
        latest_recommendation: recommendation,
        ai_provider,
        processes,
        recent_samples,
        recent_batches,
        recent_outcomes,
        recent_events,
        alarms,
        ai_memory,
        field_scenario,
    }))
}

async fn demo_context(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<DemoContextResponse>>, AppError> {
    Ok(Json(success(DemoContextResponse {
        demo: true,
        sensor_data_policy:
            "demo context excludes sensor_samples and never fabricates runtime sensor values",
        latest_recommendation: state
            .db
            .latest_recommendation_sqlx()
            .await?
            .filter(|recommendation| provider_allows_recommendation(&state, recommendation))
            .filter(|recommendation| recommendation.based_on_batch_count > 0),
        ai_provider: local_provider_for(&state),
        processes: state.db.list_processes_sqlx().await?,
        recent_batches: state.db.recent_batches_sqlx(20).await?,
        recent_outcomes: state.db.recent_batch_outcomes_sqlx(20).await?,
        recent_events: state.db.recent_control_events_sqlx(100).await?,
        demo_alarms: state.db.recent_demo_alarms_sqlx(20).await?,
        ai_memory: AiMemorySummary::from(state.ai_memory.as_ref()),
    })))
}

async fn audit_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<V1Envelope<AuditLogResponse>>, AppError> {
    require_permission(&headers, Permission::ViewAudit)?;
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 500);
    let event_type = query
        .event_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let total = state.db.audit_event_count_sqlx(event_type).await?;
    let events = state
        .db
        .audit_events_sqlx(page_size, (page - 1) * page_size, event_type)
        .await?;
    let chain = state.db.audit_chain_status_sqlx().await?;
    Ok(Json(success(AuditLogResponse {
        page,
        page_size,
        total,
        events,
        chain,
    })))
}

async fn audit_export_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<impl IntoResponse, AppError> {
    require_permission(&headers, Permission::ExportReports)?;
    let event_type = query
        .event_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let events = state.db.audit_events_sqlx(10_000, 0, event_type).await?;
    let csv = build_audit_csv(&events);
    Ok((
        [
            ("content-type", "text/csv; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"reactor-audit-log.csv\"",
            ),
        ],
        csv,
    ))
}

async fn auth_login(
    ApiJson(payload): ApiJson<LoginRequest>,
) -> Result<Json<V1Envelope<LoginResponse>>, AppError> {
    Ok(Json(success(login_response(payload)?)))
}

async fn auth_me(headers: HeaderMap) -> Result<Json<V1Envelope<AuthUser>>, AppError> {
    let user = authenticated_user(&headers)?;
    Ok(Json(success(user)))
}

async fn config_summary(State(state): State<AppState>) -> Json<V1Envelope<Value>> {
    let mqtt_status = crate::mqtt::mqtt_status_snapshot().await;
    let modbus_tcp_status = crate::modbus_tcp::modbus_tcp_status_snapshot().await;
    let field_scenario = detect_field_scenario(FieldScenarioContext::config_only(
        &state.device_mode,
        state.ai_memory.as_ref(),
    ));
    Json(success(json!({
        "device_mode": state.device_mode,
        "device": state.device_config.as_ref(),
        "safety": state.safety.as_ref(),
        "field_scenario": field_scenario,
        "ai_memory": AiMemorySummary::from(state.ai_memory.as_ref()),
        "ai_provider": local_provider_for(&state),
        "local_ai": LocalAiStatus::from_env(),
        "permissions": permission_policy(),
        "data_security": {
            "storage_encryption": state.db.encryption_status()
        },
        "integrations": {
            "rest_api": true,
            "cli": true,
            "mqtt": mqtt_status.enabled,
            "mqtt_status": mqtt_status,
            "ainas_ready": true,
            "ainas_task_api": true,
            "modbus_rtu": matches!(state.device_mode, DeviceMode::Modbus),
            "modbus_tcp": modbus_tcp_status.enabled,
            "modbus_tcp_status": modbus_tcp_status,
            "json_bridge": matches!(state.device_mode, DeviceMode::JsonBridge)
        }
    })))
}

async fn permission_roles() -> Json<V1Envelope<Value>> {
    Json(success(permission_policy()))
}

async fn modbus_registers(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    let runtime = state.runtime.read().await.clone();
    let batch_status = unfinished_batch_status(&state, &runtime).await?;
    let tcp_status = crate::modbus_tcp::modbus_tcp_status_snapshot().await;
    Ok(Json(success(modbus_register_map::registers_payload(
        &state,
        &runtime,
        &batch_status,
        &tcp_status,
    ))))
}

async fn modbus_register_read(
    State(state): State<AppState>,
    Path(register): Path<String>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    let runtime = state.runtime.read().await.clone();
    Ok(Json(success(modbus_register_map::read_register_payload(
        &state, &runtime, &register,
    )?)))
}

async fn modbus_register_write(
    State(state): State<AppState>,
    Path(register): Path<String>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ModbusWriteRequest>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    require_admin(&headers)?;
    let response = modbus_register_map::apply_modbus_register_write(
        &state,
        &register,
        payload.value,
        payload.reason,
    )
    .await?;
    Ok(Json(success(response)))
}

async fn list_processes(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<Vec<ProcessDefinition>>>, AppError> {
    Ok(Json(success(state.db.list_processes_sqlx().await?)))
}

async fn create_process(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<CreateProcessRequest>,
) -> Result<Json<V1Envelope<ProcessDefinition>>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    ensure_production_basis_write_allowed(&state, "process definition create").await?;
    let name = clean_label(payload.name, "未命名工艺", 80);
    let description = clean_label(payload.description, "", 240);
    let process = state
        .db
        .create_process_with_audit_sqlx(
            &name,
            &description,
            "process_created",
            "operator created process",
        )
        .await?;
    Ok(Json(success(process)))
}

async fn get_process(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
) -> Result<Json<V1Envelope<ProcessDetail>>, AppError> {
    let Some(process) = process_detail_or_bad_request(&state, process_id).await? else {
        return Err(AppError::not_found("process not found"));
    };
    Ok(Json(success(process)))
}

async fn update_process(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<UpdateProcessRequest>,
) -> Result<Json<V1Envelope<ProcessDefinition>>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    ensure_production_basis_write_allowed(&state, "process definition update").await?;
    let Some(current) = process_detail_or_bad_request(&state, process_id).await? else {
        return Err(AppError::not_found("process not found"));
    };
    let name = clean_label(payload.name, &current.process.name, 80);
    let description = clean_label(payload.description, &current.process.description, 240);
    let status = clean_status(payload.status.as_deref().unwrap_or(&current.process.status))?;
    let Some(process) = state
        .db
        .update_process_with_audit_sqlx(
            process_id,
            &name,
            &description,
            status,
            "process_updated",
            "operator updated process",
        )
        .await?
    else {
        return Err(AppError::not_found("process not found"));
    };
    Ok(Json(success(process)))
}

async fn add_process_step(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ProcessStepRequest>,
) -> Result<Json<V1Envelope<ProcessStep>>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    ensure_production_basis_write_allowed(&state, "process step add").await?;
    let step = validate_process_step(&state.safety, payload)?;
    let Some(step) = state
        .db
        .add_process_step_with_audit_sqlx(
            process_id,
            &step,
            "process_step_added",
            "operator added process step",
        )
        .await?
    else {
        return Err(AppError::not_found("process not found"));
    };
    Ok(Json(success(step)))
}

async fn update_process_step(
    State(state): State<AppState>,
    Path((process_id, step_id)): Path<(i64, i64)>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ProcessStepRequest>,
) -> Result<Json<V1Envelope<ProcessStep>>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    ensure_production_basis_write_allowed(&state, "process step update").await?;
    let step = validate_process_step(&state.safety, payload)?;
    let Some(step) = state
        .db
        .update_process_step_with_audit_sqlx(
            process_id,
            step_id,
            &step,
            "process_step_updated",
            "operator updated process step",
        )
        .await?
    else {
        return Err(AppError::not_found("process step not found"));
    };
    Ok(Json(success(step)))
}

async fn apply_process(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<V1Envelope<ProcessApplyResponse>>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let response = start_process_lifecycle(&state, process_id, "process_applied").await?;
    Ok(Json(success(response)))
}

async fn start_process(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<V1Envelope<ProcessApplyResponse>>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let response = start_process_lifecycle(&state, process_id, "process_started").await?;
    Ok(Json(success(response)))
}

async fn stop_process_by_id(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
    headers: HeaderMap,
    payload: Option<Json<StopProcessRequest>>,
) -> Result<Json<V1Envelope<ProcessStopResponse>>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let response = stop_process_lifecycle(
        &state,
        Some(process_id),
        "process_stopped",
        payload.and_then(|Json(payload)| payload.reason),
    )
    .await?;
    Ok(Json(success(response)))
}

async fn stop_current_process(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Option<Json<StopProcessRequest>>,
) -> Result<Json<V1Envelope<ProcessStopResponse>>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let response = stop_process_lifecycle(
        &state,
        None,
        "process_stopped",
        payload.and_then(|Json(payload)| payload.reason),
    )
    .await?;
    Ok(Json(success(response)))
}

async fn start_process_lifecycle(
    state: &AppState,
    process_id: i64,
    event_type: &'static str,
) -> Result<ProcessApplyResponse, AppError> {
    let Some(detail) = process_detail_or_bad_request(state, process_id).await? else {
        return Err(AppError::not_found("process not found"));
    };
    if detail.steps.is_empty() {
        return Err(AppError::bad_request(
            "process must contain at least one step before starting",
        ));
    }
    let targets = targets_from_process_steps(&state.safety, &detail.steps)?;
    let acknowledged_safety_latches = {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_process_can_start(state, &runtime).await?;
        SafetyLatchGenerations::from_runtime(&runtime)
    };
    let batch = state
        .db
        .create_batch_for_process_sqlx(
            Some(process_id),
            &detail.process.name,
            targets.temperature_c,
            targets.stirrer_rpm,
            seconds_to_minutes(Some(targets.heat_time_s)),
            seconds_to_minutes(Some(targets.hold_time_s)),
        )
        .await?;
    let start_reason = if event_type == "process_applied" {
        "process applied from persisted process definition"
    } else if event_type == "ainas_process_started" {
        "process started by AINAS remote task"
    } else {
        "process started from persisted process definition"
    };
    if let Err(err) = start_process_on_device(state, &targets, Some(batch.id)).await {
        audit_start_failed_before_activation(state, Some(batch.id), &targets, "process", &err)
            .await;
        if let Err(finish_err) = state.db.finish_batch_sqlx(batch.id).await {
            tracing::warn!("failed to mark failed process start batch finished: {finish_err}");
        }
        return Err(err);
    }
    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            Some(batch.id),
            event_type,
            Some(&SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: start_reason.to_string(),
            }),
            start_reason,
        )
        .await
    {
        let rollback_stop_error = rollback_failed_activation(state, batch.id, Some(&targets)).await;
        latch_tail_failure_after_device_action(
            state,
            "process start audit",
            tail_error_with_activation_rollback(
                format_error_for_control_fault(&err),
                rollback_stop_error,
            ),
        )
        .await;
        return Err(err.into());
    }
    let process = match state.db.mark_process_applied_sqlx(process_id).await {
        Ok(Some(process)) => process,
        Ok(None) => {
            let rollback_stop_error =
                rollback_failed_activation(state, batch.id, Some(&targets)).await;
            latch_tail_failure_after_device_action(
                state,
                "process start state commit",
                tail_error_with_activation_rollback(
                    "process not found after device start".to_string(),
                    rollback_stop_error,
                ),
            )
            .await;
            return Err(AppError::not_found("process not found"));
        }
        Err(err) => {
            let rollback_stop_error =
                rollback_failed_activation(state, batch.id, Some(&targets)).await;
            latch_tail_failure_after_device_action(
                state,
                "process start state commit",
                tail_error_with_activation_rollback(
                    format_error_for_control_fault(&err),
                    rollback_stop_error,
                ),
            )
            .await;
            return Err(err.into());
        }
    };
    if let Err(err) = commit_process_activation_after_final_interlock(
        state,
        batch.id,
        &targets,
        true,
        Some(acknowledged_safety_latches),
    )
    .await
    {
        let rollback_stop_error = rollback_failed_activation(state, batch.id, Some(&targets)).await;
        latch_tail_failure_after_device_action(
            state,
            "process start final interlock",
            tail_error_with_activation_rollback(err.message().to_string(), rollback_stop_error),
        )
        .await;
        return Err(err);
    }
    Ok(ProcessApplyResponse {
        process,
        batch,
        applied_targets: targets,
        status: "running".to_string(),
    })
}

async fn rollback_failed_activation(
    state: &AppState,
    batch_id: i64,
    activation_targets: Option<&ControlTargets>,
) -> Option<String> {
    let stopped_targets = process_stop_targets(state);
    if let Some(activation_targets) = activation_targets {
        if let Err(err) = stop_process_on_device(state, &stopped_targets).await {
            tracing::warn!("failed to write stop targets during activation rollback: {err:?}");
            let message = err.message().to_string();
            let mut runtime = state.runtime.write().await;
            runtime.active_batch_id = Some(batch_id);
            runtime.auto_enabled = false;
            runtime.targets = activation_targets.clone();
            runtime.latch_control_fault(format!(
                "activation rollback stop command failed after device start; field may still be running: {message}"
            ));
            return Some(message);
        }
    }
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id == Some(batch_id) {
            runtime.active_batch_id = None;
            runtime.auto_enabled = false;
        }
        runtime.targets = stopped_targets;
    }
    if let Err(err) = state.db.finish_batch_sqlx(batch_id).await {
        tracing::warn!("failed to mark failed activation batch finished: {err}");
    }
    None
}

fn tail_error_with_activation_rollback(err: String, rollback_stop_error: Option<String>) -> String {
    match rollback_stop_error {
        Some(stop_error) => format!(
            "{err}; activation rollback stop command also failed, field may still be running: {stop_error}"
        ),
        None => err,
    }
}

async fn audit_start_failed_before_activation(
    state: &AppState,
    batch_id: Option<i64>,
    targets: &ControlTargets,
    start_kind: &str,
    err: &AppError,
) {
    let error_message = err.message().to_string();
    let reason = format!("{start_kind} start failed before activation: {error_message}");
    if let Err(audit_err) = state
        .db
        .insert_control_event_sqlx(
            batch_id,
            "process_start_failed",
            Some(&SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: reason.clone(),
            }),
            &reason,
        )
        .await
    {
        tracing::warn!("failed to persist process_start_failed audit event: {audit_err}");
    }
}

async fn rollback_v1_auto_start_activation(
    state: &AppState,
    batch_id: Option<i64>,
    targets: &ControlTargets,
) -> Option<String> {
    let Some(batch_id) = batch_id else {
        return None;
    };
    rollback_failed_activation(state, batch_id, Some(targets)).await
}

async fn stop_process_lifecycle(
    state: &AppState,
    expected_process_id: Option<i64>,
    event_type: &'static str,
    operator_reason: Option<String>,
) -> Result<ProcessStopResponse, AppError> {
    let batch_id = {
        let runtime = state.runtime.read().await;
        let Some(batch_id) = runtime.active_batch_id else {
            return Err(AppError::conflict("no active process batch to stop"));
        };
        batch_id
    };

    let batch = state.db.batch_by_id_sqlx(batch_id).await?;
    if batch.is_none() && expected_process_id.is_some() {
        return Err(AppError::not_found("active batch not found"));
    }
    if let Some(process_id) = expected_process_id {
        let Some(batch) = batch.as_ref() else {
            return Err(AppError::not_found("active batch not found"));
        };
        if batch.process_id != Some(process_id) {
            return Err(AppError::conflict(format!(
                "active batch belongs to process {:?}, not process {process_id}",
                batch.process_id
            )));
        }
    }

    let stopped_targets = process_stop_targets(state);
    stop_process_on_device(state, &stopped_targets).await?;
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id != Some(batch_id) {
            runtime.auto_enabled = false;
            runtime.targets = stopped_targets.clone();
            let found_active_batch_id = runtime.active_batch_id;
            runtime.latch_control_fault(format!(
                "process stop active batch changed after stop command; expected {batch_id}, found {:?}; production record was not closed",
                found_active_batch_id
            ));
            return Err(AppError::conflict(
                "active batch changed during process stop; verify field state before retrying",
            ));
        }
        runtime.auto_enabled = false;
        runtime.targets = stopped_targets.clone();
    }
    let batch = if let Some(batch) = batch {
        if batch.finished_at.is_some() {
            batch
        } else {
            if let Err(err) = state.db.finish_batch_sqlx(batch_id).await {
                latch_tail_failure_after_device_action(
                    state,
                    "process stop batch finish",
                    format_error_for_control_fault(&err),
                )
                .await;
                return Err(err.into());
            }
            match state.db.batch_by_id_sqlx(batch_id).await {
                Ok(Some(batch)) => batch,
                Ok(None) => {
                    latch_tail_failure_after_device_action(
                        state,
                        "process stop batch reload",
                        "stopped batch not found after device stop",
                    )
                    .await;
                    return Err(AppError::not_found("stopped batch not found"));
                }
                Err(err) => {
                    latch_tail_failure_after_device_action(
                        state,
                        "process stop batch reload",
                        format_error_for_control_fault(&err),
                    )
                    .await;
                    return Err(err.into());
                }
            }
        }
    } else {
        let recovery_reason = format!(
            "active runtime batch {batch_id} record was missing; risk-reducing stop target was still written and runtime active state was cleared"
        );
        if let Err(err) = state
            .db
            .insert_control_event_sqlx(
                None,
                "process_stop_recovery_missing_batch",
                Some(&SafeCommand {
                    target_temperature_c: stopped_targets.temperature_c,
                    heat_time_s: stopped_targets.heat_time_s,
                    hold_time_s: stopped_targets.hold_time_s,
                    cool_time_s: stopped_targets.cool_time_s,
                    target_stirrer_rpm: stopped_targets.stirrer_rpm,
                    target_shake_speed_cpm: stopped_targets.shake_speed_cpm,
                    target_pressure_mpa: stopped_targets.target_pressure_mpa,
                    reason: recovery_reason.clone(),
                }),
                &recovery_reason,
            )
            .await
        {
            latch_tail_failure_after_device_action(
                state,
                "process stop missing batch recovery audit",
                format_error_for_control_fault(&err),
            )
            .await;
            return Err(err.into());
        }
        {
            let mut runtime = state.runtime.write().await;
            if runtime.active_batch_id == Some(batch_id) {
                runtime.active_batch_id = None;
            }
            runtime.auto_enabled = false;
            runtime.targets = stopped_targets.clone();
        }
        return Ok(ProcessStopResponse {
            stopped_batch_id: batch_id,
            process_id: None,
            batch: None,
            recovery: Some(recovery_reason),
            active_batch_id: None,
            auto_enabled: false,
            stopped_targets,
        });
    };
    let stop_reason = clean_label(operator_reason, stop_process_reason(event_type), 240);
    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            Some(batch_id),
            event_type,
            Some(&SafeCommand {
                target_temperature_c: stopped_targets.temperature_c,
                heat_time_s: stopped_targets.heat_time_s,
                hold_time_s: stopped_targets.hold_time_s,
                cool_time_s: stopped_targets.cool_time_s,
                target_stirrer_rpm: stopped_targets.stirrer_rpm,
                target_shake_speed_cpm: stopped_targets.shake_speed_cpm,
                target_pressure_mpa: stopped_targets.target_pressure_mpa,
                reason: stop_reason.clone(),
            }),
            &stop_reason,
        )
        .await
    {
        latch_tail_failure_after_device_action(
            state,
            "process stop audit",
            format_error_for_control_fault(&err),
        )
        .await;
        return Err(err.into());
    }
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id == Some(batch_id) {
            runtime.active_batch_id = None;
        }
        runtime.auto_enabled = false;
        runtime.targets = stopped_targets.clone();
    }
    Ok(ProcessStopResponse {
        stopped_batch_id: batch_id,
        process_id: batch.process_id,
        batch: Some(batch),
        recovery: None,
        active_batch_id: None,
        auto_enabled: false,
        stopped_targets,
    })
}

async fn ensure_process_can_start(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<(), AppError> {
    if runtime.active_batch_id.is_some() {
        return Err(AppError::conflict(
            "device is busy running an active process batch",
        ));
    }
    ensure_no_unclosed_db_batch_for_new_production(state, runtime).await?;
    ensure_target_update_interlock_clear(state, runtime, TargetUpdateInterlockMode::ProcessStart)
}

async fn ensure_no_unclosed_db_batch_for_new_production(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<(), AppError> {
    ensure_no_unclosed_db_batch_except(state, runtime, None).await
}

async fn ensure_no_unclosed_db_batch_except(
    state: &AppState,
    runtime: &RuntimeState,
    allowed_batch_id: Option<i64>,
) -> Result<(), AppError> {
    let unfinished = state.db.unfinished_batches_sqlx(100).await?;
    let mut expected_runtime = runtime.clone();
    if let Some(allowed_batch_id) = allowed_batch_id {
        if expected_runtime.active_batch_id.is_none() {
            expected_runtime.active_batch_id = Some(allowed_batch_id);
        }
    }
    let batch_status = unfinished_batch_status_from_batches(&unfinished, &expected_runtime);
    if batch_status.recovery_required() {
        return Err(AppError::conflict(format!(
            "unfinished batch recovery must be resolved before starting new production: {}",
            batch_status.reason(&expected_runtime)
        )));
    }
    for batch in unfinished {
        if runtime.active_batch_id != Some(batch.id) && allowed_batch_id != Some(batch.id) {
            return Err(AppError::conflict(format!(
                "database still has unfinished batch {}; close or repair it before starting new production",
                batch.id
            )));
        }
    }
    Ok(())
}

pub(crate) async fn ensure_persisted_batch_state_consistent(
    state: &AppState,
    runtime: &RuntimeState,
    mode: TargetUpdateInterlockMode,
) -> Result<(), AppError> {
    let batch_status = unfinished_batch_status(state, runtime).await?;
    ensure_persisted_batch_status_consistent(&batch_status, runtime, mode)
}

fn ensure_persisted_batch_status_consistent(
    batch_status: &UnfinishedBatchStatus,
    runtime: &RuntimeState,
    mode: TargetUpdateInterlockMode,
) -> Result<(), AppError> {
    if batch_status.recovery_required() {
        return Err(AppError::conflict(format!(
            "{} blocked until unfinished batch recovery is resolved: {}",
            mode.description(),
            batch_status.reason(runtime)
        )));
    }
    Ok(())
}

pub(crate) async fn unfinished_batch_status(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<UnfinishedBatchStatus, AppError> {
    Ok(unfinished_batch_status_from_batches(
        &state.db.unfinished_batches_sqlx(100).await?,
        runtime,
    ))
}

fn unfinished_batch_status_from_batches(
    unfinished: &[Batch],
    runtime: &RuntimeState,
) -> UnfinishedBatchStatus {
    let unfinished_batch_ids = unfinished.iter().map(|batch| batch.id).collect::<Vec<_>>();
    let unexpected_batch_ids = unfinished
        .iter()
        .filter(|batch| runtime.active_batch_id != Some(batch.id))
        .map(|batch| batch.id)
        .collect::<Vec<_>>();
    let runtime_active_batch_missing = runtime
        .active_batch_id
        .is_some_and(|active_id| !unfinished.iter().any(|batch| batch.id == active_id));
    UnfinishedBatchStatus {
        unfinished_batch_ids,
        unexpected_batch_ids,
        runtime_active_batch_missing,
    }
}

/// Reject a risk-increasing commit when a safety latch (emergency stop, manual
/// lock, or control fault) was engaged at any point during the device-write and
/// audit window, even if its boolean flag was cleared again before the final
/// interlock re-check. A risk-increasing action must commit on a field state
/// that stayed continuously safe across the whole audit window, not merely one
/// whose final boolean snapshot happens to look safe. Mirrors the auto-enable
/// guard so process/batch/v1 starts cannot re-arm auto control around it.
fn ensure_safety_latches_unchanged_for_commit(
    runtime: &mut RuntimeState,
    acknowledged_safety_latches: Option<SafetyLatchGenerations>,
    action: &str,
) -> Result<(), AppError> {
    if let Some(acknowledged) = acknowledged_safety_latches {
        if acknowledged.changed_since(runtime) {
            runtime.auto_enabled = false;
            return Err(AppError::conflict(format!(
                "{action} blocked: a safety latch fired during the audit window; field state changed, so the risk-increasing commit was refused. Re-verify field state before retrying"
            )));
        }
    }
    Ok(())
}

async fn commit_process_activation_after_final_interlock(
    state: &AppState,
    batch_id: i64,
    targets: &ControlTargets,
    auto_enabled: bool,
    acknowledged_safety_latches: Option<SafetyLatchGenerations>,
) -> Result<(), AppError> {
    let unfinished = state.db.unfinished_batches_sqlx(100).await?;
    let mut runtime = state.runtime.write().await;
    ensure_target_update_interlock_clear(state, &runtime, TargetUpdateInterlockMode::ProcessStart)?;
    ensure_safety_latches_unchanged_for_commit(
        &mut runtime,
        acknowledged_safety_latches,
        "process activation",
    )?;
    if let Some(active_batch_id) = runtime.active_batch_id {
        return Err(AppError::conflict(format!(
            "device is busy running active batch {active_batch_id}; process start cannot commit activation"
        )));
    }
    let mut activation_runtime = runtime.clone();
    activation_runtime.active_batch_id = Some(batch_id);
    let batch_status = unfinished_batch_status_from_batches(&unfinished, &activation_runtime);
    ensure_persisted_batch_status_consistent(
        &batch_status,
        &activation_runtime,
        TargetUpdateInterlockMode::ProcessStart,
    )?;
    runtime.targets = targets.clone();
    runtime.active_batch_id = Some(batch_id);
    runtime.auto_enabled = auto_enabled;
    Ok(())
}

pub(crate) async fn commit_targets_after_final_interlock(
    state: &AppState,
    targets: &ControlTargets,
    mode: TargetUpdateInterlockMode,
    expected_current: Option<&ControlTargets>,
    acknowledged_safety_latches: Option<SafetyLatchGenerations>,
) -> Result<(), AppError> {
    let unfinished = state.db.unfinished_batches_sqlx(100).await?;
    let mut runtime = state.runtime.write().await;
    ensure_target_update_interlock_clear(state, &runtime, mode)?;
    ensure_safety_latches_unchanged_for_commit(
        &mut runtime,
        acknowledged_safety_latches,
        mode.description(),
    )?;
    let batch_status = unfinished_batch_status_from_batches(&unfinished, &runtime);
    ensure_persisted_batch_status_consistent(&batch_status, &runtime, mode)?;
    if let Some(expected_current) = expected_current {
        if runtime.targets != *expected_current {
            runtime.auto_enabled = false;
            return Err(AppError::conflict(format!(
                "{} saw stale runtime targets; retry after re-reading current targets",
                mode.description()
            )));
        }
    }
    runtime.targets = targets.clone();
    Ok(())
}

async fn commit_component_targets_after_final_interlock(
    state: &AppState,
    targets: &SafeCommand,
    requires_proven_safe_field: bool,
    expected_current: Option<&ControlTargets>,
    acknowledged_safety_latches: Option<SafetyLatchGenerations>,
) -> Result<(), AppError> {
    let unfinished = if requires_proven_safe_field {
        Some(state.db.unfinished_batches_sqlx(100).await?)
    } else {
        None
    };
    let mut runtime = state.runtime.write().await;
    if requires_proven_safe_field {
        ensure_target_update_interlock_clear(
            state,
            &runtime,
            TargetUpdateInterlockMode::ComponentControl,
        )?;
        ensure_safety_latches_unchanged_for_commit(
            &mut runtime,
            acknowledged_safety_latches,
            "component control",
        )?;
        let batch_status = unfinished_batch_status_from_batches(
            unfinished.as_deref().unwrap_or_default(),
            &runtime,
        );
        ensure_persisted_batch_status_consistent(
            &batch_status,
            &runtime,
            TargetUpdateInterlockMode::ComponentControl,
        )?;
        if let Some(expected_current) = expected_current {
            if runtime.targets != *expected_current {
                runtime.auto_enabled = false;
                return Err(AppError::conflict(
                    "component control saw stale runtime targets; retry after re-reading current targets",
                ));
            }
        }
    }
    runtime.targets = ControlTargets {
        temperature_c: targets.target_temperature_c,
        heat_time_s: targets.heat_time_s,
        hold_time_s: targets.hold_time_s,
        cool_time_s: targets.cool_time_s,
        stirrer_rpm: targets.target_stirrer_rpm,
        shake_speed_cpm: targets.target_shake_speed_cpm,
        target_pressure_mpa: targets.target_pressure_mpa,
    };
    Ok(())
}

async fn start_process_on_device(
    state: &AppState,
    targets: &ControlTargets,
    pending_batch_id: Option<i64>,
) -> Result<(), AppError> {
    {
        let runtime = state.runtime.read().await;
        if runtime.active_batch_id.is_some() {
            return Err(AppError::conflict(
                "device is busy running an active process batch",
            ));
        }
        ensure_no_unclosed_db_batch_except(state, &runtime, pending_batch_id).await?;
        ensure_target_update_interlock_clear(
            state,
            &runtime,
            TargetUpdateInterlockMode::ProcessStart,
        )?;
    }
    let command = safe_command_from_runtime_targets(
        targets,
        "process start target write accepted by safety gate",
    );
    match state.device.write_targets(&command).await {
        Ok(()) => Ok(()),
        Err(err) => {
            latch_control_write_fault(state, &err).await;
            Err(AppError::service_unavailable(format!(
                "device process start command failed: {err}"
            )))
        }
    }
}

async fn stop_process_on_device(
    state: &AppState,
    targets: &ControlTargets,
) -> Result<(), AppError> {
    let command = safe_command_from_runtime_targets(targets, "process stop target write");
    match state.device.write_targets(&command).await {
        Ok(()) => Ok(()),
        Err(err) => {
            latch_control_write_fault(state, &err).await;
            Err(AppError::service_unavailable(format!(
                "device process stop command failed: {err}"
            )))
        }
    }
}

async fn latch_control_write_fault(state: &AppState, err: &anyhow::Error) {
    let mut runtime = state.runtime.write().await;
    runtime.latch_control_fault(err.to_string());
}

async fn latch_tail_failure_after_device_action(
    state: &AppState,
    action: &str,
    err: impl AsRef<str>,
) {
    let mut runtime = state.runtime.write().await;
    runtime.latch_control_fault(format!(
        "{action} failed after device action: {}",
        err.as_ref()
    ));
}

fn format_error_for_control_fault(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

fn process_stop_targets(state: &AppState) -> ControlTargets {
    let mut targets = clamp_operator_targets(
        &state.safety,
        ControlTargets {
            temperature_c: state.safety.temperature.min_c,
            heat_time_s: 0.0,
            hold_time_s: 0.0,
            cool_time_s: 0.0,
            stirrer_rpm: state.safety.stirrer.min_rpm,
            shake_speed_cpm: 0.0,
            target_pressure_mpa: 0.0,
        },
    );
    targets.heat_time_s = 0.0;
    targets.hold_time_s = 0.0;
    targets.cool_time_s = 0.0;
    targets.shake_speed_cpm = 0.0;
    targets.target_pressure_mpa = 0.0;
    targets
}

fn stop_process_reason(event_type: &str) -> &'static str {
    if event_type == "ai_process_stopped" {
        "process stopped by AI master control after safety evaluation"
    } else if event_type == "ainas_process_stopped" {
        "process stopped by AINAS remote task"
    } else {
        "process stopped by operator"
    }
}

async fn list_batches(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<BatchListResponse>>, AppError> {
    Ok(Json(success(BatchListResponse {
        batches: state.db.recent_batches_sqlx(100).await?,
        outcomes: state.db.recent_batch_outcomes_sqlx(100).await?,
    })))
}

async fn get_batch_detail(
    State(state): State<AppState>,
    Path(batch_id): Path<i64>,
) -> Result<Json<V1Envelope<BatchDetailResponse>>, AppError> {
    let Some(batch) = state.db.batch_by_id_sqlx(batch_id).await? else {
        return Err(AppError::not_found("batch not found"));
    };
    Ok(Json(success(BatchDetailResponse {
        outcome: state.db.batch_outcome_by_id_sqlx(batch_id).await?,
        samples: state
            .db
            .sample_records_for_batch_sqlx(batch_id, 480)
            .await?,
        events: state
            .db
            .control_events_for_batch_sqlx(batch_id, 100)
            .await?,
        batch,
    })))
}

async fn batches_export_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    require_permission(&headers, Permission::ExportReports)?;
    let batches = state.db.recent_batches_sqlx(10_000).await?;
    let outcomes = state.db.recent_batch_outcomes_sqlx(10_000).await?;
    let csv = build_batches_csv(&batches, &outcomes);
    Ok((
        [
            ("content-type", "text/csv; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"reactor-batches.csv\"",
            ),
        ],
        csv,
    ))
}

async fn batches_export_xlsx(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    require_permission(&headers, Permission::ExportReports)?;
    let batches = state.db.recent_batches_sqlx(10_000).await?;
    let outcomes = state.db.recent_batch_outcomes_sqlx(10_000).await?;
    let workbook = build_batches_xlsx(&batches, &outcomes)?;
    Ok((
        [
            (
                "content-type",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
            (
                "content-disposition",
                "attachment; filename=\"reactor-batches.xlsx\"",
            ),
        ],
        workbook,
    ))
}

async fn batch_report_markdown(
    State(state): State<AppState>,
    Path(batch_id): Path<i64>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    require_permission(&headers, Permission::ExportReports)?;
    let Some(batch) = state.db.batch_by_id_sqlx(batch_id).await? else {
        return Err(AppError::not_found("batch not found"));
    };
    let outcome = state.db.batch_outcome_by_id_sqlx(batch_id).await?;
    let samples = state
        .db
        .sample_records_for_batch_sqlx(batch_id, 10_000)
        .await?;
    let events = state
        .db
        .control_events_for_batch_sqlx(batch_id, 500)
        .await?;
    let report = build_batch_report_markdown(&batch, outcome.as_ref(), &samples, &events);
    Ok((
        [
            ("content-type", "text/markdown; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"reactor-batch-report.md\"",
            ),
        ],
        report,
    ))
}

async fn devices_status(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<DeviceStatusSummary>>, AppError> {
    let runtime = state.runtime.read().await.clone();
    Ok(Json(success(
        device_status_summary_with_db(&state, &runtime).await?,
    )))
}

async fn devices_capabilities(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<DeviceCapabilitiesResponse>>, AppError> {
    let runtime = state.runtime.read().await.clone();
    let summary = device_status_summary_with_db(&state, &runtime).await?;
    let device = &summary.devices[0];
    Ok(Json(success(DeviceCapabilitiesResponse {
        total_count: summary.total_count,
        online_count: summary.online_count,
        devices: vec![DeviceCapabilityDevice {
            device_id: device.device_id.clone(),
            device_role: device.device_role.clone(),
            mode: device_mode_label(&state.device_mode).to_string(),
            online: device.online,
            status: device.status.clone(),
            sensors: device.sensors.clone(),
            components: device.components.clone(),
        }],
    })))
}

async fn control_component(
    State(state): State<AppState>,
    Path((device_id, component_id)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ComponentControlRequest>,
) -> Result<Json<V1Envelope<ComponentControlResponse>>, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    let response = execute_component_control(
        &state,
        &device_id,
        &component_id,
        payload,
        "component_control",
    )
    .await?;
    Ok(Json(success(response)))
}

async fn execute_component_control(
    state: &AppState,
    device_id: &str,
    component_id: &str,
    payload: ComponentControlRequest,
    event_type: &'static str,
) -> Result<ComponentControlResponse, AppError> {
    ensure_reactor_device_id(device_id)?;

    let mut runtime = state.runtime.read().await.clone();
    let capabilities = component_items(&state, &runtime);
    let Some(component) = capabilities
        .iter()
        .find(|component| component.component_id == component_id)
        .cloned()
    else {
        return Err(AppError::not_found("component not found"));
    };
    if !component.controllable {
        return Err(AppError::bad_request("component is not controllable"));
    }
    let Some(action_capability) = component
        .actions
        .iter()
        .find(|action| action.action == payload.action)
        .cloned()
    else {
        return Err(AppError::bad_request("component action is not supported"));
    };
    let requires_proven_safe_field = component_control_requires_proven_safe_field(&payload.action);
    validate_component_control_payload(&action_capability, &payload)?;
    let operator_reason = component_control_reason(&payload, requires_proven_safe_field)?;
    if requires_proven_safe_field {
        runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_target_update_interlock_clear(
            state,
            &runtime,
            TargetUpdateInterlockMode::ComponentControl,
        )?;
        ensure_persisted_batch_state_consistent(
            state,
            &runtime,
            TargetUpdateInterlockMode::ComponentControl,
        )
        .await?;
    }
    if requires_proven_safe_field {
        if let Some(status) = runtime.device_status.as_ref() {
            if let Some(reason) =
                device_status_field_fault_reason(status, state.safety.control.sensor_timeout_ms)
            {
                return Err(AppError::service_unavailable(format!(
                    "device status is not healthy; component control blocked: {reason}"
                )));
            }
        }
    }

    let command = ComponentControlCommand {
        component_id: component_id.to_string(),
        action: payload.action.clone(),
        value: component_control_command_value(&action_capability, &payload),
    };
    let outcome = match state
        .device
        .write_component(&command, &runtime.targets, &state.safety)
        .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            latch_control_write_fault(state, &err).await;
            return Err(err.into());
        }
    };

    let audit_reason = clean_label(
        operator_reason,
        &format!(
            "operator component control {}:{}",
            component_id, payload.action
        ),
        240,
    );
    let audit_command = outcome
        .as_ref()
        .and_then(|outcome| outcome.targets.clone())
        .unwrap_or_else(|| safe_command_from_runtime_targets(&runtime.targets, &audit_reason));
    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            runtime.active_batch_id,
            event_type,
            Some(&audit_command),
            &audit_reason,
        )
        .await
    {
        let mut runtime = state.runtime.write().await;
        runtime.latch_control_fault(format!(
            "component control audit failed after device action: {err}"
        ));
        return Err(err.into());
    }

    if let Some(outcome) = &outcome {
        if let Some(targets) = &outcome.targets {
            if let Err(err) = commit_component_targets_after_final_interlock(
                state,
                targets,
                requires_proven_safe_field,
                requires_proven_safe_field.then_some(&runtime.targets),
                requires_proven_safe_field.then(|| SafetyLatchGenerations::from_runtime(&runtime)),
            )
            .await
            {
                let error = err.message().to_string();
                latch_tail_failure_after_device_action(
                    state,
                    "component control final interlock",
                    error,
                )
                .await;
                return Err(err);
            }
        }
    }

    Ok(ComponentControlResponse {
        device_id: device_id.to_string(),
        component,
        outcome,
    })
}

fn component_control_requires_proven_safe_field(action: &str) -> bool {
    !matches!(action, "stop" | "off")
}

fn component_control_reason(
    payload: &ComponentControlRequest,
    requires_proven_safe_field: bool,
) -> Result<Option<String>, AppError> {
    match &payload.reason {
        Some(Some(reason)) => Ok(Some(reason.clone())),
        Some(None) if requires_proven_safe_field => {
            Err(AppError::bad_request("reason must not be null"))
        }
        Some(None) | None => Ok(None),
    }
}

fn component_control_command_value(
    action: &ComponentActionItem,
    payload: &ComponentControlRequest,
) -> Option<Value> {
    match action.value_type.as_str() {
        "none" if matches!(payload.value.as_ref(), Some(Value::Null)) => None,
        _ => payload.value.clone(),
    }
}

fn validate_component_control_payload(
    action: &ComponentActionItem,
    payload: &ComponentControlRequest,
) -> Result<(), AppError> {
    match action.value_type.as_str() {
        "none" => {
            if component_control_command_value(action, payload).is_some() {
                return Err(AppError::bad_request(format!(
                    "component action {} does not accept a value",
                    action.action
                )));
            }
        }
        "number" => {
            let value = payload.value.as_ref().ok_or_else(|| {
                AppError::bad_request(format!("component action {} requires value", action.action))
            })?;
            let number = value.as_f64().ok_or_else(|| {
                AppError::bad_request(format!(
                    "component action {} value must be a number",
                    action.action
                ))
            })?;
            if !number.is_finite() {
                return Err(AppError::bad_request(format!(
                    "component action {} value must be finite",
                    action.action
                )));
            }
            if let Some(min) = action.min {
                if number < min {
                    return Err(AppError::bad_request(format!(
                        "component action {} value must be >= {min}",
                        action.action
                    )));
                }
            }
            if let Some(max) = action.max {
                if number > max {
                    return Err(AppError::bad_request(format!(
                        "component action {} value must be <= {max}",
                        action.action
                    )));
                }
            }
        }
        value_type => {
            return Err(AppError::bad_request(format!(
                "component action {} has unsupported value type {value_type}",
                action.action
            )));
        }
    }
    Ok(())
}

async fn ai_control(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<AiControlRequest>,
) -> Result<Json<V1Envelope<AiControlResponse>>, AppError> {
    require_permission(&headers, Permission::ApplyAiSuggestion)?;
    let dry_run = optional_present_bool(payload.dry_run, "dry_run")?.unwrap_or(true);
    let allow_process_start =
        optional_present_bool(payload.allow_process_start, "allow_process_start")?.unwrap_or(true);
    let allow_process_stop =
        optional_present_bool(payload.allow_process_stop, "allow_process_stop")?.unwrap_or(true);
    let allow_component_control =
        optional_present_bool(payload.allow_component_control, "allow_component_control")?
            .unwrap_or(true);
    let allow_target_adjustment =
        optional_present_bool(payload.allow_target_adjustment, "allow_target_adjustment")?
            .unwrap_or(true);
    let preferred_process_id =
        optional_present_i64(payload.preferred_process_id, "preferred_process_id")?;
    let requested_intent = payload
        .intent
        .as_deref()
        .or(payload.mode.as_deref())
        .unwrap_or("optimize_and_control")
        .to_string();

    let runtime = state.runtime.read().await.clone();
    if let Err(err) = ensure_fresh_sample(&state, &runtime) {
        if !dry_run {
            disable_auto_control(&state).await;
        }
        return Err(err);
    }
    let safety = ai_control_safety(&state, &runtime).await?;
    if safety.emergency_stop {
        return Err(AppError::conflict(
            "emergency stop is active; AI master control blocked",
        ));
    }
    if safety.manual_lock {
        return Err(AppError::conflict(
            "manual lock is active; AI master control blocked",
        ));
    }
    if !safety.device_online && !dry_run {
        disable_auto_control(&state).await;
        return Err(AppError::service_unavailable(
            "device is offline or unhealthy; AI master control blocked",
        ));
    }
    let process_start_block_reason = if allow_process_start && runtime.active_batch_id.is_none() {
        if safety.batch_recovery_required {
            safety.unfinished_batch_ids.first().map(|batch_id| {
                format!(
                    "database still has unfinished batch {batch_id}; AI process start blocked until production state is closed"
                )
            }).or_else(|| Some(
                "runtime active batch is not backed by an unfinished database record; AI process start blocked until production state is repaired"
                    .to_string(),
            ))
        } else if !safety.device_online {
            Some("device is offline or unhealthy; AI process start blocked".to_string())
        } else if safety.high_alarm_count > 0 {
            Some("high level alarm is active; AI process start blocked".to_string())
        } else if let Some(batch) = state.db.unfinished_batches_sqlx(100).await?.first() {
            Some(format!(
                "database still has unfinished batch {}; AI process start blocked until production state is closed",
                batch.id
            ))
        } else {
            None
        }
    } else {
        None
    };

    let recommendation = match state
        .db
        .latest_recommendation_sqlx()
        .await?
        .filter(|recommendation| provider_allows_recommendation(&state, recommendation))
        .filter(|recommendation| recommendation.based_on_batch_count > 0)
    {
        Some(recommendation) => Some(recommendation),
        None if allow_target_adjustment => {
            let recommendation = generate_recommendation(&state).await?;
            if !dry_run {
                state
                    .db
                    .insert_recommendation_with_audit_sqlx(
                        &recommendation,
                        "ai_master_recommendation_generated",
                        "AI master control generated a recommendation before applying targets",
                    )
                    .await?;
            }
            Some(recommendation)
        }
        None => None,
    };
    let recommended_targets = recommendation
        .as_ref()
        .map(|recommendation| ai_targets_from_recommendation(&state, &runtime, recommendation))
        .transpose()?;
    let mut actions = Vec::new();
    let mut decision = "hold".to_string();
    let mut rationale = format!(
        "AI master-control intent '{requested_intent}' evaluated with live pipeline data and safety interlocks"
    );

    if let Some(targets) = recommended_targets.as_ref() {
        if allow_target_adjustment && targets_differ(&runtime.targets, targets) {
            decision = "adjust_targets".to_string();
            actions.push(AiControlAction {
                action_type: "target_adjustment".to_string(),
                target: "/api/v1/reactor/reactor_001/control".to_string(),
                status: if dry_run { "planned" } else { "executed" }.to_string(),
                message: "apply latest AI batch recommendation to safe runtime targets".to_string(),
                result: None,
            });
            if !dry_run {
                apply_ai_targets(
                    &state,
                    targets.clone(),
                    "AI master control adjusted targets",
                )
                .await?;
                actions.last_mut().expect("target action exists").result =
                    Some(json!({ "targets": targets }));
            }
        }
    } else if allow_target_adjustment {
        actions.push(AiControlAction {
            action_type: "target_adjustment".to_string(),
            target: "/api/recommendations/latest".to_string(),
            status: "skipped".to_string(),
            message: "no persisted recommendation yet; waiting for finished batch outcomes"
                .to_string(),
            result: None,
        });
    }

    let selected_process = select_ai_process(&state, preferred_process_id).await?;
    if runtime.active_batch_id.is_none()
        && allow_process_start
        && process_start_block_reason.is_none()
    {
        if let Some(process) = selected_process {
            decision = if decision == "adjust_targets" {
                "adjust_targets_and_start_process".to_string()
            } else {
                "start_process".to_string()
            };
            actions.push(AiControlAction {
                action_type: "process_start".to_string(),
                target: format!("/api/processes/{}/start", process.id),
                status: if dry_run { "planned" } else { "executed" }.to_string(),
                message: format!("AI selected runnable process '{}'", process.name),
                result: None,
            });
            if !dry_run {
                let started =
                    start_process_lifecycle(&state, process.id, "ai_process_started").await?;
                actions.last_mut().expect("process action exists").result =
                    Some(serde_json::to_value(started).map_err(anyhow::Error::from)?);
            }
        } else {
            actions.push(AiControlAction {
                action_type: "process_start".to_string(),
                target: "/api/processes/:id/start".to_string(),
                status: "skipped".to_string(),
                message: "no runnable process definition with at least one step".to_string(),
                result: None,
            });
        }
    } else if let Some(reason) = process_start_block_reason {
        actions.push(AiControlAction {
            action_type: "process_start".to_string(),
            target: "/api/processes/:id/start".to_string(),
            status: "blocked".to_string(),
            message: reason,
            result: None,
        });
    }

    let runtime_after_process = if dry_run {
        let mut planned = runtime.clone();
        if actions
            .iter()
            .any(|action| action.action_type == "process_start" && action.status == "planned")
        {
            planned.active_batch_id = Some(-1);
        }
        planned
    } else {
        state.runtime.read().await.clone()
    };
    if runtime_after_process.active_batch_id.is_some() {
        let sample = runtime_after_process.latest_sample.as_ref();
        if allow_process_stop && should_ai_stop_process(sample, &safety) {
            decision = "stop_process".to_string();
            actions.push(AiControlAction {
                action_type: "process_stop".to_string(),
                target: "/api/processes/current/stop".to_string(),
                status: if dry_run { "planned" } else { "executed" }.to_string(),
                message: "AI stop condition met from live concentration or alarm state".to_string(),
                result: None,
            });
            if !dry_run {
                let stopped =
                    stop_process_lifecycle(&state, None, "ai_process_stopped", None).await?;
                actions.last_mut().expect("stop action exists").result =
                    Some(serde_json::to_value(stopped).map_err(anyhow::Error::from)?);
            }
        } else if allow_component_control {
            let component_runtime = state.runtime.read().await.clone();
            if let Some(action) = plan_shake_component_action(
                &state,
                &component_runtime,
                recommended_targets.as_ref(),
            ) {
                if decision == "hold" {
                    decision = "control_shake_vessel".to_string();
                } else if !decision.contains("shake") {
                    decision.push_str("_and_control_shake");
                }
                actions.push(AiControlAction {
                    action_type: "component_control".to_string(),
                    target: format!(
                        "/api/devices/reactor_001/components/shake_stepper/control:{}",
                        action.action
                    ),
                    status: if dry_run { "planned" } else { "executed" }.to_string(),
                    message: "AI controls shake vessel stepper through component safety gate"
                        .to_string(),
                    result: None,
                });
                if !dry_run {
                    let component = execute_component_control(
                        &state,
                        "reactor_001",
                        "shake_stepper",
                        action,
                        "ai_component_control",
                    )
                    .await?;
                    actions.last_mut().expect("component action exists").result =
                        Some(serde_json::to_value(component).map_err(anyhow::Error::from)?);
                }
            }
        }
    }

    if actions.iter().any(|action| action.status == "blocked") {
        decision = "hold".to_string();
    }

    if actions.is_empty() {
        rationale.push_str("; no control action was necessary");
        actions.push(AiControlAction {
            action_type: "hold".to_string(),
            target: "safety_supervisor".to_string(),
            status: "skipped".to_string(),
            message:
                "system is already within AI target envelope or no allowed action is available"
                    .to_string(),
            result: None,
        });
    }

    if !dry_run {
        let audit_command = recommended_targets
            .as_ref()
            .map(|targets| safe_command_from_runtime_targets(targets, "AI master decision"));
        let audit_reason = ai_control_audit_reason(&decision, &rationale, &actions)?;
        let active_batch_id = state.runtime.read().await.active_batch_id;
        if let Err(err) = state
            .db
            .insert_control_event_sqlx(
                active_batch_id,
                "ai_master_decision",
                audit_command.as_ref(),
                &audit_reason,
            )
            .await
        {
            if ai_actions_include_executed_device_action(&actions) {
                latch_tail_failure_after_device_action(
                    &state,
                    "AI master decision audit",
                    format_error_for_control_fault(&err),
                )
                .await;
            }
            return Err(err.into());
        }
    }

    Ok(Json(success(AiControlResponse {
        mode: "ai_master_control".to_string(),
        dry_run,
        decision,
        rationale,
        recommended_targets,
        safety,
        actions,
    })))
}

fn ai_actions_include_executed_device_action(actions: &[AiControlAction]) -> bool {
    actions.iter().any(|action| {
        action.status == "executed"
            && matches!(
                action.action_type.as_str(),
                "target_adjustment" | "process_start" | "process_stop" | "component_control"
            )
    })
}

fn ai_control_audit_reason(
    decision: &str,
    rationale: &str,
    actions: &[AiControlAction],
) -> Result<String, AppError> {
    let reason = AiControlAuditReason {
        decision: clean_label(Some(decision.to_string()), "hold", 120),
        rationale: clean_label(
            Some(rationale.to_string()),
            "AI master control decision",
            480,
        ),
        actions: actions
            .iter()
            .map(|action| AiControlAuditAction {
                action_type: clean_label(Some(action.action_type.clone()), "unknown", 80),
                target: clean_label(Some(action.target.clone()), "unknown", 160),
                status: clean_label(Some(action.status.clone()), "unknown", 40),
            })
            .collect(),
    };
    serde_json::to_string(&reason)
        .map_err(anyhow::Error::from)
        .map_err(AppError::from)
}

async fn ai_experiment_plan(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<ExperimentPlanResponse>>, AppError> {
    ensure_production_basis_write_allowed(&state, "AI experiment plan generation").await?;
    let recommendation = generate_recommendation(&state).await?;
    state
        .db
        .insert_recommendation_with_audit_sqlx(
            &recommendation,
            "ai_experiment_plan_recommendation_generated",
            "AI experiment plan generated a recommendation draft for operator review",
        )
        .await?;
    let plan = build_experiment_plan(&state, recommendation).await?;
    Ok(Json(success(plan)))
}

async fn build_experiment_plan(
    state: &AppState,
    recommendation: Recommendation,
) -> Result<ExperimentPlanResponse, AppError> {
    let runtime = state.runtime.read().await.clone();
    let targets = ai_targets_from_recommendation(state, &runtime, &recommendation)?;
    let local_ai = LocalAiStatus::from_env();
    let recent_outcomes = state.db.recent_batch_outcomes_sqlx(5).await?;
    let best_outcome = recent_outcomes.iter().max_by(|a, b| {
        ((a.yield_percent * 0.8) + (a.product_ratio * 100.0 * 0.2))
            .total_cmp(&((b.yield_percent * 0.8) + (b.product_ratio * 100.0 * 0.2)))
    });
    let target_pressure_mpa = round2(targets.target_pressure_mpa.max(0.0));
    let shake_speed_cpm = round2(targets.shake_speed_cpm);
    let heat_minutes = round2((targets.heat_time_s / 60.0).max(1.0));
    let hold_minutes = round2((targets.hold_time_s / 60.0).max(1.0));
    let cool_minutes = round2((targets.cool_time_s / 60.0).max(1.0));
    let heat_temp = round2(targets.temperature_c);
    let hold_temp = heat_temp;
    let cool_temp = round2((heat_temp - 20.0).max(state.safety.temperature.min_c));
    let rpm = round2(targets.stirrer_rpm);

    let steps = vec![
        ExperimentPlanStep {
            step_no: 1,
            name: "Pre-check and heat-up".to_string(),
            target_temperature_c: heat_temp,
            target_stirrer_rpm: rpm,
            target_shake_speed_cpm: shake_speed_cpm,
            target_pressure_mpa,
            duration_minutes: heat_minutes,
            operator_action: "Verify vessel charge, sensor freshness, coolant flow, and clear interlocks before starting the ramp.".to_string(),
            safety_check: format!(
                "Temperature must remain within {:.1}-{:.1} degC and each automatic adjustment must respect max_step_c {:.1}.",
                state.safety.temperature.min_c,
                state.safety.temperature.max_c,
                state.safety.temperature.max_step_c
            ),
        },
        ExperimentPlanStep {
            step_no: 2,
            name: "Reaction hold and sampling".to_string(),
            target_temperature_c: hold_temp,
            target_stirrer_rpm: rpm,
            target_shake_speed_cpm: shake_speed_cpm,
            target_pressure_mpa,
            duration_minutes: hold_minutes,
            operator_action: "Hold the recommended targets, collect sample checkpoints, and record yield/product ratio after completion.".to_string(),
            safety_check: format!(
                "Stirrer target must remain <= {:.1} RPM; pressure target is safety-clamped before device write.",
                state.safety.stirrer.max_rpm
            ),
        },
        ExperimentPlanStep {
            step_no: 3,
            name: "Cool-down and result capture".to_string(),
            target_temperature_c: cool_temp,
            target_stirrer_rpm: round2((rpm * 0.5).max(state.safety.stirrer.min_rpm)),
            target_shake_speed_cpm: 0.0,
            target_pressure_mpa,
            duration_minutes: cool_minutes,
            operator_action: "Stop active heating, cool under observation, finish the batch, and save product result data for the next recommendation cycle.".to_string(),
            safety_check: "Emergency stop, manual lock, stale sensor data, and high alarms must block any automatic execution.".to_string(),
        },
    ];

    let source = if state.ai_provider.is_some() {
        "StepFun/local safety fallback"
    } else {
        "local optimizer with safety memory"
    };
    let best_note = best_outcome
        .map(|outcome| {
            format!(
                " Best recent batch #{} reached {:.1}% yield and {:.2} product ratio.",
                outcome.batch_id, outcome.yield_percent, outcome.product_ratio
            )
        })
        .unwrap_or_else(|| " No finished product-result batch is available beyond the current recommendation context.".to_string());
    let lora_note = if local_ai.ready_for_prd_lora {
        "Local Qwen LoRA/RK evidence boundary is complete.".to_string()
    } else if local_ai.ready_for_lora_inference && local_ai.ready_for_training {
        "Local Qwen LoRA inference/training boundary is present, but RK validation is still missing.".to_string()
    } else if local_ai.ready_for_base_inference {
        "Local base-model inference is configured, but LoRA adapter/training/RK evidence is still missing.".to_string()
    } else {
        format!(
            "Local Qwen LoRA is not executable yet; missing assets: {}.",
            if local_ai.missing.is_empty() {
                "daemon training/inference API".to_string()
            } else {
                local_ai.missing.join(", ")
            }
        )
    };

    Ok(ExperimentPlanResponse {
        plan_id: format!("xingshu-plan-{}", Utc::now().format("%Y%m%d%H%M%S")),
        title: "Safety-gated AI experiment plan draft".to_string(),
        status: "draft_requires_operator_review".to_string(),
        source: source.to_string(),
        objective: "Explore the next safe parameter point while preserving auditability and requiring operator approval before execution.".to_string(),
        sop_summary: format!(
            "Run a three-stage heat/hold/cool experiment at {:.1} degC and {:.1} RPM based on {} recorded product-result batches.{}",
            heat_temp, rpm, recommendation.based_on_batch_count, best_note
        ),
        recommendation,
        steps,
        acceptance_criteria: vec![
            "All control targets remain inside configured safety bounds after clamping.".to_string(),
            "Operator records yield_percent and product_ratio before a new recommendation is considered valid.".to_string(),
            "No emergency stop, manual lock, stale sensor, or high alarm is active during execution.".to_string(),
            "Batch report and audit events can be exported after completion.".to_string(),
        ],
        safety_notes: vec![
            "This endpoint only drafts an SOP; it does not start a process or write hardware targets.".to_string(),
            "Execution must use /api/ai/control dry-run first, then an operator-confirmed process start or target write.".to_string(),
            "All later writes still pass through RBAC, safety clamp, audit logging, and the optional independent safety guard.".to_string(),
        ],
        model_boundary: vec![
            lora_note,
            "The draft uses local optimizer evidence unless a configured cloud provider produces a fresh recommendation.".to_string(),
            "It is not proof of PRD local LoRA self-evolution until Qwen/GGUF/LoRA/training/RK assets are supplied and validated.".to_string(),
        ],
        next_actions: vec![
            "Review the draft SOP against the actual vessel charge and hardware readiness.".to_string(),
            "Run AI master-control preview before applying any target.".to_string(),
            "After the batch, save product result data to close the learning loop.".to_string(),
        ],
    })
}

async fn ai_control_safety(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<AiControlSafety, AppError> {
    let batch_status = unfinished_batch_status(state, runtime).await?;
    let alarms = live_alarms_for(
        state,
        state.safety.as_ref(),
        runtime,
        runtime.latest_sample.as_ref(),
        state.ai_memory.as_ref(),
    )
    .await?;
    let high_alarm_count = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("high"))
        .count();
    let warning_alarm_count = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("medium"))
        .count();
    let device_status = device_status_summary_with_db(state, runtime).await?;
    Ok(AiControlSafety {
        fresh_sample_required: true,
        sensor_fresh: ensure_fresh_sample(state, runtime).is_ok(),
        emergency_stop: runtime.emergency_stop,
        manual_lock: runtime.manual_lock,
        device_online: device_status.online_count > 0,
        active_batch_id: runtime.active_batch_id,
        batch_recovery_required: batch_status.recovery_required(),
        unfinished_batch_ids: batch_status.unfinished_batch_ids,
        unexpected_batch_ids: batch_status.unexpected_batch_ids,
        high_alarm_count,
        warning_alarm_count,
        stop_product_concentration_percent: state
            .safety
            .control
            .ai_stop_product_concentration_percent,
    })
}

fn ai_targets_from_recommendation(
    state: &AppState,
    runtime: &RuntimeState,
    recommendation: &Recommendation,
) -> Result<ControlTargets, AppError> {
    validate_ai_recommendation_targets(
        state,
        ControlTargets {
            temperature_c: recommendation.target_temperature_c,
            heat_time_s: recommendation.heating_minutes * 60.0,
            hold_time_s: recommendation.stirring_minutes * 60.0,
            cool_time_s: runtime.targets.cool_time_s,
            stirrer_rpm: recommendation.target_stirrer_rpm,
            shake_speed_cpm: runtime.targets.shake_speed_cpm,
            target_pressure_mpa: runtime.targets.target_pressure_mpa,
        },
    )
}

fn validate_ai_recommendation_targets(
    state: &AppState,
    targets: ControlTargets,
) -> Result<ControlTargets, AppError> {
    validate_target_temperature(&state.safety, targets.temperature_c)
        .map_err(|err| err.with_message_prefix("ai_target_temperature_c"))?;
    validate_stir_speed(&state.safety, targets.stirrer_rpm)
        .map_err(|err| err.with_message_prefix("ai_target_stirrer_rpm"))?;
    validate_range(
        "ai_heat_time_s",
        targets.heat_time_s,
        0.0,
        state.safety.optimizer.max_heating_minutes * 60.0,
    )?;
    validate_range(
        "ai_hold_time_s",
        targets.hold_time_s,
        0.0,
        state.safety.optimizer.max_stirring_minutes * 60.0,
    )?;
    validate_range("ai_cool_time_s", targets.cool_time_s, 0.0, 3600.0)?;
    validate_range("ai_shake_speed_cpm", targets.shake_speed_cpm, 0.0, 60.0)?;
    validate_range(
        "target_pressure_mpa",
        targets.target_pressure_mpa,
        0.0,
        10.0,
    )?;
    Ok(ControlTargets {
        temperature_c: round2(targets.temperature_c),
        heat_time_s: round2(targets.heat_time_s),
        hold_time_s: round2(targets.hold_time_s),
        cool_time_s: round2(targets.cool_time_s),
        stirrer_rpm: round2(targets.stirrer_rpm),
        shake_speed_cpm: round2(targets.shake_speed_cpm),
        target_pressure_mpa: round2(targets.target_pressure_mpa),
    })
}

fn targets_differ(current: &ControlTargets, next: &ControlTargets) -> bool {
    (current.temperature_c - next.temperature_c).abs() > 0.01
        || (current.heat_time_s - next.heat_time_s).abs() > 0.01
        || (current.hold_time_s - next.hold_time_s).abs() > 0.01
        || (current.cool_time_s - next.cool_time_s).abs() > 0.01
        || (current.stirrer_rpm - next.stirrer_rpm).abs() > 0.01
        || (current.shake_speed_cpm - next.shake_speed_cpm).abs() > 0.01
        || (current.target_pressure_mpa - next.target_pressure_mpa).abs() > 0.01
}

async fn apply_ai_targets(
    state: &AppState,
    targets: ControlTargets,
    reason: &str,
) -> Result<(), AppError> {
    let expected_current = state.runtime.read().await.targets.clone();
    let acknowledged_safety_latches = {
        let mut runtime = state.runtime.write().await;
        runtime.auto_enabled = false;
        ensure_target_update_interlock_clear(
            &state,
            &runtime,
            TargetUpdateInterlockMode::DesiredTargets,
        )?;
        SafetyLatchGenerations::from_runtime(&runtime)
    };
    ensure_targets_allowed(&state.safety, &targets)?;
    let command = safe_command_from_runtime_targets(&targets, reason);
    state
        .db
        .insert_control_event_sqlx(None, "ai_targets_updated", Some(&command), reason)
        .await?;
    if let Err(err) = state.device.write_targets(&command).await {
        latch_control_write_fault(state, &err).await;
        return Err(AppError::service_unavailable(format!(
            "AI target write to device failed: {err}"
        )));
    }
    if let Err(err) = commit_targets_after_final_interlock(
        state,
        &targets,
        TargetUpdateInterlockMode::DesiredTargets,
        Some(&expected_current),
        Some(acknowledged_safety_latches),
    )
    .await
    {
        latch_tail_failure_after_device_action(
            state,
            "AI target final interlock",
            err.message().to_string(),
        )
        .await;
        return Err(err);
    }
    Ok(())
}

async fn disable_auto_control(state: &AppState) {
    state.runtime.write().await.auto_enabled = false;
}

async fn select_ai_process(
    state: &AppState,
    preferred_process_id: Option<i64>,
) -> Result<Option<ProcessDefinition>, AppError> {
    if let Some(process_id) = preferred_process_id {
        let Some(detail) = process_detail_or_bad_request(state, process_id).await? else {
            return Err(AppError::not_found("preferred process not found"));
        };
        if detail.steps.is_empty() {
            return Err(AppError::bad_request(
                "preferred process must contain at least one step",
            ));
        }
        return Ok(Some(detail.process));
    }
    let processes = state.db.list_processes_sqlx().await?;
    if let Some(process) = processes
        .iter()
        .find(|process| process.step_count > 0 && process.status == "applied")
        .cloned()
    {
        return Ok(Some(process));
    }
    Ok(processes.into_iter().find(|process| process.step_count > 0))
}

async fn process_detail_or_bad_request(
    state: &AppState,
    process_id: i64,
) -> Result<Option<ProcessDetail>, AppError> {
    state
        .db
        .process_detail_sqlx(process_id)
        .await
        .map_err(|err| {
            let message = format!("{err:#}");
            if message.contains("invalid process step in database") {
                AppError::bad_request(message)
            } else {
                AppError::from(err)
            }
        })
}

fn should_ai_stop_process(sample: Option<&SensorSnapshot>, safety: &AiControlSafety) -> bool {
    sample
        .map(|sample| {
            sample.product_concentration_percent >= safety.stop_product_concentration_percent
        })
        .unwrap_or(false)
        || (safety.high_alarm_count > 0 && safety.active_batch_id.is_some())
}

fn plan_shake_component_action(
    state: &AppState,
    runtime: &RuntimeState,
    recommended_targets: Option<&ControlTargets>,
) -> Option<ComponentControlRequest> {
    let components = component_items(state, runtime);
    if !components
        .iter()
        .any(|component| component.component_id == "shake_stepper" && component.controllable)
    {
        return None;
    }
    let target_shake_speed = recommended_targets
        .map(|targets| targets.shake_speed_cpm)
        .unwrap_or(runtime.targets.shake_speed_cpm);
    let motor_running = runtime
        .device_status
        .as_ref()
        .and_then(|status| status.motor)
        .map(|motor| motor == 1)
        .unwrap_or_else(|| {
            runtime
                .latest_sample
                .as_ref()
                .map(|sample| sample.shake_speed_cpm > 0.01)
                .unwrap_or(false)
        });
    if target_shake_speed > 0.01 && !motor_running {
        Some(ComponentControlRequest {
            action: "start".to_string(),
            value: None,
            reason: Some(Some(
                "AI master control started shake vessel stepper".to_string(),
            )),
        })
    } else if target_shake_speed <= 0.01 && motor_running {
        Some(ComponentControlRequest {
            action: "stop".to_string(),
            value: None,
            reason: Some(Some(
                "AI master control stopped shake vessel stepper".to_string(),
            )),
        })
    } else {
        None
    }
}

async fn v1_control(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<V1ControlRequest>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    ensure_reactor_device_id(&device_id)?;
    require_permission(&headers, Permission::SetSafeTargets)?;
    let auto_start = optional_present_bool(payload.auto_start, "auto_start")?.unwrap_or(false);
    let targets = validate_v1_control_params(&state.safety, &payload.params, auto_start)?;
    if auto_start {
        let runtime = state.runtime.read().await.clone();
        if runtime.active_batch_id.is_some() {
            return Err(AppError::conflict("device is busy running an active batch"));
        }
        ensure_no_unclosed_db_batch_for_new_production(&state, &runtime).await?;
    }
    let acknowledged_safety_latches = {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_target_update_interlock_clear(
            &state,
            &runtime,
            TargetUpdateInterlockMode::DesiredTargets,
        )?;
        SafetyLatchGenerations::from_runtime(&runtime)
    };

    let heating_minutes = seconds_to_minutes(Some(targets.heat_time_s));
    let stirring_minutes = seconds_to_minutes(Some(targets.hold_time_s));

    let mut batch_id = None;
    if auto_start {
        let batch = state.db.create_batch(
            &format!(
                "{}:{}",
                device_id,
                payload.command_id.as_deref().unwrap_or("process")
            ),
            targets.temperature_c,
            targets.stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        )?;
        batch_id = Some(batch.id);
    }

    if auto_start {
        if let Err(err) = start_process_on_device(&state, &targets, batch_id).await {
            audit_start_failed_before_activation(&state, batch_id, &targets, "v1 control", &err)
                .await;
            // The device start write failed, so the field was never commanded on and
            // runtime was never armed with this batch. Mirror batch start: just close
            // the pending batch record. Do NOT call the post-activation rollback, which
            // re-issues a stop write and conservatively re-arms active_batch_id when that
            // stop also fails -- that is only correct after a successful device start.
            if let Some(batch_id) = batch_id {
                if let Err(finish_err) = state.db.finish_batch_sqlx(batch_id).await {
                    tracing::warn!(
                        "failed to mark failed v1 auto_start batch finished: {finish_err}"
                    );
                }
            }
            return Err(err);
        }
    }

    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            batch_id,
            "v1_control_accepted",
            Some(&SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: "v1 control request accepted after document range validation".to_string(),
            }),
            "v1 control request accepted after document range validation",
        )
        .await
    {
        let rollback_stop_error =
            rollback_v1_auto_start_activation(&state, batch_id, &targets).await;
        if auto_start {
            latch_tail_failure_after_device_action(
                &state,
                "v1 auto_start audit",
                tail_error_with_activation_rollback(
                    format_error_for_control_fault(&err),
                    rollback_stop_error,
                ),
            )
            .await;
        }
        return Err(err.into());
    }
    let commit_result = if let Some(batch_id) = batch_id {
        commit_process_activation_after_final_interlock(
            &state,
            batch_id,
            &targets,
            auto_start,
            Some(acknowledged_safety_latches),
        )
        .await
    } else {
        commit_targets_after_final_interlock(
            &state,
            &targets,
            TargetUpdateInterlockMode::DesiredTargets,
            None,
            Some(acknowledged_safety_latches),
        )
        .await
    };
    if let Err(err) = commit_result {
        if auto_start {
            if let Some(batch_id) = batch_id {
                let rollback_stop_error =
                    rollback_failed_activation(&state, batch_id, Some(&targets)).await;
                latch_tail_failure_after_device_action(
                    &state,
                    "v1 auto_start final interlock",
                    tail_error_with_activation_rollback(
                        err.message().to_string(),
                        rollback_stop_error,
                    ),
                )
                .await;
            } else {
                latch_tail_failure_after_device_action(
                    &state,
                    "v1 auto_start final interlock",
                    err.message().to_string(),
                )
                .await;
            }
        }
        return Err(err);
    }

    let estimated_duration =
        (targets.heat_time_s + targets.hold_time_s + targets.cool_time_s).round() as i64;
    Ok(Json(success(json!({
        "command_id": payload.command_id.unwrap_or_else(|| format!("cmd_{}", chrono::Utc::now().timestamp_millis())),
        "status": "accepted",
        "estimated_duration": estimated_duration
    }))))
}

async fn v1_pipeline_sample(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    ensure_reactor_device_id(&device_id)?;
    require_permission(&headers, Permission::IngestSensorSample)?;
    let payload = parse_pipeline_sample_payload(&state, body).await?;
    let sample = accept_pipeline_sample(&state, payload).await?;
    Ok(Json(success(json!({
        "device_id": device_id,
        "timestamp": sample.captured_at.to_rfc3339(),
        "sample": sample
    }))))
}

async fn v1_realtime(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    ensure_reactor_device_id(&device_id)?;
    require_permission(&headers, Permission::ViewMonitor)?;
    let runtime = state.runtime.read().await.clone();
    Ok(Json(
        v1_realtime_payload(&state, &device_id, &runtime).await?,
    ))
}

async fn v1_realtime_ws(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    ensure_reactor_device_id(&device_id)?;
    require_permission(&headers, Permission::ViewMonitor)?;
    Ok(ws.on_upgrade(move |socket| v1_realtime_socket(socket, state, device_id)))
}

async fn v1_realtime_socket(mut socket: WebSocket, state: AppState, device_id: String) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let runtime = state.runtime.read().await.clone();
        let payload = match v1_realtime_payload(&state, &device_id, &runtime).await {
            Ok(payload) => payload,
            Err(err) => {
                let Ok(text) = serde_json::to_string(&err.to_envelope()) else {
                    break;
                };
                let _ = socket.send(Message::Text(text)).await;
                break;
            }
        };
        let Ok(text) = serde_json::to_string(&payload) else {
            break;
        };
        if socket.send(Message::Text(text)).await.is_err() {
            break;
        }
    }
}

async fn v1_realtime_payload(
    state: &AppState,
    device_id: &str,
    runtime: &RuntimeState,
) -> Result<Value, AppError> {
    let sample = fresh_sample_for_realtime(state, runtime)?;
    let device_summary = device_status_summary_with_db(state, runtime).await?;
    let device = &device_summary.devices[0];
    let alarms = live_alarms_for(
        state,
        state.safety.as_ref(),
        runtime,
        Some(sample),
        state.ai_memory.as_ref(),
    )
    .await?;
    Ok(json!({
        "device_id": device_id,
        "timestamp": sample.captured_at.to_rfc3339(),
        "status": device.status,
        "device_online": device.online,
        "device_status": device,
        "data": {
            "current_temp": sample.temperature_c,
            "current_pressure": sample.pressure_mpa,
            "stir_speed": sample.stirrer_rpm,
            "shake_speed": sample.shake_speed_cpm,
            "tilt_state": sample.tilt_state,
            "tilt_angle": sample.tilt_angle_deg,
            "tilt_angle_source": "software_fit_from_binary_sensor",
            "flow_rate": sample.flow_rate_l_min,
            "phase": phase_for(&runtime, device.online),
            "progress": progress_for(Some(sample))
        },
        "alarms": alarms
    }))
}

async fn v1_history(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Query(query): Query<V1HistoryQuery>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    ensure_reactor_device_id(&device_id)?;
    let start_time = parse_required_time(query.start_time.as_deref(), "start_time")?;
    let end_time = parse_required_time(query.end_time.as_deref(), "end_time")?;
    if end_time < start_time {
        return Err(AppError::bad_request(
            "end_time must be greater than or equal to start_time",
        ));
    }
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(100).clamp(1, 500);
    let offset = (page - 1) * page_size;
    let samples = state
        .db
        .samples_between_sqlx(start_time, end_time, page_size, offset)
        .await?;
    let rows: Vec<Value> = samples
        .into_iter()
        .map(|record| {
            let sample = record.sample;
            json!({
                "device_id": device_id,
                "batch_id": record.batch_id,
                "timestamp": sample.captured_at.to_rfc3339(),
                "data": {
                    "current_temp": sample.temperature_c,
                    "current_pressure": sample.pressure_mpa,
                    "stir_speed": sample.stirrer_rpm,
                    "shake_speed": sample.shake_speed_cpm,
                    "tilt_state": sample.tilt_state,
                    "tilt_angle": sample.tilt_angle_deg,
                    "tilt_angle_source": "software_fit_from_binary_sensor",
                    "flow_rate": sample.flow_rate_l_min,
                    "product_concentration": sample.product_concentration_percent,
                    "ph": sample.ph
                }
            })
        })
        .collect();

    Ok(Json(success(json!({
        "device_id": device_id,
        "page": page,
        "page_size": page_size,
        "interval": query.interval.unwrap_or_else(|| "raw".to_string()),
        "start_time": start_time.to_rfc3339(),
        "end_time": end_time.to_rfc3339(),
        "items": rows.clone(),
        "records": rows
    }))))
}

async fn v1_process(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<V1ProcessRequest>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    ensure_reactor_device_id(&device_id)?;
    require_permission(&headers, Permission::EditProcess)?;
    if payload.phases.is_empty() {
        return Err(AppError::bad_request("phases must not be empty"));
    }
    let mut target_temperature = None;
    let mut stirrer_rpm = None;
    let mut shake_speed_cpm = None;
    let mut target_pressure_mpa = None;
    let mut heat_time_s = None;
    let mut hold_time_s = None;
    let mut cool_time_s = None;
    let mut total_seconds = 0.0;
    let mut saw_control_param = false;
    for phase in &payload.phases {
        let phase_name = phase.phase.trim();
        if phase_name.is_empty() {
            return Err(AppError::bad_request("phase must not be blank"));
        }
        if !phase.params.is_object() {
            return Err(AppError::bad_request("phase params must be an object"));
        }
        if let Some(duration) = optional_number_field(&phase.params, "duration")? {
            saw_control_param = true;
            match phase_name {
                "heating" => {
                    validate_range(
                        "heating duration",
                        duration,
                        0.0,
                        state.safety.optimizer.max_heating_minutes * 60.0,
                    )?;
                    heat_time_s = Some(duration);
                }
                "holding" => {
                    validate_range(
                        "holding duration",
                        duration,
                        0.0,
                        state.safety.optimizer.max_stirring_minutes * 60.0,
                    )?;
                    hold_time_s = Some(duration);
                }
                "cooling" => {
                    validate_range("cooling duration", duration, 0.0, 3600.0)?;
                    cool_time_s = Some(duration);
                }
                _ => {
                    validate_range("duration", duration, 0.0, 7200.0)?;
                }
            }
            total_seconds += duration;
        }
        match phase_name {
            "heating" | "holding" | "cooling" => {}
            _ => {
                return Err(AppError::bad_request(format!(
                    "unsupported process phase '{}'",
                    phase.phase
                )))
            }
        }
        if let Some(temp) = optional_number_field(&phase.params, "target_temp")? {
            saw_control_param = true;
            validate_target_temperature(&state.safety, temp)?;
            target_temperature = Some(temp);
        }
        if let Some(speed) = optional_number_field(&phase.params, "stir_speed")? {
            saw_control_param = true;
            validate_stir_speed(&state.safety, speed)?;
            stirrer_rpm = Some(speed);
        }
        if let Some(speed) = optional_number_field(&phase.params, "shake_speed")? {
            saw_control_param = true;
            validate_range("shake_speed", speed, 0.0, 60.0)?;
            shake_speed_cpm = Some(speed);
        }
        if let Some(pressure) = optional_number_field(&phase.params, "target_pressure")? {
            saw_control_param = true;
            validate_range("target_pressure", pressure, 0.0, 10.0)?;
            target_pressure_mpa = Some(pressure);
        }
    }
    if !saw_control_param {
        return Err(AppError::bad_request(
            "process phases must include at least one recognized control parameter",
        ));
    }
    let targets = validate_v1_process_targets(
        &state.safety,
        ControlTargets {
            temperature_c: target_temperature.unwrap_or(120.0),
            heat_time_s: heat_time_s.unwrap_or(300.0),
            hold_time_s: hold_time_s.unwrap_or(600.0),
            cool_time_s: cool_time_s.unwrap_or(180.0),
            stirrer_rpm: stirrer_rpm.unwrap_or(800.0),
            shake_speed_cpm: shake_speed_cpm.unwrap_or(30.0),
            target_pressure_mpa: target_pressure_mpa.unwrap_or(0.5),
        },
    )?;
    ensure_targets_allowed(&state.safety, &targets)?;
    let acknowledged_safety_latches = {
        let mut runtime = state.runtime.write().await;
        runtime.auto_enabled = false;
        ensure_target_update_interlock_clear(
            &state,
            &runtime,
            TargetUpdateInterlockMode::V1ProcessLoad,
        )?;
        SafetyLatchGenerations::from_runtime(&runtime)
    };
    state
        .db
        .insert_control_event_sqlx(
            None,
            "v1_process_loaded",
            Some(&SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: "v1 process accepted after document range validation".to_string(),
            }),
            "v1 process accepted after document range validation",
        )
        .await?;
    commit_targets_after_final_interlock(
        &state,
        &targets,
        TargetUpdateInterlockMode::V1ProcessLoad,
        None,
        Some(acknowledged_safety_latches),
    )
    .await?;

    Ok(Json(success(json!({
        "process_id": payload.process_id,
        "device_id": device_id,
        "name": payload.name,
        "status": "accepted",
        "phase_count": payload.phases.len(),
        "estimated_duration": total_seconds.round() as i64,
        "applied_targets": {
            "target_temp": targets.temperature_c,
            "stir_speed": targets.stirrer_rpm,
            "shake_speed": targets.shake_speed_cpm,
            "target_pressure": targets.target_pressure_mpa
        }
    }))))
}

fn ensure_reactor_device_id(device_id: &str) -> Result<(), AppError> {
    if device_id == "reactor_001" {
        Ok(())
    } else {
        Err(AppError::not_found("device not found"))
    }
}

async fn start_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<StartBatchRequest>,
) -> Result<Json<Batch>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let has_explicit_control_field = payload.target_temperature_c.is_some()
        || payload.target_stirrer_rpm.is_some()
        || payload.target_shake_speed_cpm.is_some()
        || payload.heating_minutes.is_some()
        || payload.stirring_minutes.is_some();
    if !has_explicit_control_field {
        return Err(AppError::bad_request(
            "batch start must include at least one explicit target or duration field",
        ));
    }
    let targets = state.runtime.read().await.targets.clone();
    let target_temperature_c =
        optional_present_number(payload.target_temperature_c, "target_temperature_c")?
            .unwrap_or(targets.temperature_c);
    validate_target_temperature(&state.safety, target_temperature_c)?;
    let target_stirrer_rpm =
        optional_present_number(payload.target_stirrer_rpm, "target_stirrer_rpm")?
            .unwrap_or(targets.stirrer_rpm);
    validate_stir_speed(&state.safety, target_stirrer_rpm)?;
    validate_target_pair_allowed(&state.safety, target_temperature_c, target_stirrer_rpm)?;
    let target_shake_speed_cpm = payload.target_shake_speed_cpm;
    let target_shake_speed_cpm =
        optional_present_number(target_shake_speed_cpm, "target_shake_speed_cpm")?
            .unwrap_or(targets.shake_speed_cpm);
    validate_range("target_shake_speed_cpm", target_shake_speed_cpm, 0.0, 60.0)?;
    let heating_minutes =
        optional_present_number(payload.heating_minutes, "heating_minutes")?.unwrap_or(60.0);
    validate_range(
        "heating_minutes",
        heating_minutes,
        0.0,
        state.safety.optimizer.max_heating_minutes,
    )?;
    let stirring_minutes =
        optional_present_number(payload.stirring_minutes, "stirring_minutes")?.unwrap_or(60.0);
    validate_range(
        "stirring_minutes",
        stirring_minutes,
        0.0,
        state.safety.optimizer.max_stirring_minutes,
    )?;
    let process_id = optional_present_i64(payload.process_id, "process_id")?;
    let applied_targets = validate_batch_start_targets(
        &state.safety,
        ControlTargets {
            temperature_c: target_temperature_c,
            heat_time_s: heating_minutes * 60.0,
            hold_time_s: stirring_minutes * 60.0,
            cool_time_s: targets.cool_time_s,
            stirrer_rpm: target_stirrer_rpm,
            shake_speed_cpm: target_shake_speed_cpm,
            target_pressure_mpa: targets.target_pressure_mpa,
        },
    )?;
    let heating_minutes = round2(applied_targets.heat_time_s / 60.0);
    let stirring_minutes = round2(applied_targets.hold_time_s / 60.0);
    let name = clean_label(payload.name, "batch", 160);
    let acknowledged_safety_latches = {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_no_unclosed_db_batch_for_new_production(&state, &runtime).await?;
        ensure_target_update_interlock_clear(
            &state,
            &runtime,
            TargetUpdateInterlockMode::BatchStart,
        )?;
        SafetyLatchGenerations::from_runtime(&runtime)
    };
    let batch = state
        .db
        .create_batch_for_process_sqlx(
            process_id,
            &name,
            applied_targets.temperature_c,
            applied_targets.stirrer_rpm,
            heating_minutes,
            stirring_minutes,
        )
        .await?;
    if let Err(err) = start_process_on_device(&state, &applied_targets, Some(batch.id)).await {
        audit_start_failed_before_activation(
            &state,
            Some(batch.id),
            &applied_targets,
            "batch",
            &err,
        )
        .await;
        if let Err(finish_err) = state.db.finish_batch_sqlx(batch.id).await {
            tracing::warn!("failed to mark failed batch start as finished: {finish_err}");
        }
        return Err(err);
    }
    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            Some(batch.id),
            "batch_started",
            Some(&SafeCommand {
                target_temperature_c: applied_targets.temperature_c,
                heat_time_s: applied_targets.heat_time_s,
                hold_time_s: applied_targets.hold_time_s,
                cool_time_s: applied_targets.cool_time_s,
                target_stirrer_rpm: applied_targets.stirrer_rpm,
                target_shake_speed_cpm: applied_targets.shake_speed_cpm,
                target_pressure_mpa: applied_targets.target_pressure_mpa,
                reason: "batch started and runtime targets updated".to_string(),
            }),
            "batch started and runtime targets updated",
        )
        .await
    {
        let rollback_stop_error =
            rollback_failed_activation(&state, batch.id, Some(&applied_targets)).await;
        latch_tail_failure_after_device_action(
            &state,
            "batch start audit",
            tail_error_with_activation_rollback(
                format_error_for_control_fault(&err),
                rollback_stop_error,
            ),
        )
        .await;
        return Err(err.into());
    }
    if let Err(err) = commit_process_activation_after_final_interlock(
        &state,
        batch.id,
        &applied_targets,
        false,
        Some(acknowledged_safety_latches),
    )
    .await
    {
        let rollback_stop_error =
            rollback_failed_activation(&state, batch.id, Some(&applied_targets)).await;
        latch_tail_failure_after_device_action(
            &state,
            "batch start final interlock",
            tail_error_with_activation_rollback(err.message().to_string(), rollback_stop_error),
        )
        .await;
        return Err(err);
    }
    Ok(Json(batch))
}

async fn finish_batch(
    State(state): State<AppState>,
    axum::extract::Path(batch_id): axum::extract::Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let active_batch_id = state.runtime.read().await.active_batch_id;
    let Some(batch) = state.db.batch_by_id_sqlx(batch_id).await? else {
        if active_batch_id == Some(batch_id) {
            return finish_missing_active_batch_recovery(&state, batch_id).await;
        }
        return Err(AppError::not_found("batch not found"));
    };
    if batch.finished_at.is_some() {
        if active_batch_id != Some(batch_id) {
            return Err(AppError::conflict("batch is already finished"));
        }
    }
    if let Some(active_batch_id) = active_batch_id {
        if active_batch_id != batch_id {
            return Err(AppError::conflict(format!(
                "active batch is {active_batch_id}; cannot finish batch {batch_id} while another batch is running"
            )));
        }
    }
    let was_active = active_batch_id == Some(batch_id);
    let stopped_targets = if was_active {
        let targets = process_stop_targets(&state);
        stop_process_on_device(&state, &targets).await?;
        Some(targets)
    } else {
        None
    };
    {
        let mut runtime = state.runtime.write().await;
        if was_active && runtime.active_batch_id != Some(batch_id) {
            runtime.auto_enabled = false;
            if let Some(targets) = &stopped_targets {
                runtime.targets = targets.clone();
            }
            let found_active_batch_id = runtime.active_batch_id;
            runtime.latch_control_fault(format!(
                "batch finish active batch changed after stop command; expected {batch_id}, found {:?}; production record was not closed",
                found_active_batch_id
            ));
            return Err(AppError::conflict(
                "active batch changed during batch finish; verify field state before retrying",
            ));
        }
        runtime.auto_enabled = false;
        if let Some(targets) = &stopped_targets {
            runtime.targets = targets.clone();
        }
    }
    if batch.finished_at.is_none() {
        if let Err(err) = state.db.finish_batch_sqlx(batch_id).await {
            if was_active {
                latch_tail_failure_after_device_action(
                    &state,
                    "batch finish state commit",
                    format_error_for_control_fault(&err),
                )
                .await;
            }
            return Err(err.into());
        }
    }
    let Some(finished_batch) = state.db.batch_by_id_sqlx(batch_id).await? else {
        if was_active {
            latch_tail_failure_after_device_action(
                &state,
                "batch finish state commit",
                "batch disappeared after finish update",
            )
            .await;
        }
        return Err(AppError::not_found("batch not found after finish update"));
    };
    if finished_batch.finished_at.is_none() {
        if was_active {
            latch_tail_failure_after_device_action(
                &state,
                "batch finish state commit",
                "batch finish update did not mark the batch finished",
            )
            .await;
        }
        return Err(AppError::conflict(
            "batch finish update did not mark the batch finished",
        ));
    }
    let audit_command = stopped_targets.as_ref().map(|targets| SafeCommand {
        target_temperature_c: targets.temperature_c,
        heat_time_s: targets.heat_time_s,
        hold_time_s: targets.hold_time_s,
        cool_time_s: targets.cool_time_s,
        target_stirrer_rpm: targets.stirrer_rpm,
        target_shake_speed_cpm: targets.shake_speed_cpm,
        target_pressure_mpa: targets.target_pressure_mpa,
        reason: "active batch finished; device stop targets written".to_string(),
    });
    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            Some(batch_id),
            "batch_finished",
            audit_command.as_ref(),
            "batch finished; automatic control disabled",
        )
        .await
    {
        if was_active {
            latch_tail_failure_after_device_action(
                &state,
                "batch finish audit",
                format_error_for_control_fault(&err),
            )
            .await;
        }
        return Err(err.into());
    }
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id == Some(batch_id) {
            runtime.active_batch_id = None;
        }
        runtime.auto_enabled = false;
        if let Some(targets) = &stopped_targets {
            runtime.targets = targets.clone();
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn finish_missing_active_batch_recovery(
    state: &AppState,
    batch_id: i64,
) -> Result<StatusCode, AppError> {
    let stopped_targets = process_stop_targets(state);
    stop_process_on_device(state, &stopped_targets).await?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.auto_enabled = false;
        runtime.targets = stopped_targets.clone();
    }

    let recovery_reason = format!(
        "active runtime batch {batch_id} record was missing during finish; risk-reducing stop target was still written and runtime active state was cleared"
    );
    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            None,
            "batch_finish_recovery_missing_batch",
            Some(&SafeCommand {
                target_temperature_c: stopped_targets.temperature_c,
                heat_time_s: stopped_targets.heat_time_s,
                hold_time_s: stopped_targets.hold_time_s,
                cool_time_s: stopped_targets.cool_time_s,
                target_stirrer_rpm: stopped_targets.stirrer_rpm,
                target_shake_speed_cpm: stopped_targets.shake_speed_cpm,
                target_pressure_mpa: stopped_targets.target_pressure_mpa,
                reason: recovery_reason.clone(),
            }),
            &recovery_reason,
        )
        .await
    {
        latch_tail_failure_after_device_action(
            state,
            "batch finish missing batch recovery audit",
            format_error_for_control_fault(&err),
        )
        .await;
        return Err(err.into());
    }
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id == Some(batch_id) {
            runtime.active_batch_id = None;
        }
        runtime.auto_enabled = false;
        runtime.targets = stopped_targets;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn product_results(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ProductResultRequest>,
) -> Result<Json<AiRecommendationEnvelope>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    ensure_batch_ready_for_product_result(&state, payload.batch_id).await?;
    if !(0.0..=100.0).contains(&payload.yield_percent) {
        return Err(AppError::bad_request(
            "yield_percent must be between 0 and 100",
        ));
    }
    if !(0.0..=1.0).contains(&payload.product_ratio) {
        return Err(AppError::bad_request(
            "product_ratio must be between 0 and 1",
        ));
    }
    let result = ProductResult {
        batch_id: payload.batch_id,
        yield_percent: round2(payload.yield_percent),
        product_ratio: round2(payload.product_ratio),
        notes: clean_label(payload.notes, "", 500),
    };
    state
        .db
        .insert_product_result_with_audit_sqlx(
            &result,
            "product_result_recorded",
            "product result saved; recommendation regeneration queued",
        )
        .await?;
    let recommendation = generate_recommendation(&state).await?;
    state
        .db
        .insert_recommendation_with_audit_sqlx(
            &recommendation,
            "product_result_recommendation_generated",
            "product result regenerated AI recommendation",
        )
        .await?;
    Ok(Json(recommendation_envelope(&state, recommendation).await))
}

async fn ensure_batch_ready_for_product_result(
    state: &AppState,
    batch_id: i64,
) -> Result<(), AppError> {
    let Some(batch) = state.db.batch_by_id_sqlx(batch_id).await? else {
        return Err(AppError::not_found("batch not found"));
    };
    if batch.finished_at.is_none() {
        return Err(AppError::conflict(
            "cannot record product result before the batch is finished",
        ));
    }
    let runtime = state.runtime.read().await.clone();
    if let Some(active_batch_id) = runtime.active_batch_id {
        return Err(AppError::conflict(format!(
            "cannot record product result while batch {active_batch_id} is active; finish and verify active production first"
        )));
    }
    let batch_status = unfinished_batch_status(state, &runtime).await?;
    if batch_status.recovery_required() || batch_status.has_unfinished_batch(&runtime) {
        return Err(AppError::conflict(format!(
            "cannot record product result until unfinished batch recovery is resolved: {}",
            batch_status.reason(&runtime)
        )));
    }
    Ok(())
}

async fn ensure_production_basis_write_allowed(
    state: &AppState,
    action: &str,
) -> Result<(), AppError> {
    let runtime = state.runtime.read().await.clone();
    if let Some(active_batch_id) = runtime.active_batch_id {
        return Err(AppError::conflict(format!(
            "{action} blocked while batch {active_batch_id} is active; finish and verify active production first"
        )));
    }
    let batch_status = unfinished_batch_status(state, &runtime).await?;
    if batch_status.recovery_required() || batch_status.has_unfinished_batch(&runtime) {
        return Err(AppError::conflict(format!(
            "{action} blocked until unfinished batch recovery is resolved: {}",
            batch_status.reason(&runtime)
        )));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct SafetyLatchGenerations {
    manual_lock: u64,
    emergency_stop: u64,
    control_fault: u64,
}

impl SafetyLatchGenerations {
    pub(crate) fn from_runtime(runtime: &RuntimeState) -> Self {
        Self {
            manual_lock: runtime.manual_lock_generation,
            emergency_stop: runtime.emergency_stop_generation,
            control_fault: runtime.control_fault_generation,
        }
    }

    fn changed_since(self, runtime: &RuntimeState) -> bool {
        self.manual_lock != runtime.manual_lock_generation
            || self.emergency_stop != runtime.emergency_stop_generation
            || self.control_fault != runtime.control_fault_generation
    }
}

async fn set_auto(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<AutoRequest>,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    let acknowledged_safety_latches = if payload.enabled {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_target_update_interlock_clear(
            &state,
            &runtime,
            TargetUpdateInterlockMode::AutoEnable,
        )?;
        ensure_persisted_batch_state_consistent(
            &state,
            &runtime,
            TargetUpdateInterlockMode::AutoEnable,
        )
        .await?;
        Some(SafetyLatchGenerations::from_runtime(&runtime))
    } else {
        let mut runtime = state.runtime.write().await;
        runtime.auto_enabled = false;
        None
    };
    state
        .db
        .insert_control_event_sqlx(
            None,
            if payload.enabled {
                "auto_enabled"
            } else {
                "auto_disabled"
            },
            None,
            "operator changed automatic control state",
        )
        .await?;
    if payload.enabled {
        let mut runtime = state.runtime.write().await;
        ensure_target_update_interlock_clear(
            &state,
            &runtime,
            TargetUpdateInterlockMode::AutoEnable,
        )?;
        if acknowledged_safety_latches
            .map(|latches| latches.changed_since(&runtime))
            .unwrap_or(false)
        {
            runtime.auto_enabled = false;
            return Err(AppError::conflict(
                "automatic control enable safety state changed during audit; operator must re-check field state",
            ));
        }
        let batch_status = unfinished_batch_status(&state, &runtime).await?;
        ensure_persisted_batch_status_consistent(
            &batch_status,
            &runtime,
            TargetUpdateInterlockMode::AutoEnable,
        )?;
        runtime.auto_enabled = true;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn set_manual_lock(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ManualLockRequest>,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    let acknowledged_safety_latches = if !payload.locked {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_manual_unlock_interlock_clear(&state, &runtime).await?;
        // Snapshot all three latch generations (not just manual_lock): unlocking is a
        // risk-increasing action, and an emergency-stop or control-fault that fired and
        // cleared transiently during the audit window must refuse the unlock just as the
        // commit_*_after_final_interlock paths refuse other risk-increasing commits.
        Some(SafetyLatchGenerations::from_runtime(&runtime))
    } else {
        None
    };
    if payload.locked {
        let mut runtime = state.runtime.write().await;
        runtime.engage_manual_lock();
        // Locking is a risk-reducing action and cannot be refused by a latch
        // change in the audit window, so audit it unconditionally.
        state
            .db
            .insert_control_event_sqlx(
                None,
                "manual_lock_on",
                None,
                "operator enabled manual lock; automatic control disabled",
            )
            .await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    // Unlocking is risk-INCREASING, so we must NOT write the "manual_lock_off"
    // audit row until the generation re-check below confirms the field state is
    // still the one we audited. Writing it before the re-check (as the prior
    // code did) left a stale "lock off" row in the hash-chained audit trail even
    // when the unlock was refused and runtime.manual_lock stayed true.
    let interlock_runtime = { state.runtime.read().await.clone() };
    ensure_manual_unlock_interlock_clear(&state, &interlock_runtime).await?;
    // Order rationale (this was the hard part):
    //  - The generation re-check must run AFTER an audit insert succeeds, because
    //    that audit commit is the "time anchor": the test hook (and any real
    //    concurrent latch) fires inside that window, and only a re-check after it
    //    can catch a generation change "during the audit window". Re-checking
    //    before any audit would miss it (regression: returned 204 instead of 409).
    //  - But we must NOT mutate runtime state until the audit that precedes it has
    //    succeeded — so an audit failure leaves manual_lock engaged (fail-closed,
    //    matches commit_*_after_final_interlock). clear_manual_lock only runs on
    //    the success path, AFTER the re-check passes.
    //  - And we must not leave a stale "manual_lock_off" row when the unlock is
    //    refused (#7). We accept that the off row is written first (it is the
    //    audit anchor that the re-check needs), but on refusal we immediately
    //    append a "manual_unlock_refused" compensating row stating the lock is
    //    STILL engaged. The chain is therefore self-consistent: an off row
    //    followed by a refused row means "attempted off, but it was refused and
    //    did not take effect".
    state
        .db
        .insert_control_event_sqlx(
            None,
            "manual_lock_off",
            None,
            "operator requested manual lock disable; awaiting final generation re-check before clearing",
        )
        .await?;
    // Final generation re-check: any latch that advanced during the audit window
    // (manual re-lock, emergency stop, or control fault) refuses the unlock.
    let refused = {
        let runtime = state.runtime.read().await;
        acknowledged_safety_latches
            .map(|acknowledged| acknowledged.changed_since(&runtime))
            .unwrap_or(false)
    };
    if refused {
        // The off row above was written but the lock is STILL engaged — record a
        // compensating refused row so the audit chain is self-consistent, then
        // refuse. We never call clear_manual_lock here.
        state
            .db
            .insert_control_event_sqlx(
                None,
                "manual_unlock_refused",
                None,
                "manual lock unlock refused: a safety latch fired during the audit window; the preceding manual_lock_off did NOT take effect and manual lock remains engaged",
            )
            .await?;
        {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
        }
        return Err(AppError::conflict(
            "manual lock unlock blocked: a safety latch fired during the audit window; field state changed, so the unlock was refused. Re-verify field state before retrying",
        ));
    }
    // Re-check passed: clear the lock. The audit anchor (off row) is already
    // durably persisted, so the field change follows a durable record.
    {
        let mut runtime = state.runtime.write().await;
        runtime.clear_manual_lock();
    }
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_no_active_batch_for_reset(
    runtime: &RuntimeState,
    action: &'static str,
) -> Result<(), AppError> {
    if let Some(batch_id) = runtime.active_batch_id {
        return Err(AppError::conflict(format!(
            "active batch {batch_id} is still open; retry stop/finish to close production state before {action}"
        )));
    }
    Ok(())
}

async fn ensure_no_unfinished_batch_recovery_for_reset(
    state: &AppState,
    runtime: &RuntimeState,
    action: &'static str,
) -> Result<(), AppError> {
    let batch_status = unfinished_batch_status(state, runtime).await?;
    if batch_status.recovery_required() {
        return Err(AppError::conflict(format!(
            "{action} blocked until unfinished batch recovery is resolved: {}",
            batch_status.reason(runtime)
        )));
    }
    Ok(())
}

async fn reset_control_fault(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    let (acknowledged_error, acknowledged_control_fault_generation) = {
        let mut runtime = state.runtime.write().await;
        let Some(error) = runtime.last_control_error.clone() else {
            return Err(AppError::conflict(
                "no latched device control fault to reset",
            ));
        };
        // A terminated control-loop task is not a recoverable device-write
        // fault: the supervisor is gone and is only re-spawned by a process
        // restart. Clearing last_control_error here would make the API report
        // "no fault" while nothing is supervising the device, and
        // ensure_target_update_interlock_clear would then let automatic control
        // resume unsupervised. Refuse; the operator must restart the process.
        if runtime.control_loop_terminated {
            return Err(AppError::conflict(
                "control loop task has terminated; this fault cannot be cleared via the API. Restart the daemon process and re-verify field state before re-enabling automatic control",
            ));
        }
        runtime.auto_enabled = false;
        ensure_no_active_batch_for_reset(&runtime, "resetting the control fault")?;
        ensure_no_unfinished_batch_recovery_for_reset(&state, &runtime, "control fault reset")
            .await?;
        ensure_fresh_sample(&state, &runtime)?;
        ensure_proven_healthy_device_status(&state, &runtime, "control fault reset")?;
        (error, runtime.control_fault_generation)
    };
    state
        .db
        .insert_control_event_sqlx(
            None,
            "control_fault_reset",
            None,
            "operator acknowledged device write fault after field verification; automatic control remains disabled",
        )
        .await?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.auto_enabled = false;
        if let Err(err) = ensure_no_active_batch_for_reset(&runtime, "resetting the control fault")
        {
            return Err(err);
        }
        ensure_no_unfinished_batch_recovery_for_reset(
            &state,
            &runtime,
            "control fault reset after audit",
        )
        .await?;
        if let Err(err) = ensure_fresh_sample(&state, &runtime) {
            return Err(err);
        }
        if let Err(err) =
            ensure_proven_healthy_device_status(&state, &runtime, "control fault reset after audit")
        {
            return Err(err);
        }
        if runtime.control_fault_generation != acknowledged_control_fault_generation
            || runtime.last_control_error.as_deref() != Some(acknowledged_error.as_str())
        {
            return Err(AppError::conflict(
                "latched control fault changed during reset; maintenance must re-verify field state",
            ));
        }
        runtime.clear_control_fault();
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn set_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<TargetRequest>,
) -> Result<Json<ControlTargets>, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    let current = state.runtime.read().await.targets.clone();
    let targets = ControlTargets {
        temperature_c: payload.temperature_c,
        heat_time_s: current.heat_time_s,
        hold_time_s: current.hold_time_s,
        cool_time_s: current.cool_time_s,
        stirrer_rpm: payload.stirrer_rpm,
        shake_speed_cpm: optional_present_number(payload.shake_speed_cpm, "shake_speed_cpm")?
            .unwrap_or(current.shake_speed_cpm),
        target_pressure_mpa: current.target_pressure_mpa,
    };
    let targets = validate_operator_targets(&state.safety, targets)?;
    ensure_targets_allowed(&state.safety, &targets)?;
    let acknowledged_safety_latches = {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_target_update_interlock_clear(
            &state,
            &runtime,
            TargetUpdateInterlockMode::DesiredTargets,
        )?;
        SafetyLatchGenerations::from_runtime(&runtime)
    };
    state
        .db
        .insert_control_event_sqlx(
            None,
            "operator_targets_updated",
            Some(&SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: "operator target request after safety validation".to_string(),
            }),
            "operator changed desired targets after configured safety validation",
        )
        .await?;
    commit_targets_after_final_interlock(
        &state,
        &targets,
        TargetUpdateInterlockMode::DesiredTargets,
        Some(&current),
        Some(acknowledged_safety_latches),
    )
    .await?;
    Ok(Json(targets))
}

fn validate_operator_targets(
    safety: &SafetyConfig,
    targets: ControlTargets,
) -> Result<ControlTargets, AppError> {
    validate_target_temperature(safety, targets.temperature_c)
        .map_err(|err| err.with_message_prefix("temperature_c"))?;
    validate_stir_speed(safety, targets.stirrer_rpm)
        .map_err(|err| err.with_message_prefix("stirrer_rpm"))?;
    validate_range("shake_speed_cpm", targets.shake_speed_cpm, 0.0, 60.0)?;
    validate_range(
        "target_pressure_mpa",
        targets.target_pressure_mpa,
        0.0,
        10.0,
    )?;
    validate_range("heat_time_s", targets.heat_time_s, 0.0, 3600.0)?;
    validate_range("hold_time_s", targets.hold_time_s, 0.0, 7200.0)?;
    validate_range("cool_time_s", targets.cool_time_s, 0.0, 3600.0)?;
    Ok(ControlTargets {
        temperature_c: round2(targets.temperature_c),
        heat_time_s: round2(targets.heat_time_s),
        hold_time_s: round2(targets.hold_time_s),
        cool_time_s: round2(targets.cool_time_s),
        stirrer_rpm: round2(targets.stirrer_rpm),
        shake_speed_cpm: round2(targets.shake_speed_cpm),
        target_pressure_mpa: round2(targets.target_pressure_mpa),
    })
}

async fn emergency_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::EmergencyStop)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.engage_emergency_stop();
    }
    if let Err(err) = state
        .db
        .insert_control_event_sqlx(
            None,
            "emergency_stop",
            None,
            "operator triggered emergency stop; automatic control disabled",
        )
        .await
    {
        let mut runtime = state.runtime.write().await;
        runtime.latch_control_fault(format!(
            "emergency stop audit failed after fail-safe state change: {err}"
        ));
        return Err(err.into());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_emergency_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::EmergencyStop)?;
    let acknowledged_emergency_stop_generation = {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            if !runtime.emergency_stop {
                return Err(AppError::conflict(
                    "emergency stop is not active; reset is not required",
                ));
            }
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_no_active_batch_for_reset(&runtime, "resetting emergency stop")?;
        ensure_no_unfinished_batch_recovery_for_reset(&state, &runtime, "emergency stop reset")
            .await?;
        ensure_fresh_sample(&state, &runtime)?;
        ensure_proven_healthy_device_status(&state, &runtime, "emergency stop reset")?;
        runtime.emergency_stop_generation
    };
    state
        .db
        .insert_control_event_sqlx(
            None,
            "emergency_stop_reset",
            None,
            "operator reset emergency stop flag",
        )
        .await?;
    {
        let mut runtime = state.runtime.write().await;
        if !runtime.emergency_stop {
            runtime.auto_enabled = false;
            return Err(AppError::conflict(
                "emergency stop changed during reset; operator must re-check field state",
            ));
        }
        if runtime.emergency_stop_generation != acknowledged_emergency_stop_generation {
            runtime.auto_enabled = false;
            return Err(AppError::conflict(
                "emergency stop changed during reset; operator must re-check field state",
            ));
        }
        ensure_no_active_batch_for_reset(&runtime, "resetting emergency stop after audit")?;
        ensure_no_unfinished_batch_recovery_for_reset(
            &state,
            &runtime,
            "emergency stop reset after audit",
        )
        .await?;
        ensure_fresh_sample(&state, &runtime)?;
        if let Err(err) = ensure_proven_healthy_device_status(
            &state,
            &runtime,
            "emergency stop reset after audit",
        ) {
            runtime.auto_enabled = false;
            return Err(err);
        }
        runtime.clear_emergency_stop();
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn latest_recommendation(
    State(state): State<AppState>,
) -> Result<Json<Option<AiRecommendationEnvelope>>, AppError> {
    let recommendation = state.db.latest_recommendation_sqlx().await?;
    Ok(Json(match recommendation {
        Some(recommendation) => Some(recommendation_envelope(&state, recommendation).await),
        None => None,
    }))
}

async fn generate_latest_recommendation(
    State(state): State<AppState>,
) -> Result<Json<Option<AiRecommendationEnvelope>>, AppError> {
    ensure_production_basis_write_allowed(&state, "recommendation regeneration").await?;
    let recommendation = generate_recommendation(&state).await?;
    state
        .db
        .insert_recommendation_with_audit_sqlx(
            &recommendation,
            "recommendation_generated",
            "operator regenerated latest AI recommendation",
        )
        .await?;
    Ok(Json(Some(
        recommendation_envelope(&state, recommendation).await,
    )))
}

async fn api_not_found() -> AppError {
    AppError::not_found("api route not found")
}

async fn test_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    ensure_test_endpoint_enabled(&state, &headers)?;
    {
        let runtime = state.runtime.read().await;
        ensure_test_reset_runtime_safe(&runtime)?;
        ensure_no_unclosed_db_batch_for_test_reset(&state, &runtime).await?;
    }
    state.db.clear_runtime_data_for_tests()?;
    {
        let mut runtime = state.runtime.write().await;
        *runtime = RuntimeState::from_safety(&state.safety);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn test_pipeline_sample(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SensorSnapshot>, AppError> {
    ensure_test_endpoint_enabled(&state, &headers)?;

    let payload = parse_pipeline_sample_payload(&state, body).await?;
    let sample = accept_pipeline_sample(&state, payload).await?;
    Ok(Json(sample))
}

fn ensure_test_endpoint_enabled(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    if !state.test_reset_enabled {
        return Err(AppError::not_found("not found"));
    }
    let confirmed = headers
        .get("x-xingshu-test-confirm")
        .and_then(|value| value.to_str().ok())
        == Some("local-e2e");
    if !confirmed {
        return Err(AppError::forbidden(
            "test endpoint requires X-Xingshu-Test-Confirm: local-e2e",
        ));
    }
    Ok(())
}

fn ensure_test_reset_runtime_safe(runtime: &RuntimeState) -> Result<(), AppError> {
    if runtime.active_batch_id.is_some() {
        return Err(AppError::conflict(
            "test reset blocked while a batch is active; stop the process first",
        ));
    }
    if runtime.auto_enabled {
        return Err(AppError::conflict(
            "test reset blocked while automatic control is enabled",
        ));
    }
    if runtime.emergency_stop {
        return Err(AppError::conflict(
            "test reset blocked while emergency stop is active",
        ));
    }
    if runtime.last_control_error.is_some() {
        return Err(AppError::conflict(
            "test reset blocked while a control fault is uncleared",
        ));
    }
    Ok(())
}

async fn ensure_no_unclosed_db_batch_for_test_reset(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<(), AppError> {
    let batch_status = unfinished_batch_status(state, runtime).await?;
    if batch_status.has_unfinished_batch(runtime) {
        return Err(AppError::conflict(format!(
            "test reset blocked while database has unfinished batch state: {}",
            batch_status.reason(runtime)
        )));
    }
    Ok(())
}

async fn accept_pipeline_sample(
    state: &AppState,
    payload: PipelineSampleRequest,
) -> Result<SensorSnapshot, AppError> {
    let sample = match pipeline_sample_from_request(payload) {
        Ok(sample) => sample,
        Err(err) => {
            reject_pipeline_sample_input(
                state,
                format!("sensor sample rejected: {}", err.message()),
            )
            .await;
            return Err(err);
        }
    };
    let active_batch_id = state.runtime.read().await.active_batch_id;
    if let Err(err) = state.db.insert_sample_sqlx(active_batch_id, &sample).await {
        let reason = format!("sensor sample persistence failed: {err:#}");
        let auto_was_disabled = {
            let mut runtime = state.runtime.write().await;
            runtime.reject_unpersisted_sample_with_status(None, reason.clone())
        };
        audit_field_input_auto_disable(&state.db, active_batch_id, auto_was_disabled, &reason)
            .await;
        return Err(err.into());
    }
    let updated_runtime = {
        let mut runtime = state.runtime.write().await;
        runtime.latest_sample = Some(sample.clone());
        runtime.last_sensor_error = None;
        runtime.clone()
    };
    let alarms = alarms_for(
        state.safety.as_ref(),
        &updated_runtime,
        updated_runtime.latest_sample.as_ref(),
        state.ai_memory.as_ref(),
    );
    let high_alarm_count = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("high"))
        .count();
    let (auto_disabled_by_alarm, auto_disabled_by_control_fault) = {
        let mut runtime = state.runtime.write().await;
        let auto_disabled_by_alarm =
            apply_high_sensor_alarm_fail_closed(&mut runtime, high_alarm_count);
        let auto_disabled_by_control_fault = runtime.enforce_control_fault_fail_closed();
        (auto_disabled_by_alarm, auto_disabled_by_control_fault)
    };
    if auto_disabled_by_alarm {
        audit_high_sensor_alarm_auto_disable(&state.db, active_batch_id, &alarms).await;
    }
    if auto_disabled_by_control_fault {
        audit_control_fault_auto_disable(&state.db, active_batch_id).await;
    }
    Ok(sample)
}

async fn parse_pipeline_sample_payload(
    state: &AppState,
    body: Bytes,
) -> Result<PipelineSampleRequest, AppError> {
    match serde_json::from_slice::<PipelineSampleRequest>(&body) {
        Ok(payload) => Ok(payload),
        Err(err) => {
            let message = format!("invalid sensor sample JSON: {err}");
            reject_pipeline_sample_input(state, format!("sensor sample rejected: {message}")).await;
            Err(AppError::bad_request(message))
        }
    }
}

async fn reject_pipeline_sample_input(state: &AppState, reason: String) {
    let (active_batch_id, auto_was_disabled) = {
        let mut runtime = state.runtime.write().await;
        let active_batch_id = runtime.active_batch_id;
        let auto_was_disabled = runtime.reject_unpersisted_sample_with_status(None, reason.clone());
        (active_batch_id, auto_was_disabled)
    };
    audit_field_input_auto_disable(&state.db, active_batch_id, auto_was_disabled, &reason).await;
}

fn apply_high_sensor_alarm_fail_closed(
    runtime: &mut RuntimeState,
    high_alarm_count: usize,
) -> bool {
    if high_alarm_count == 0 {
        return false;
    }
    let should_audit = runtime.auto_enabled || runtime.active_batch_id.is_some();
    runtime.auto_enabled = false;
    should_audit
}

async fn audit_high_sensor_alarm_auto_disable(
    db: &Db,
    active_batch_id: Option<i64>,
    alarms: &[Value],
) {
    let high_alarm_types: Vec<String> = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("high"))
        .filter_map(|alarm| {
            alarm
                .get("type")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect();
    let reason = format!(
        "high sensor alarm disabled automatic control: {}",
        if high_alarm_types.is_empty() {
            "unknown high alarm".to_string()
        } else {
            high_alarm_types.join(", ")
        }
    );
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "high_sensor_alarm_auto_disabled",
            None,
            &reason,
        )
        .await
    {
        tracing::warn!("failed to persist high_sensor_alarm_auto_disabled event: {err}");
    }
}

async fn audit_field_input_auto_disable(
    db: &Db,
    active_batch_id: Option<i64>,
    auto_was_disabled: bool,
    reason: &str,
) {
    if !auto_was_disabled {
        return;
    }
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "field_input_fault_auto_disabled",
            None,
            reason,
        )
        .await
    {
        tracing::warn!("failed to persist field_input_fault_auto_disabled event: {err}");
    }
}

async fn audit_control_fault_auto_disable(db: &Db, active_batch_id: Option<i64>) {
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "control_fault_auto_disabled",
            None,
            "control fault was already latched; automatic control forced disabled",
        )
        .await
    {
        tracing::warn!("failed to persist control_fault_auto_disabled event: {err}");
    }
}

fn pipeline_sample_from_request(
    payload: PipelineSampleRequest,
) -> Result<SensorSnapshot, AppError> {
    let captured_at = Utc::now();
    validate_sensor_tilt_state(payload.tilt_state)
        .map(|_| ())
        .map_err(AppError::bad_request)?;
    let shake_speed_cpm = round2_sensor(SENSOR_SHAKE_SPEED_CPM_RANGE, payload.shake_speed_cpm)?;
    let sample = SensorSnapshot {
        temperature_c: round2_sensor(SENSOR_TEMPERATURE_C_RANGE, payload.temperature_c)?,
        pressure_mpa: round2_sensor(SENSOR_PRESSURE_MPA_RANGE, payload.pressure_mpa)?,
        stirrer_rpm: round2_sensor(SENSOR_STIRRER_RPM_RANGE, payload.stirrer_rpm)?,
        shake_speed_cpm,
        tilt_state: payload.tilt_state,
        tilt_angle_deg: fit_tilt_angle_deg(payload.tilt_state, shake_speed_cpm, captured_at),
        flow_rate_l_min: round2_sensor(SENSOR_FLOW_RATE_L_MIN_RANGE, payload.flow_rate_l_min)?,
        product_concentration_percent: round2_sensor(
            SENSOR_PRODUCT_CONCENTRATION_PERCENT_RANGE,
            payload.product_concentration_percent,
        )?,
        ph: round2_sensor(SENSOR_PH_RANGE, payload.ph)?,
        captured_at,
    };
    validate_sensor_snapshot(&sample).map_err(AppError::bad_request)?;
    Ok(sample)
}

fn round2_sensor(range: SensorRange, value: f64) -> Result<f64, AppError> {
    range.validate(value).map_err(AppError::bad_request)?;
    Ok(round2(value))
}

fn ensure_fresh_sample(state: &AppState, runtime: &RuntimeState) -> Result<(), AppError> {
    fresh_sample_for_realtime(state, runtime).map(|_| ())
}

fn fresh_sample_for_realtime<'a>(
    state: &AppState,
    runtime: &'a RuntimeState,
) -> Result<&'a SensorSnapshot, AppError> {
    if let Some(error) = &runtime.last_sensor_error {
        return Err(AppError::service_unavailable(format!(
            "sensor data unavailable; {error}"
        )));
    }
    let Some(sample) = &runtime.latest_sample else {
        return Err(AppError::service_unavailable(
            "sensor data unavailable; waiting for ESP32/data pipeline sample",
        ));
    };
    let age_ms = timestamp_age_ms(sample.captured_at);
    if age_ms < 0 {
        return Err(AppError::service_unavailable(format!(
            "sensor data timestamp is {} ms in the future; check controller clock synchronization",
            -age_ms
        )));
    }
    if age_ms > state.safety.control.sensor_timeout_ms {
        return Err(AppError::service_unavailable(format!(
            "sensor data stale; last data pipeline sample is {} ms old",
            age_ms
        )));
    }
    Ok(sample)
}

async fn ensure_manual_unlock_interlock_clear(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<(), AppError> {
    if runtime.emergency_stop {
        return Err(AppError::conflict(
            "emergency stop is active; manual lock unlock blocked",
        ));
    }
    if let Some(error) = &runtime.last_control_error {
        return Err(AppError::service_unavailable(format!(
            "last device control write failed; manual lock unlock blocked until maintenance clears the fault: {error}"
        )));
    }
    ensure_no_unfinished_batch_recovery_for_reset(state, runtime, "manual lock unlock").await?;
    ensure_proven_healthy_device_status(state, runtime, "manual lock unlock")?;
    ensure_fresh_sample(state, runtime)
}

pub(crate) fn ensure_target_update_interlock_clear(
    state: &AppState,
    runtime: &RuntimeState,
    mode: TargetUpdateInterlockMode,
) -> Result<(), AppError> {
    if runtime.emergency_stop {
        return Err(AppError::conflict(format!(
            "emergency stop is active; {} blocked",
            mode.description()
        )));
    }
    if runtime.manual_lock {
        return Err(AppError::conflict(format!(
            "manual lock is active; {} blocked",
            mode.description()
        )));
    }
    if let Some(error) = &runtime.last_control_error {
        return Err(AppError::service_unavailable(format!(
            "last device control write failed; {} blocked until maintenance clears the fault: {error}",
            mode.description()
        )));
    }
    ensure_proven_healthy_device_status(state, runtime, mode.description())?;
    ensure_fresh_sample(state, runtime)
}

fn ensure_proven_healthy_device_status(
    state: &AppState,
    runtime: &RuntimeState,
    action: &str,
) -> Result<(), AppError> {
    let Some(status) = runtime.device_status.as_ref() else {
        if state.safety.control.require_device_status_for_control {
            return Err(AppError::service_unavailable(format!(
                "device status unavailable; {action} blocked until downstream status is proven healthy"
            )));
        }
        return Ok(());
    };
    if let Some(reason) =
        device_status_field_fault_reason(status, state.safety.control.sensor_timeout_ms)
    {
        return Err(AppError::service_unavailable(format!(
            "device status is not healthy; {action} blocked: {reason}"
        )));
    }
    if let Some(reason) = downstream_command_fault_reason(status) {
        return Err(AppError::service_unavailable(format!(
            "downstream command fault is still reported; {action} blocked until maintenance clears the fault: {reason}"
        )));
    }
    Ok(())
}

async fn generate_recommendation(state: &AppState) -> Result<Recommendation, AppError> {
    let outcomes = state.db.batch_outcomes_sqlx().await?;
    if outcomes.is_empty() {
        return Err(AppError::service_unavailable(
            "product result data unavailable; waiting for finished batch outcomes",
        ));
    }
    let fallback =
        recommend_with_memory(&state.safety.optimizer, Some(&state.ai_memory), &outcomes);
    let Some(provider) = &state.ai_provider else {
        return Ok(fallback);
    };
    provider
        .recommend(
            &state.safety.optimizer,
            &state.ai_memory,
            &outcomes,
            &fallback,
        )
        .await
        .map_err(|err| {
            tracing::warn!("StepFun recommendation failed without local fallback: {err}");
            AppError::service_unavailable(format!(
                "StepFun AI provider unavailable; recommendation was not generated by local rules: {err}"
            ))
        })
}

async fn recommendation_envelope(
    state: &AppState,
    recommendation: Recommendation,
) -> AiRecommendationEnvelope {
    let Some(provider) = &state.ai_provider else {
        return local_envelope(recommendation);
    };
    if recommendation.rationale.starts_with("StepFun:") {
        stepfun_envelope(recommendation, provider.model_name())
    } else {
        stale_local_envelope(
            recommendation,
            provider.model_name(),
            "cached local recommendation must be regenerated by StepFun before AI master control",
        )
    }
}

fn local_provider_for(state: &AppState) -> AiRecommendationProvider {
    if let Some(provider) = &state.ai_provider {
        AiRecommendationProvider {
            mode: "stepfun_configured".to_string(),
            model: provider.model_name().to_string(),
            fallback_reason: None,
        }
    } else {
        AiRecommendationProvider {
            mode: "local_optimizer".to_string(),
            model: "local-ga-sa-pid".to_string(),
            fallback_reason: None,
        }
    }
}

fn provider_allows_recommendation(state: &AppState, recommendation: &Recommendation) -> bool {
    state.ai_provider.is_none() || recommendation.rationale.starts_with("StepFun:")
}

fn clean_label(value: Option<String>, fallback: &str, max_chars: usize) -> String {
    let cleaned = value.as_deref().and_then(|value| {
        let normalized = normalize_operator_text(value, max_chars);
        (!normalized.is_empty()).then_some(normalized)
    });
    cleaned.unwrap_or_else(|| normalize_operator_text(fallback, max_chars))
}

fn normalize_operator_text(value: &str, max_chars: usize) -> String {
    let mut normalized = String::new();
    let mut pending_space = false;
    let mut count = 0usize;
    for ch in value.chars() {
        if is_invisible_format_char(ch) {
            continue;
        }
        if ch.is_control() || ch.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            if count >= max_chars {
                break;
            }
            normalized.push(' ');
            count += 1;
            pending_space = false;
        }
        if count >= max_chars {
            break;
        }
        normalized.push(ch);
        count += 1;
    }
    normalized
}

fn is_invisible_format_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

fn clean_status(value: &str) -> Result<&'static str, AppError> {
    match value {
        "draft" => Ok("draft"),
        "applied" => Ok("applied"),
        "archived" => Ok("archived"),
        _ => Err(AppError::bad_request(
            "status must be draft, applied, or archived",
        )),
    }
}

fn validate_process_step(
    safety: &SafetyConfig,
    payload: ProcessStepRequest,
) -> Result<NewProcessStep, AppError> {
    let name = clean_label(payload.name, "新步骤", 80);
    validate_target_temperature(safety, payload.target_temperature_c)?;
    let ramp_rate_c_min =
        optional_present_number(payload.ramp_rate_c_min, "ramp_rate_c_min")?.unwrap_or(0.0);
    validate_range("ramp_rate_c_min", ramp_rate_c_min, -20.0, 20.0)?;
    validate_range(
        "duration_minutes",
        payload.duration_minutes,
        1.0,
        safety
            .optimizer
            .max_stirring_minutes
            .max(safety.optimizer.max_heating_minutes),
    )?;
    validate_stir_speed(safety, payload.target_stirrer_rpm)?;
    validate_target_pair_allowed(
        safety,
        payload.target_temperature_c,
        payload.target_stirrer_rpm,
    )?;
    let target_shake_speed_cpm =
        optional_present_number(payload.target_shake_speed_cpm, "target_shake_speed_cpm")?
            .unwrap_or(30.0);
    validate_range("target_shake_speed_cpm", target_shake_speed_cpm, 0.0, 60.0)?;
    let target_pressure_mpa =
        optional_present_number(payload.target_pressure_mpa, "target_pressure_mpa")?.unwrap_or(0.5);
    validate_range("target_pressure_mpa", target_pressure_mpa, 0.0, 10.0)?;
    let cooling_mode = clean_label(payload.cooling_mode, "自然", 20);
    Ok(NewProcessStep {
        name,
        target_temperature_c: round2(payload.target_temperature_c),
        ramp_rate_c_min: round2(ramp_rate_c_min),
        duration_minutes: round2(payload.duration_minutes),
        target_stirrer_rpm: round2(payload.target_stirrer_rpm),
        target_shake_speed_cpm: round2(target_shake_speed_cpm),
        target_pressure_mpa: round2(target_pressure_mpa),
        cooling_mode,
    })
}

fn targets_from_process_steps(
    safety: &SafetyConfig,
    steps: &[ProcessStep],
) -> Result<ControlTargets, AppError> {
    for step in steps {
        validate_persisted_process_step(safety, step)?;
    }
    let first = steps
        .first()
        .ok_or_else(|| AppError::bad_request("process must contain at least one step"))?;
    let hold = steps.get(1).unwrap_or(first);
    let last = steps.last().unwrap_or(first);
    let heat_time_s =
        process_step_duration_seconds(first, "heating", safety.optimizer.max_heating_minutes)?;
    let hold_time_s =
        process_step_duration_seconds(hold, "stirring", safety.optimizer.max_stirring_minutes)?;
    let cool_time_s = if steps.len() > 2 {
        process_step_duration_seconds(last, "cooling", 60.0)?
    } else {
        180.0
    };
    let targets = ControlTargets {
        temperature_c: round2(first.target_temperature_c),
        heat_time_s: round2(heat_time_s),
        hold_time_s: round2(hold_time_s),
        cool_time_s: round2(cool_time_s),
        stirrer_rpm: round2(hold.target_stirrer_rpm),
        shake_speed_cpm: round2(hold.target_shake_speed_cpm),
        target_pressure_mpa: round2(hold.target_pressure_mpa),
    };
    ensure_targets_allowed(safety, &targets)?;
    Ok(targets)
}

fn process_step_duration_seconds(
    step: &ProcessStep,
    role: &str,
    max_minutes: f64,
) -> Result<f64, AppError> {
    validate_range(
        &format!("process step {} {role}_duration_minutes", step.id),
        step.duration_minutes,
        1.0,
        max_minutes,
    )?;
    Ok(step.duration_minutes * 60.0)
}

fn validate_persisted_process_step(
    safety: &SafetyConfig,
    step: &ProcessStep,
) -> Result<(), AppError> {
    let field = |name: &str| format!("process step {} {name}", step.id);
    validate_target_temperature(safety, step.target_temperature_c)
        .map_err(|err| err.with_message_prefix(&field("target_temperature_c")))?;
    validate_range(&field("ramp_rate_c_min"), step.ramp_rate_c_min, -20.0, 20.0)?;
    validate_range(
        &field("duration_minutes"),
        step.duration_minutes,
        1.0,
        safety
            .optimizer
            .max_stirring_minutes
            .max(safety.optimizer.max_heating_minutes),
    )?;
    validate_stir_speed(safety, step.target_stirrer_rpm)
        .map_err(|err| err.with_message_prefix(&field("target_stirrer_rpm")))?;
    validate_range(
        &field("target_shake_speed_cpm"),
        step.target_shake_speed_cpm,
        0.0,
        60.0,
    )?;
    validate_range(
        &field("target_pressure_mpa"),
        step.target_pressure_mpa,
        0.0,
        10.0,
    )?;
    validate_target_pair_allowed(safety, step.target_temperature_c, step.target_stirrer_rpm)
        .map_err(|err| err.with_message_prefix(&field("target pair")))?;
    Ok(())
}

fn seconds_to_minutes(seconds: Option<f64>) -> f64 {
    round2(seconds.unwrap_or(3600.0) / 60.0)
}

fn validate_v1_control_params(
    safety: &SafetyConfig,
    params: &V1ControlParams,
    running_state: bool,
) -> Result<ControlTargets, AppError> {
    let heat_time = optional_present_number(params.heat_time, "heat_time")?;
    let hold_time = optional_present_number(params.hold_time, "hold_time")?;
    let cool_time = optional_present_number(params.cool_time, "cool_time")?;
    let stir_speed = optional_present_number(params.stir_speed, "stir_speed")?;
    let shake_speed = optional_present_number(params.shake_speed, "shake_speed")?;
    let target_temp = optional_present_number(params.target_temp, "target_temp")?;
    let target_pressure = optional_present_number(params.target_pressure, "target_pressure")?;

    if heat_time.is_none()
        && hold_time.is_none()
        && cool_time.is_none()
        && stir_speed.is_none()
        && shake_speed.is_none()
        && target_temp.is_none()
        && target_pressure.is_none()
    {
        return Err(AppError::bad_request(
            "control params must include at least one recognized control parameter",
        ));
    }

    let heat_time_s = heat_time.unwrap_or(300.0);
    let hold_time_s = hold_time.unwrap_or(600.0);
    let cool_time_s = cool_time.unwrap_or(180.0);
    let stirrer_rpm = stir_speed.unwrap_or(800.0);
    let shake_speed_cpm = shake_speed.unwrap_or(30.0);
    let target_temperature_c = target_temp.unwrap_or(120.0);
    let target_pressure_mpa = target_pressure.unwrap_or(0.5);

    validate_range(
        "heat_time",
        heat_time_s,
        0.0,
        safety.optimizer.max_heating_minutes * 60.0,
    )?;
    validate_range(
        "hold_time",
        hold_time_s,
        0.0,
        safety.optimizer.max_stirring_minutes * 60.0,
    )?;
    validate_range("cool_time", cool_time_s, 0.0, 3600.0)?;
    validate_stir_speed(safety, stirrer_rpm)?;
    validate_range("shake_speed", shake_speed_cpm, 0.0, 60.0)?;
    validate_target_temperature(safety, target_temperature_c)?;
    validate_target_pair_allowed(safety, target_temperature_c, stirrer_rpm)?;
    validate_range("target_pressure", target_pressure_mpa, 0.0, 10.0)?;
    if running_state && stirrer_rpm == 0.0 && shake_speed_cpm == 0.0 {
        return Err(AppError::bad_request(
            "stir_speed and shake_speed cannot both be 0 while running",
        ));
    }

    Ok(ControlTargets {
        temperature_c: round2(target_temperature_c),
        heat_time_s: round2(heat_time_s),
        hold_time_s: round2(hold_time_s),
        cool_time_s: round2(cool_time_s),
        stirrer_rpm: round2(stirrer_rpm),
        shake_speed_cpm: round2(shake_speed_cpm),
        target_pressure_mpa: round2(target_pressure_mpa),
    })
}

fn optional_present_number(
    value: Option<Option<f64>>,
    field: &str,
) -> Result<Option<f64>, AppError> {
    match value {
        Some(Some(number)) if number.is_finite() => Ok(Some(number)),
        Some(Some(_)) => Err(AppError::bad_request(format!("{field} must be finite"))),
        Some(None) => Err(AppError::bad_request(format!("{field} must not be null"))),
        None => Ok(None),
    }
}

fn optional_present_bool(
    value: Option<Option<bool>>,
    field: &str,
) -> Result<Option<bool>, AppError> {
    match value {
        Some(Some(value)) => Ok(Some(value)),
        Some(None) => Err(AppError::bad_request(format!("{field} must not be null"))),
        None => Ok(None),
    }
}

fn optional_present_i64(value: Option<Option<i64>>, field: &str) -> Result<Option<i64>, AppError> {
    match value {
        Some(Some(value)) => Ok(Some(value)),
        Some(None) => Err(AppError::bad_request(format!("{field} must not be null"))),
        None => Ok(None),
    }
}

fn deserialize_optional_number_field<'de, D>(
    deserializer: D,
) -> Result<Option<Option<f64>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<f64>::deserialize(deserializer).map(Some)
}

fn deserialize_optional_bool_field<'de, D>(
    deserializer: D,
) -> Result<Option<Option<bool>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<bool>::deserialize(deserializer).map(Some)
}

fn deserialize_optional_i64_field<'de, D>(deserializer: D) -> Result<Option<Option<i64>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(Some)
}

fn deserialize_optional_string_field<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .ok_or_else(|| serde::de::Error::custom("must not be null"))
        .map(Some)
}

fn deserialize_optional_nullable_string_field<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

fn validate_v1_process_targets(
    safety: &SafetyConfig,
    targets: ControlTargets,
) -> Result<ControlTargets, AppError> {
    validate_target_temperature(safety, targets.temperature_c)?;
    validate_range(
        "heat_time_s",
        targets.heat_time_s,
        0.0,
        safety.optimizer.max_heating_minutes * 60.0,
    )?;
    validate_range(
        "hold_time_s",
        targets.hold_time_s,
        0.0,
        safety.optimizer.max_stirring_minutes * 60.0,
    )?;
    validate_range("cool_time_s", targets.cool_time_s, 0.0, 3600.0)?;
    validate_stir_speed(safety, targets.stirrer_rpm)?;
    validate_range("shake_speed", targets.shake_speed_cpm, 0.0, 60.0)?;
    validate_range("target_pressure", targets.target_pressure_mpa, 0.0, 10.0)?;
    Ok(ControlTargets {
        temperature_c: round2(targets.temperature_c),
        heat_time_s: round2(targets.heat_time_s),
        hold_time_s: round2(targets.hold_time_s),
        cool_time_s: round2(targets.cool_time_s),
        stirrer_rpm: round2(targets.stirrer_rpm),
        shake_speed_cpm: round2(targets.shake_speed_cpm),
        target_pressure_mpa: round2(targets.target_pressure_mpa),
    })
}

fn optional_number_field(value: &Value, field: &str) -> Result<Option<f64>, AppError> {
    let Some(value) = value.get(field) else {
        return Ok(None);
    };
    let number = value
        .as_f64()
        .ok_or_else(|| AppError::bad_request(format!("{field} must be a number")))?;
    if !number.is_finite() {
        return Err(AppError::bad_request(format!("{field} must be finite")));
    }
    Ok(Some(number))
}

fn validate_batch_start_targets(
    safety: &SafetyConfig,
    targets: ControlTargets,
) -> Result<ControlTargets, AppError> {
    validate_target_temperature(safety, targets.temperature_c)?;
    validate_stir_speed(safety, targets.stirrer_rpm)?;
    validate_target_pair_allowed(safety, targets.temperature_c, targets.stirrer_rpm)?;
    validate_range("target_shake_speed_cpm", targets.shake_speed_cpm, 0.0, 60.0)?;
    validate_range(
        "heating_minutes",
        targets.heat_time_s / 60.0,
        0.0,
        safety.optimizer.max_heating_minutes,
    )?;
    validate_range(
        "stirring_minutes",
        targets.hold_time_s / 60.0,
        0.0,
        safety.optimizer.max_stirring_minutes,
    )?;
    validate_range("cool_time_s", targets.cool_time_s, 0.0, 3600.0)?;
    validate_range(
        "target_pressure_mpa",
        targets.target_pressure_mpa,
        0.0,
        10.0,
    )?;
    Ok(ControlTargets {
        temperature_c: round2(targets.temperature_c),
        heat_time_s: round2(targets.heat_time_s),
        hold_time_s: round2(targets.hold_time_s),
        cool_time_s: round2(targets.cool_time_s),
        stirrer_rpm: round2(targets.stirrer_rpm),
        shake_speed_cpm: round2(targets.shake_speed_cpm),
        target_pressure_mpa: round2(targets.target_pressure_mpa),
    })
}

pub(crate) fn validate_target_temperature(
    safety: &SafetyConfig,
    value: f64,
) -> Result<(), AppError> {
    validate_range("target_temp", value, 0.0, 500.0)?;
    if value > safety.temperature.max_c {
        return Err(AppError::bad_request(format!(
            "target_temp exceeds device maximum temperature {:.1}",
            safety.temperature.max_c
        )));
    }
    Ok(())
}

pub(crate) fn validate_stir_speed(safety: &SafetyConfig, value: f64) -> Result<(), AppError> {
    validate_range("stir_speed", value, 0.0, 2000.0)?;
    if value > safety.stirrer.max_rpm {
        return Err(AppError::bad_request(format!(
            "stir_speed exceeds device maximum RPM {:.0}",
            safety.stirrer.max_rpm
        )));
    }
    Ok(())
}

fn validate_target_pair_allowed(
    safety: &SafetyConfig,
    temperature_c: f64,
    stirrer_rpm: f64,
) -> Result<(), AppError> {
    if let Some(zone) = forbidden_control_zone(safety, temperature_c, stirrer_rpm) {
        return Err(AppError::forbidden(format!(
            "target pair temp={:.1} degC rpm={:.1} enters forbidden control zone {}: {}",
            temperature_c, stirrer_rpm, zone.name, zone.reason
        )));
    }
    Ok(())
}

fn ensure_targets_allowed(safety: &SafetyConfig, targets: &ControlTargets) -> Result<(), AppError> {
    validate_target_pair_allowed(safety, targets.temperature_c, targets.stirrer_rpm)
}

pub(crate) fn validate_range(field: &str, value: f64, min: f64, max: f64) -> Result<(), AppError> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(AppError::bad_request(format!(
            "{field} must be between {min} and {max}"
        )));
    }
    Ok(())
}

fn parse_required_time(value: Option<&str>, field: &str) -> Result<DateTime<Utc>, AppError> {
    let Some(value) = value else {
        return Err(AppError::bad_request(format!("{field} is required")));
    };
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| AppError::bad_request(format!("{field} must be ISO8601")))
}

fn device_status_summary(state: &AppState, runtime: &RuntimeState) -> DeviceStatusSummary {
    let device = device_status_item("reactor_001", "reactor_bridge", state, runtime);
    let sensors = device.sensors.clone();
    let components = device.components.clone();
    DeviceStatusSummary {
        total_count: 1,
        online_count: usize::from(device.online),
        devices: vec![device],
        sensors,
        components,
    }
}

async fn device_status_summary_with_db(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<DeviceStatusSummary, AppError> {
    let mut summary = device_status_summary(state, runtime);
    apply_unfinished_batch_status(
        &unfinished_batch_status(state, runtime).await?,
        runtime,
        &mut summary,
    );
    Ok(summary)
}

fn apply_unfinished_batch_status(
    batch_status: &UnfinishedBatchStatus,
    runtime: &RuntimeState,
    summary: &mut DeviceStatusSummary,
) {
    if !batch_status.has_unfinished_batch(runtime) {
        return;
    }
    if batch_status.is_consistent() {
        return;
    }
    let reason = batch_status.reason(runtime);
    for device in &mut summary.devices {
        device.online = false;
        device.status = "error".to_string();
        device.unfinished_batch_ids = batch_status.unfinished_batch_ids.clone();
        device.unexpected_unfinished_batch_ids = batch_status.unexpected_batch_ids.clone();
        device.last_control_error = Some(match &device.last_control_error {
            Some(error) => format!("{error}; {reason}"),
            None => reason.clone(),
        });
    }
    summary.online_count = 0;
}

fn device_status_item(
    device_id: &str,
    device_role: &str,
    state: &AppState,
    runtime: &RuntimeState,
) -> DeviceStatusItem {
    let stale_after_ms = state.safety.control.sensor_timeout_ms;
    let bridge_status = runtime.device_status.as_ref();
    let (last_seen_at, last_seen_age_ms, sample_fresh) = match &runtime.latest_sample {
        Some(sample) => {
            let age = timestamp_age_ms(sample.captured_at);
            (
                Some(sample.captured_at.to_rfc3339()),
                Some(age),
                timestamp_is_fresh(sample.captured_at, stale_after_ms),
            )
        }
        None => (None, None, false),
    };
    let (last_seen_at, last_seen_age_ms) =
        if let Some(last_seen) = bridge_status.and_then(|status| status.last_seen_at.as_ref()) {
            (
                Some(last_seen.to_rfc3339()),
                Some(timestamp_age_ms(*last_seen)),
            )
        } else {
            (last_seen_at, last_seen_age_ms)
        };
    let bridge_online = bridge_status
        .map(|status| {
            status.connected
                && status.last_frame_ok
                && status
                    .last_seen_at
                    .as_ref()
                    .map(|last_seen| timestamp_is_fresh(*last_seen, stale_after_ms))
                    .unwrap_or(false)
        })
        .unwrap_or_else(|| !state.safety.control.require_device_status_for_control && sample_fresh);
    let downstream_command_fault = bridge_status
        .map(downstream_command_fault_reason)
        .unwrap_or(None)
        .is_some();
    let status = if runtime.emergency_stop {
        "error"
    } else if bridge_status
        .map(|status| !status.connected || !status.last_frame_ok)
        .unwrap_or(false)
    {
        "error"
    } else if downstream_command_fault {
        "error"
    } else if bridge_status.is_none() && state.safety.control.require_device_status_for_control {
        "offline"
    } else if !sample_fresh && !bridge_online {
        if runtime.latest_sample.is_some() {
            "stale"
        } else {
            "offline"
        }
    } else if !sample_fresh && runtime.last_sensor_error.is_some() {
        "error"
    } else if runtime.last_sensor_error.is_some() {
        "error"
    } else if runtime.last_control_error.is_some() {
        "error"
    } else if runtime.active_batch_id.is_some() {
        "running"
    } else {
        "idle"
    };

    let sensors = sensor_items(state, runtime, sample_fresh);
    let components = component_items(state, runtime);

    DeviceStatusItem {
        device_id: device_id.to_string(),
        device_role: device_role.to_string(),
        online: bridge_online && !runtime.emergency_stop && !downstream_command_fault,
        status: status.to_string(),
        auto_enabled: runtime.auto_enabled,
        manual_lock: runtime.manual_lock,
        last_seen_at,
        last_seen_age_ms,
        stale_after_ms,
        active_batch_id: runtime.active_batch_id,
        emergency_stop: runtime.emergency_stop,
        last_sensor_error: runtime.last_sensor_error.clone(),
        last_control_error: runtime.last_control_error.clone(),
        unfinished_batch_ids: Vec::new(),
        unexpected_unfinished_batch_ids: Vec::new(),
        relay: bridge_status.and_then(|status| status.relay),
        motor: bridge_status.and_then(|status| status.motor),
        tilt: bridge_status.and_then(|status| status.tilt),
        speed_delay_us: bridge_status.and_then(|status| status.speed_delay_us),
        port: bridge_status.and_then(|status| status.port.clone()),
        baudrate: bridge_status.and_then(|status| status.baudrate),
        last_command_request_id: bridge_status
            .and_then(|status| status.last_command_request_id.clone()),
        last_command_ok: bridge_status.and_then(|status| status.last_command_ok),
        last_command_error: bridge_status.and_then(|status| status.last_command_error.clone()),
        sensors,
        components,
    }
}

fn sensor_items(
    state: &AppState,
    runtime: &RuntimeState,
    sample_fresh: bool,
) -> Vec<DeviceSensorItem> {
    let sample = runtime.latest_sample.as_ref();
    let sample_updated_at = sample.map(|sample| sample.captured_at.to_rfc3339());
    let status_for = |value: Option<f64>, stale_status: &str| -> String {
        if runtime.emergency_stop {
            "blocked".to_string()
        } else if runtime.last_sensor_error.is_some() {
            "error".to_string()
        } else if value.is_none() {
            "unavailable".to_string()
        } else if !sample_fresh {
            stale_status.to_string()
        } else {
            "online".to_string()
        }
    };
    let item = |sensor_id: &str,
                label: &str,
                unit: &str,
                value: Option<f64>,
                target: Option<f64>,
                component_id: Option<&str>|
     -> DeviceSensorItem {
        DeviceSensorItem {
            sensor_id: sensor_id.to_string(),
            label: label.to_string(),
            unit: unit.to_string(),
            status: status_for(value, "stale"),
            value: value.map(round2),
            target: target.map(round2),
            source: device_mode_label(&state.device_mode).to_string(),
            component_id: component_id.map(ToString::to_string),
            updated_at: sample_updated_at.clone(),
        }
    };
    vec![
        item(
            "temperature_c",
            "Temperature",
            "C",
            sample.map(|sample| sample.temperature_c),
            Some(runtime.targets.temperature_c),
            Some("temperature_controller"),
        ),
        item(
            "pressure_mpa",
            "Pressure",
            "MPa",
            sample.map(|sample| sample.pressure_mpa),
            Some(runtime.targets.target_pressure_mpa),
            None,
        ),
        item(
            "stirrer_rpm",
            "Stirrer RPM",
            "RPM",
            sample.map(|sample| sample.stirrer_rpm),
            Some(runtime.targets.stirrer_rpm),
            Some("stirrer_motor"),
        ),
        item(
            "shake_speed_cpm",
            "Shake Vessel Speed",
            "CPM",
            sample.map(|sample| sample.shake_speed_cpm),
            Some(runtime.targets.shake_speed_cpm),
            Some("shake_stepper"),
        ),
        item(
            "tilt_state",
            "Tilt Binary State",
            "0/1",
            sample.map(|sample| sample.tilt_state as f64),
            None,
            Some("shake_stepper"),
        ),
        item(
            "tilt_angle_deg",
            "Fitted Tilt Angle",
            "deg",
            sample.map(|sample| sample.tilt_angle_deg),
            None,
            Some("shake_stepper"),
        ),
        item(
            "flow_rate_l_min",
            "Flow Rate",
            "L/min",
            sample.map(|sample| sample.flow_rate_l_min),
            None,
            None,
        ),
        item(
            "product_concentration_percent",
            "Product Concentration",
            "%",
            sample.map(|sample| sample.product_concentration_percent),
            None,
            None,
        ),
        item("ph", "pH", "", sample.map(|sample| sample.ph), None, None),
    ]
}

fn component_items(state: &AppState, runtime: &RuntimeState) -> Vec<DeviceComponentItem> {
    let status = runtime.device_status.as_ref();
    let device_status_required = state.safety.control.require_device_status_for_control;
    state
        .device
        .control_capabilities()
        .into_iter()
        .map(|capability| {
            let state_value = match capability.component_id.as_str() {
                "shake_stepper" => json!({
                    "motor": status.and_then(|status| status.motor),
                    "tilt": status.and_then(|status| status.tilt),
                    "speed_delay_us": status.and_then(|status| status.speed_delay_us),
                    "target_shake_speed_cpm": runtime.targets.shake_speed_cpm,
                    "current_shake_speed_cpm": runtime.latest_sample.as_ref().map(|sample| sample.shake_speed_cpm)
                }),
                "heater_relay" => json!({
                    "relay": status.and_then(|status| status.relay),
                    "target_temperature_c": runtime.targets.temperature_c,
                    "current_temperature_c": runtime.latest_sample.as_ref().map(|sample| sample.temperature_c)
                }),
                "temperature_controller" => json!({
                    "target_temperature_c": runtime.targets.temperature_c,
                    "current_temperature_c": runtime.latest_sample.as_ref().map(|sample| sample.temperature_c)
                }),
                "stirrer_motor" => json!({
                    "target_stirrer_rpm": runtime.targets.stirrer_rpm,
                    "current_stirrer_rpm": runtime.latest_sample.as_ref().map(|sample| sample.stirrer_rpm)
                }),
                _ => json!({}),
            };
            let component_status = if runtime.emergency_stop {
                "blocked"
            } else if runtime.manual_lock {
                "locked"
            } else if status.is_none() && device_status_required {
                "unavailable"
            } else if status
                .map(|status| !status.connected || !status.last_frame_ok)
                .unwrap_or(false)
            {
                "error"
            } else if status
                .and_then(downstream_command_fault_reason)
                .is_some()
            {
                "error"
            } else {
                match capability.component_id.as_str() {
                    "shake_stepper" if status.and_then(|status| status.motor) == Some(1) => {
                        "running"
                    }
                    "heater_relay" if status.and_then(|status| status.relay) == Some(1) => "on",
                    "stirrer_motor" if runtime
                        .latest_sample
                        .as_ref()
                        .map(|sample| sample.stirrer_rpm > 0.01)
                        .unwrap_or(false) =>
                    {
                        "running"
                    }
                    _ => "idle",
                }
            };
            DeviceComponentItem {
                component_id: capability.component_id,
                component_type: capability.component_type,
                label: capability.label,
                controllable: capability.controllable,
                status: component_status.to_string(),
                state: state_value,
                actions: capability
                    .actions
                    .into_iter()
                    .map(|action| ComponentActionItem {
                        action: action.action,
                        label: action.label,
                        value_type: action.value_type,
                        min: action.min,
                        max: action.max,
                        unit: action.unit,
                    })
                    .collect(),
            }
        })
        .collect()
}

fn safe_command_from_runtime_targets(targets: &ControlTargets, reason: &str) -> SafeCommand {
    SafeCommand {
        target_temperature_c: targets.temperature_c,
        heat_time_s: targets.heat_time_s,
        hold_time_s: targets.hold_time_s,
        cool_time_s: targets.cool_time_s,
        target_stirrer_rpm: targets.stirrer_rpm,
        target_shake_speed_cpm: targets.shake_speed_cpm,
        target_pressure_mpa: targets.target_pressure_mpa,
        reason: reason.to_string(),
    }
}

fn device_mode_label(mode: &DeviceMode) -> &'static str {
    match mode {
        DeviceMode::Pipeline => "pipeline",
        DeviceMode::Modbus => "modbus",
        DeviceMode::Esp32Serial => "esp32_serial",
        DeviceMode::JsonBridge => "json_bridge",
    }
}

fn phase_for(runtime: &RuntimeState, device_online: bool) -> &'static str {
    if !device_online {
        return "offline";
    }
    if runtime.emergency_stop
        || runtime.last_sensor_error.is_some()
        || runtime.last_control_error.is_some()
    {
        "error"
    } else if runtime.active_batch_id.is_some() {
        "heating"
    } else {
        "idle"
    }
}

fn progress_for(sample: Option<&SensorSnapshot>) -> f64 {
    sample
        .map(|sample| sample.product_concentration_percent.clamp(0.0, 100.0))
        .unwrap_or(0.0)
}

pub(crate) fn alarms_for(
    safety: &SafetyConfig,
    runtime: &RuntimeState,
    sample: Option<&SensorSnapshot>,
    memory: &AiMemory,
) -> Vec<Value> {
    let mut alarms = Vec::new();
    if let Some(reason) = sensor_data_alarm_reason(safety, runtime, sample) {
        alarms.push(json!({
            "type": "sensor_data_unavailable",
            "level": "high",
            "message": reason,
            "suggestion": "restore fresh persisted sensor data before enabling control or continuing production decisions"
        }));
    }
    if let Some(reason) = device_status_alarm_reason(safety, runtime) {
        alarms.push(json!({
            "type": "device_status_unavailable",
            "level": "high",
            "message": reason,
            "suggestion": "verify downstream controller status before enabling control or starting production"
        }));
    }
    if let Some(reason) = runtime
        .device_status
        .as_ref()
        .and_then(downstream_command_fault_reason)
    {
        alarms.push(json!({
            "type": "downstream_command_fault",
            "level": "high",
            "message": reason,
            "suggestion": "confirm actuator state and clear the downstream command fault before resuming production"
        }));
    }
    if runtime.emergency_stop {
        alarms.push(json!({
            "type": "emergency_stop",
            "level": "high",
            "message": "manual emergency stop is active",
            "suggestion": "confirm field safety before resetting emergency stop"
        }));
    }
    if let Some(error) = &runtime.last_control_error {
        alarms.push(json!({
            "type": "communication_error",
            "level": "medium",
            "message": error
        }));
    }
    if let Some(error) = &runtime.last_sensor_error {
        alarms.push(json!({
            "type": "sensor_error",
            "level": "high",
            "message": error
        }));
    }
    if let Some(sample) = sample {
        push_sensor_alarm(
            &mut alarms,
            "temperature_limit",
            memory.sensor_limits.temperature_c.as_ref(),
            sample.temperature_c,
        );
        push_sensor_alarm(
            &mut alarms,
            "pressure_limit",
            memory.sensor_limits.pressure_mpa.as_ref(),
            sample.pressure_mpa,
        );
        push_sensor_alarm(
            &mut alarms,
            "stirrer_limit",
            memory.sensor_limits.stirrer_rpm.as_ref(),
            sample.stirrer_rpm,
        );
        push_sensor_alarm(
            &mut alarms,
            "shake_speed_limit",
            memory.sensor_limits.shake_speed_cpm.as_ref(),
            sample.shake_speed_cpm,
        );
        push_sensor_alarm(
            &mut alarms,
            "tilt_angle_limit",
            memory.sensor_limits.tilt_angle_deg.as_ref(),
            sample.tilt_angle_deg,
        );
        push_sensor_alarm(
            &mut alarms,
            "flow_rate_limit",
            memory.sensor_limits.flow_rate_l_min.as_ref(),
            sample.flow_rate_l_min,
        );
        push_sensor_alarm(
            &mut alarms,
            "product_concentration_limit",
            memory.sensor_limits.product_concentration_percent.as_ref(),
            sample.product_concentration_percent,
        );
        push_sensor_alarm(
            &mut alarms,
            "ph_limit",
            memory.sensor_limits.ph.as_ref(),
            sample.ph,
        );
    }
    alarms
}

pub(crate) async fn live_alarms_for(
    state: &AppState,
    safety: &SafetyConfig,
    runtime: &RuntimeState,
    sample: Option<&SensorSnapshot>,
    memory: &AiMemory,
) -> Result<Vec<Value>, AppError> {
    let mut alarms = alarms_for(safety, runtime, sample, memory);
    append_unfinished_batch_recovery_alarms(state, runtime, &mut alarms).await?;
    Ok(alarms)
}

async fn append_unfinished_batch_recovery_alarms(
    state: &AppState,
    runtime: &RuntimeState,
    alarms: &mut Vec<Value>,
) -> Result<(), AppError> {
    let batch_status = unfinished_batch_status(state, runtime).await?;
    if !batch_status.recovery_required() {
        return Ok(());
    }
    alarms.push(json!({
        "type": "unfinished_batch_recovery",
        "level": "high",
        "message": batch_status.reason(runtime),
        "active_batch_id": runtime.active_batch_id,
        "unfinished_batch_ids": batch_status.unfinished_batch_ids,
        "unexpected_batch_ids": batch_status.unexpected_batch_ids,
        "runtime_active_batch_missing": batch_status.runtime_active_batch_missing,
        "suggestion": "verify field state and close or repair unfinished batches before starting production"
    }));
    Ok(())
}

fn sensor_data_alarm_reason(
    safety: &SafetyConfig,
    runtime: &RuntimeState,
    sample: Option<&SensorSnapshot>,
) -> Option<String> {
    if let Some(error) = &runtime.last_sensor_error {
        return Some(format!("sensor data unavailable; {error}"));
    }
    let Some(sample) = sample else {
        return Some(
            "sensor data unavailable; no persisted pipeline sample is available".to_string(),
        );
    };
    let age_ms = timestamp_age_ms(sample.captured_at);
    if age_ms < 0 {
        return Some(format!(
            "sensor data timestamp is {} ms in the future; field clock state is not trusted",
            -age_ms
        ));
    }
    if age_ms > safety.control.sensor_timeout_ms {
        return Some(format!(
            "sensor data stale; last persisted sample is {age_ms} ms old, max {} ms",
            safety.control.sensor_timeout_ms
        ));
    }
    None
}

fn device_status_alarm_reason(safety: &SafetyConfig, runtime: &RuntimeState) -> Option<String> {
    let Some(status) = runtime.device_status.as_ref() else {
        if safety.control.require_device_status_for_control {
            return Some(
                "downstream device status unavailable; field state is not proven safe".to_string(),
            );
        }
        return None;
    };
    device_status_field_fault_reason(status, safety.control.sensor_timeout_ms)
}

fn push_sensor_alarm(
    alarms: &mut Vec<Value>,
    alarm_type: &str,
    limit: Option<&SensorLimit>,
    value: f64,
) {
    let Some(limit) = limit else {
        return;
    };
    let Some(alarm) = limit.check(value) else {
        return;
    };
    alarms.push(json!({
        "type": alarm_type,
        "level": match alarm.level {
            LimitLevel::Warning => "medium",
            LimitLevel::High => "high",
        },
        "message": alarm.message,
        "current_value": alarm.current_value,
        "limit_value": alarm.limit_value,
        "suggestion": alarm.suggestion
    }));
}
