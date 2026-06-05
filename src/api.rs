use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{any, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
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
    local_ai::LocalAiStatus,
    memory::{AiMemory, AiMemorySummary, LimitLevel, SensorLimit},
    number::round2,
    optimizer::{recommend_with_memory, Recommendation},
    reports::{
        build_audit_csv, build_batch_report_markdown, build_batches_csv, build_batches_xlsx,
    },
    state::{fit_tilt_angle_deg, ControlTargets, RuntimeState, SensorSnapshot, SharedState},
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
    pub latest_recommendation: Option<Recommendation>,
    pub ai_provider: AiRecommendationProvider,
    pub processes: Vec<ProcessDefinition>,
    pub recent_samples: Vec<SensorSampleRecord>,
    pub recent_batches: Vec<Batch>,
    pub recent_outcomes: Vec<BatchOutcome>,
    pub recent_events: Vec<ControlEvent>,
    pub alarms: Vec<Value>,
    pub ai_memory: AiMemorySummary,
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
    pub last_seen_at: Option<String>,
    pub last_seen_age_ms: Option<i64>,
    pub stale_after_ms: i64,
    pub active_batch_id: Option<i64>,
    pub emergency_stop: bool,
    pub last_sensor_error: Option<String>,
    pub last_control_error: Option<String>,
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
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ComponentControlResponse {
    pub device_id: String,
    pub component: DeviceComponentItem,
    pub outcome: Option<ComponentControlOutcome>,
}

#[derive(Debug, Deserialize)]
pub struct StartBatchRequest {
    pub name: Option<String>,
    pub process_id: Option<i64>,
    pub target_temperature_c: Option<f64>,
    pub target_stirrer_rpm: Option<f64>,
    pub target_shake_speed_cpm: Option<f64>,
    pub heating_minutes: Option<f64>,
    pub stirring_minutes: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct ProductResultRequest {
    pub batch_id: i64,
    pub yield_percent: f64,
    pub product_ratio: f64,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProcessRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProcessRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProcessStepRequest {
    pub name: Option<String>,
    pub target_temperature_c: f64,
    pub ramp_rate_c_min: Option<f64>,
    pub duration_minutes: f64,
    pub target_stirrer_rpm: f64,
    pub target_shake_speed_cpm: Option<f64>,
    pub target_pressure_mpa: Option<f64>,
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
    pub batch: Batch,
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
pub struct ManualLockRequest {
    pub locked: bool,
}

#[derive(Debug, Deserialize)]
pub struct TargetRequest {
    pub temperature_c: f64,
    pub stirrer_rpm: f64,
    pub shake_speed_cpm: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct V1ControlRequest {
    pub command_id: Option<String>,
    pub timestamp: Option<String>,
    pub params: V1ControlParams,
    pub priority: Option<String>,
    pub auto_start: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct V1ControlParams {
    pub heat_time: Option<f64>,
    pub hold_time: Option<f64>,
    pub cool_time: Option<f64>,
    pub stir_speed: Option<f64>,
    pub shake_speed: Option<f64>,
    pub target_temp: Option<f64>,
    pub target_pressure: Option<f64>,
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
    pub intent: Option<String>,
    pub mode: Option<String>,
    pub dry_run: Option<bool>,
    pub allow_process_start: Option<bool>,
    pub allow_process_stop: Option<bool>,
    pub allow_component_control: Option<bool>,
    pub allow_target_adjustment: Option<bool>,
    pub preferred_process_id: Option<i64>,
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
struct AiControlAuditReason<'a> {
    decision: &'a str,
    rationale: &'a str,
    actions: Vec<AiControlAuditAction<'a>>,
}

#[derive(Debug, Serialize)]
struct AiControlAuditAction<'a> {
    action_type: &'a str,
    target: &'a str,
    status: &'a str,
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
        state.db.list_processes()?
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
        state.db.recent_control_events(100)?
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
    let alarms = alarms_for(
        &runtime,
        runtime.latest_sample.as_ref(),
        state.ai_memory.as_ref(),
    );
    Ok(Json(LiveResponse {
        runtime,
        latest_recommendation: recommendation,
        ai_provider,
        processes,
        recent_samples,
        recent_batches,
        recent_outcomes,
        recent_events,
        alarms,
        ai_memory,
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
        processes: state.db.list_processes()?,
        recent_batches: state.db.recent_batches_sqlx(20).await?,
        recent_outcomes: state.db.recent_batch_outcomes_sqlx(20).await?,
        recent_events: state.db.recent_control_events(100)?,
        demo_alarms: state.db.recent_demo_alarms(20)?,
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
    Json(success(json!({
        "device_mode": state.device_mode,
        "device": state.device_config.as_ref(),
        "safety": state.safety.as_ref(),
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

async fn modbus_registers(State(state): State<AppState>) -> Json<V1Envelope<Value>> {
    let runtime = state.runtime.read().await.clone();
    let tcp_status = crate::modbus_tcp::modbus_tcp_status_snapshot().await;
    Json(success(modbus_register_map::registers_payload(
        &state,
        &runtime,
        &tcp_status,
    )))
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
    Ok(Json(success(state.db.list_processes()?)))
}

async fn create_process(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<CreateProcessRequest>,
) -> Result<Json<V1Envelope<ProcessDefinition>>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    let name = clean_label(payload.name, "未命名工艺", 80);
    let description = clean_label(payload.description, "", 240);
    let process = state.db.create_process(&name, &description)?;
    state
        .db
        .insert_control_event(None, "process_created", None, "operator created process")?;
    Ok(Json(success(process)))
}

async fn get_process(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
) -> Result<Json<V1Envelope<ProcessDetail>>, AppError> {
    let Some(process) = state.db.process_detail(process_id)? else {
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
    let Some(current) = state.db.process_detail(process_id)? else {
        return Err(AppError::not_found("process not found"));
    };
    let name = clean_label(payload.name, &current.process.name, 80);
    let description = clean_label(payload.description, &current.process.description, 240);
    let status = clean_status(payload.status.as_deref().unwrap_or(&current.process.status))?;
    let Some(process) = state
        .db
        .update_process(process_id, &name, &description, status)?
    else {
        return Err(AppError::not_found("process not found"));
    };
    state
        .db
        .insert_control_event(None, "process_updated", None, "operator updated process")?;
    Ok(Json(success(process)))
}

async fn add_process_step(
    State(state): State<AppState>,
    Path(process_id): Path<i64>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ProcessStepRequest>,
) -> Result<Json<V1Envelope<ProcessStep>>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    let step = validate_process_step(&state.safety, payload)?;
    let Some(step) = state.db.add_process_step(process_id, &step)? else {
        return Err(AppError::not_found("process not found"));
    };
    state.db.insert_control_event(
        None,
        "process_step_added",
        None,
        "operator added process step",
    )?;
    Ok(Json(success(step)))
}

async fn update_process_step(
    State(state): State<AppState>,
    Path((process_id, step_id)): Path<(i64, i64)>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ProcessStepRequest>,
) -> Result<Json<V1Envelope<ProcessStep>>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
    let step = validate_process_step(&state.safety, payload)?;
    let Some(step) = state.db.update_process_step(process_id, step_id, &step)? else {
        return Err(AppError::not_found("process step not found"));
    };
    state.db.insert_control_event(
        None,
        "process_step_updated",
        None,
        "operator updated process step",
    )?;
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
) -> Result<Json<V1Envelope<ProcessStopResponse>>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let response = stop_process_lifecycle(&state, Some(process_id), "process_stopped").await?;
    Ok(Json(success(response)))
}

async fn stop_current_process(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<V1Envelope<ProcessStopResponse>>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let response = stop_process_lifecycle(&state, None, "process_stopped").await?;
    Ok(Json(success(response)))
}

async fn start_process_lifecycle(
    state: &AppState,
    process_id: i64,
    event_type: &'static str,
) -> Result<ProcessApplyResponse, AppError> {
    {
        let runtime = state.runtime.read().await;
        ensure_process_can_start(state, &runtime)?;
    }
    let Some(detail) = state.db.process_detail(process_id)? else {
        return Err(AppError::not_found("process not found"));
    };
    if detail.steps.is_empty() {
        return Err(AppError::bad_request(
            "process must contain at least one step before starting",
        ));
    }
    let targets = targets_from_process_steps(&state.safety, &detail.steps)?;
    let batch = state.db.create_batch_for_process(
        Some(process_id),
        &detail.process.name,
        targets.temperature_c,
        targets.stirrer_rpm,
        seconds_to_minutes(Some(targets.heat_time_s)),
        seconds_to_minutes(Some(targets.hold_time_s)),
    )?;
    let start_reason = if event_type == "process_applied" {
        "process applied from persisted process definition"
    } else if event_type == "ainas_process_started" {
        "process started by AINAS remote task"
    } else {
        "process started from persisted process definition"
    };
    if let Err(err) = start_process_on_device(state, &targets).await {
        let error_message = err.message().to_string();
        if let Err(audit_err) = state.db.insert_control_event(
            Some(batch.id),
            "process_start_failed",
            Some(&SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: format!("process start failed before activation: {error_message}"),
            }),
            "process start failed before activation",
        ) {
            tracing::warn!("failed to persist process_start_failed audit event: {audit_err}");
        }
        if let Err(finish_err) = state.db.finish_batch(batch.id) {
            tracing::warn!("failed to mark failed process start batch finished: {finish_err}");
        }
        return Err(err);
    }
    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
        runtime.active_batch_id = Some(batch.id);
        runtime.auto_enabled = true;
    }
    if let Err(err) = state.db.insert_control_event(
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
    ) {
        rollback_failed_activation(state, batch.id).await;
        return Err(err.into());
    }
    let process = match state.db.mark_process_applied(process_id) {
        Ok(Some(process)) => process,
        Ok(None) => {
            rollback_failed_activation(state, batch.id).await;
            return Err(AppError::not_found("process not found"));
        }
        Err(err) => {
            rollback_failed_activation(state, batch.id).await;
            return Err(err.into());
        }
    };
    Ok(ProcessApplyResponse {
        process,
        batch,
        applied_targets: targets,
        status: "running".to_string(),
    })
}

async fn rollback_failed_activation(state: &AppState, batch_id: i64) {
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id == Some(batch_id) {
            runtime.active_batch_id = None;
            runtime.auto_enabled = false;
        }
    }
    if let Err(err) = state.db.finish_batch(batch_id) {
        tracing::warn!("failed to mark failed activation batch finished: {err}");
    }
}

async fn rollback_v1_auto_start_activation(state: &AppState, batch_id: Option<i64>) {
    let Some(batch_id) = batch_id else {
        return;
    };
    rollback_failed_activation(state, batch_id).await;
}

async fn stop_process_lifecycle(
    state: &AppState,
    expected_process_id: Option<i64>,
    event_type: &'static str,
) -> Result<ProcessStopResponse, AppError> {
    let (batch_id, targets) = {
        let runtime = state.runtime.read().await;
        let Some(batch_id) = runtime.active_batch_id else {
            return Err(AppError::conflict("no active process batch to stop"));
        };
        (batch_id, runtime.targets.clone())
    };

    let Some(batch) = state.db.batch_by_id(batch_id)? else {
        return Err(AppError::not_found("active batch not found"));
    };
    if let Some(process_id) = expected_process_id {
        if batch.process_id != Some(process_id) {
            return Err(AppError::conflict(format!(
                "active batch belongs to process {:?}, not process {process_id}",
                batch.process_id
            )));
        }
    }

    let stopped_targets = process_stop_targets(state, &targets);
    stop_process_on_device(state, &stopped_targets).await?;
    state.db.finish_batch(batch_id)?;
    let Some(batch) = state.db.batch_by_id(batch_id)? else {
        return Err(AppError::not_found("stopped batch not found"));
    };
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id == Some(batch_id) {
            runtime.active_batch_id = None;
        }
        runtime.auto_enabled = false;
    }
    state.db.insert_control_event(
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
            reason: stop_process_reason(event_type).to_string(),
        }),
        stop_process_reason(event_type),
    )?;
    Ok(ProcessStopResponse {
        stopped_batch_id: batch_id,
        process_id: batch.process_id,
        batch,
        active_batch_id: None,
        auto_enabled: false,
        stopped_targets,
    })
}

fn ensure_process_can_start(state: &AppState, runtime: &RuntimeState) -> Result<(), AppError> {
    if runtime.active_batch_id.is_some() {
        return Err(AppError::conflict(
            "device is busy running an active process batch",
        ));
    }
    if runtime.emergency_stop {
        return Err(AppError::conflict(
            "emergency stop is active; process start blocked",
        ));
    }
    if runtime.manual_lock {
        return Err(AppError::conflict(
            "manual lock is active; process start blocked",
        ));
    }
    ensure_fresh_sample(state, runtime)
}

async fn start_process_on_device(
    state: &AppState,
    targets: &ControlTargets,
) -> Result<(), AppError> {
    let command = safe_command_from_runtime_targets(
        targets,
        "process start target write accepted by safety gate",
    );
    state.device.write_targets(&command).await.map_err(|err| {
        AppError::service_unavailable(format!("device process start command failed: {err}"))
    })
}

async fn stop_process_on_device(
    state: &AppState,
    targets: &ControlTargets,
) -> Result<(), AppError> {
    let command = safe_command_from_runtime_targets(targets, "process stop target write");
    state.device.write_targets(&command).await.map_err(|err| {
        AppError::service_unavailable(format!("device process stop command failed: {err}"))
    })
}

fn process_stop_targets(state: &AppState, current: &ControlTargets) -> ControlTargets {
    let mut targets = clamp_operator_targets(
        &state.safety,
        ControlTargets {
            temperature_c: state.safety.temperature.min_c,
            heat_time_s: 0.0,
            hold_time_s: 0.0,
            cool_time_s: 0.0,
            stirrer_rpm: state.safety.stirrer.min_rpm,
            shake_speed_cpm: 0.0,
            target_pressure_mpa: current.target_pressure_mpa,
        },
    );
    targets.heat_time_s = 0.0;
    targets.hold_time_s = 0.0;
    targets.cool_time_s = 0.0;
    targets.shake_speed_cpm = 0.0;
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
    let Some(batch) = state.db.batch_by_id(batch_id)? else {
        return Err(AppError::not_found("batch not found"));
    };
    Ok(Json(success(BatchDetailResponse {
        outcome: state.db.batch_outcome_by_id(batch_id)?,
        samples: state
            .db
            .sample_records_for_batch_sqlx(batch_id, 480)
            .await?,
        events: state.db.control_events_for_batch(batch_id, 100)?,
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
    let Some(batch) = state.db.batch_by_id(batch_id)? else {
        return Err(AppError::not_found("batch not found"));
    };
    let outcome = state.db.batch_outcome_by_id(batch_id)?;
    let samples = state
        .db
        .sample_records_for_batch_sqlx(batch_id, 10_000)
        .await?;
    let events = state.db.control_events_for_batch(batch_id, 500)?;
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
    Ok(Json(success(device_status_summary(&state, &runtime))))
}

async fn devices_capabilities(
    State(state): State<AppState>,
) -> Result<Json<V1Envelope<DeviceCapabilitiesResponse>>, AppError> {
    let runtime = state.runtime.read().await.clone();
    let summary = device_status_summary(&state, &runtime);
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
    if device_id != "reactor_001" {
        return Err(AppError::not_found("device not found"));
    }

    let runtime = state.runtime.read().await.clone();
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
    if !component
        .actions
        .iter()
        .any(|action| action.action == payload.action)
    {
        return Err(AppError::bad_request("component action is not supported"));
    }
    if runtime.emergency_stop {
        return Err(AppError::conflict(
            "emergency stop is active; component control blocked",
        ));
    }
    if runtime.manual_lock {
        return Err(AppError::conflict(
            "manual lock is active; component control blocked",
        ));
    }
    if runtime
        .device_status
        .as_ref()
        .map(|status| !status.connected || !status.last_frame_ok)
        .unwrap_or(false)
    {
        return Err(AppError::service_unavailable(
            "device status is not healthy; component control blocked",
        ));
    }

    let command = ComponentControlCommand {
        component_id: component_id.to_string(),
        action: payload.action.clone(),
        value: payload.value.clone(),
    };
    let outcome = state
        .device
        .write_component(&command, &runtime.targets, &state.safety)
        .await?;

    if let Some(outcome) = &outcome {
        if let Some(targets) = &outcome.targets {
            let mut runtime = state.runtime.write().await;
            runtime.targets = ControlTargets {
                temperature_c: targets.target_temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                stirrer_rpm: targets.target_stirrer_rpm,
                shake_speed_cpm: targets.target_shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
            };
        }
    }

    let audit_reason = payload.reason.unwrap_or_else(|| {
        format!(
            "operator component control {}:{}",
            component_id, payload.action
        )
    });
    let audit_command = outcome
        .as_ref()
        .and_then(|outcome| outcome.targets.clone())
        .unwrap_or_else(|| safe_command_from_runtime_targets(&runtime.targets, &audit_reason));
    state.db.insert_control_event(
        runtime.active_batch_id,
        event_type,
        Some(&audit_command),
        &audit_reason,
    )?;

    Ok(ComponentControlResponse {
        device_id: device_id.to_string(),
        component,
        outcome,
    })
}

async fn ai_control(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<AiControlRequest>,
) -> Result<Json<V1Envelope<AiControlResponse>>, AppError> {
    require_permission(&headers, Permission::ApplyAiSuggestion)?;
    let dry_run = payload.dry_run.unwrap_or(true);
    let allow_process_start = payload.allow_process_start.unwrap_or(true);
    let allow_process_stop = payload.allow_process_stop.unwrap_or(true);
    let allow_component_control = payload.allow_component_control.unwrap_or(true);
    let allow_target_adjustment = payload.allow_target_adjustment.unwrap_or(true);
    let requested_intent = payload
        .intent
        .as_deref()
        .or(payload.mode.as_deref())
        .unwrap_or("optimize_and_control")
        .to_string();

    let runtime = state.runtime.read().await.clone();
    ensure_fresh_sample(&state, &runtime)?;
    let safety = ai_control_safety(&state, &runtime);
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
    if !safety.device_online {
        return Err(AppError::service_unavailable(
            "device is offline or unhealthy; AI master control blocked",
        ));
    }
    let process_start_blocked_by_alarm =
        safety.high_alarm_count > 0 && allow_process_start && runtime.active_batch_id.is_none();

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
            state.db.insert_recommendation(&recommendation)?;
            Some(recommendation)
        }
        None => None,
    };
    let recommended_targets = recommendation
        .as_ref()
        .map(|recommendation| ai_targets_from_recommendation(&state, &runtime, recommendation));
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

    let selected_process = select_ai_process(&state, payload.preferred_process_id)?;
    if runtime.active_batch_id.is_none() && allow_process_start && !process_start_blocked_by_alarm {
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
    } else if process_start_blocked_by_alarm {
        actions.push(AiControlAction {
            action_type: "process_start".to_string(),
            target: "/api/processes/:id/start".to_string(),
            status: "blocked".to_string(),
            message: "high level alarm is active; AI process start blocked".to_string(),
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
                let stopped = stop_process_lifecycle(&state, None, "ai_process_stopped").await?;
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
        state.db.insert_control_event(
            state.runtime.read().await.active_batch_id,
            "ai_master_decision",
            audit_command.as_ref(),
            &audit_reason,
        )?;
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

fn ai_control_audit_reason(
    decision: &str,
    rationale: &str,
    actions: &[AiControlAction],
) -> Result<String, AppError> {
    let reason = AiControlAuditReason {
        decision,
        rationale,
        actions: actions
            .iter()
            .map(|action| AiControlAuditAction {
                action_type: action.action_type.as_str(),
                target: action.target.as_str(),
                status: action.status.as_str(),
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
    let recommendation = generate_recommendation(&state).await?;
    state.db.insert_recommendation(&recommendation)?;
    let plan = build_experiment_plan(&state, recommendation).await?;
    Ok(Json(success(plan)))
}

async fn build_experiment_plan(
    state: &AppState,
    recommendation: Recommendation,
) -> Result<ExperimentPlanResponse, AppError> {
    let runtime = state.runtime.read().await.clone();
    let targets = ai_targets_from_recommendation(state, &runtime, &recommendation);
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
    let lora_note = if local_ai.ready_for_inference && local_ai.ready_for_training {
        "Local Qwen LoRA assets are ready for inference/training boundary checks.".to_string()
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

fn ai_control_safety(state: &AppState, runtime: &RuntimeState) -> AiControlSafety {
    let alarms = alarms_for(
        runtime,
        runtime.latest_sample.as_ref(),
        state.ai_memory.as_ref(),
    );
    let high_alarm_count = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("high"))
        .count();
    let warning_alarm_count = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("medium"))
        .count();
    AiControlSafety {
        fresh_sample_required: true,
        sensor_fresh: ensure_fresh_sample(state, runtime).is_ok(),
        emergency_stop: runtime.emergency_stop,
        manual_lock: runtime.manual_lock,
        device_online: device_status_summary(state, runtime).online_count > 0,
        active_batch_id: runtime.active_batch_id,
        high_alarm_count,
        warning_alarm_count,
        stop_product_concentration_percent: state
            .safety
            .control
            .ai_stop_product_concentration_percent,
    }
}

fn ai_targets_from_recommendation(
    state: &AppState,
    runtime: &RuntimeState,
    recommendation: &Recommendation,
) -> ControlTargets {
    clamp_operator_targets(
        &state.safety,
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
    ensure_targets_allowed(&state.safety, &targets)?;
    let command = safe_command_from_runtime_targets(&targets, reason);
    state.device.write_targets(&command).await.map_err(|err| {
        AppError::service_unavailable(format!("AI target write to device failed: {err}"))
    })?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
    }
    state
        .db
        .insert_control_event(None, "ai_targets_updated", Some(&command), reason)?;
    Ok(())
}

fn select_ai_process(
    state: &AppState,
    preferred_process_id: Option<i64>,
) -> Result<Option<ProcessDefinition>, AppError> {
    if let Some(process_id) = preferred_process_id {
        let Some(detail) = state.db.process_detail(process_id)? else {
            return Err(AppError::not_found("preferred process not found"));
        };
        if detail.steps.is_empty() {
            return Err(AppError::bad_request(
                "preferred process must contain at least one step",
            ));
        }
        return Ok(Some(detail.process));
    }
    let processes = state.db.list_processes()?;
    if let Some(process) = processes
        .iter()
        .find(|process| process.step_count > 0 && process.status == "applied")
        .cloned()
    {
        return Ok(Some(process));
    }
    Ok(processes.into_iter().find(|process| process.step_count > 0))
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
            reason: Some("AI master control started shake vessel stepper".to_string()),
        })
    } else if target_shake_speed <= 0.01 && motor_running {
        Some(ComponentControlRequest {
            action: "stop".to_string(),
            value: None,
            reason: Some("AI master control stopped shake vessel stepper".to_string()),
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
    require_permission(&headers, Permission::SetSafeTargets)?;
    let auto_start = payload.auto_start.unwrap_or(false);
    let already_running = state.runtime.read().await.active_batch_id.is_some();
    if auto_start && already_running {
        return Err(AppError::conflict("device is busy running an active batch"));
    }

    let targets = validate_v1_control_params(&state.safety, &payload.params, auto_start)?;
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

    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
        if let Some(id) = batch_id {
            runtime.active_batch_id = Some(id);
        }
        if auto_start {
            runtime.auto_enabled = true;
        }
    }

    if let Err(err) = state.db.insert_control_event(
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
    ) {
        rollback_v1_auto_start_activation(&state, batch_id).await;
        return Err(err.into());
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
    ApiJson(payload): ApiJson<PipelineSampleRequest>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
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
    require_permission(&headers, Permission::ViewMonitor)?;
    let runtime = state.runtime.read().await.clone();
    ensure_fresh_sample(&state, &runtime)?;
    Ok(Json(v1_realtime_payload(
        &device_id,
        &runtime,
        state.ai_memory.as_ref(),
    )))
}

async fn v1_realtime_ws(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    require_permission(&headers, Permission::ViewMonitor)?;
    Ok(ws.on_upgrade(move |socket| v1_realtime_socket(socket, state, device_id)))
}

async fn v1_realtime_socket(mut socket: WebSocket, state: AppState, device_id: String) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let runtime = state.runtime.read().await.clone();
        let payload = v1_realtime_payload(&device_id, &runtime, state.ai_memory.as_ref());
        let Ok(text) = serde_json::to_string(&payload) else {
            break;
        };
        if socket.send(Message::Text(text)).await.is_err() {
            break;
        }
    }
}

fn v1_realtime_payload(device_id: &str, runtime: &RuntimeState, memory: &AiMemory) -> Value {
    let sample = runtime.latest_sample.as_ref();
    json!({
        "device_id": device_id,
        "timestamp": sample
            .map(|sample| sample.captured_at.to_rfc3339())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        "status": device_status(&runtime),
        "data": {
            "current_temp": sample.map(|sample| sample.temperature_c),
            "current_pressure": sample.map(|sample| sample.pressure_mpa),
            "stir_speed": sample.map(|sample| sample.stirrer_rpm),
            "shake_speed": sample.map(|sample| sample.shake_speed_cpm),
            "tilt_state": sample.map(|sample| sample.tilt_state),
            "tilt_angle": sample.map(|sample| sample.tilt_angle_deg),
            "tilt_angle_source": "software_fit_from_binary_sensor",
            "flow_rate": sample.map(|sample| sample.flow_rate_l_min),
            "phase": phase_for(&runtime),
            "progress": progress_for(sample)
        },
        "alarms": alarms_for(runtime, sample, memory)
    })
}

async fn v1_history(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Query(query): Query<V1HistoryQuery>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
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
    for phase in &payload.phases {
        if let Some(duration) = phase.params.get("duration").and_then(Value::as_f64) {
            validate_range("duration", duration, 0.0, 7200.0)?;
            total_seconds += duration;
            match phase.phase.as_str() {
                "heating" => heat_time_s = Some(duration),
                "holding" => hold_time_s = Some(duration),
                "cooling" => cool_time_s = Some(duration),
                _ => {}
            }
        }
        if let Some(temp) = phase.params.get("target_temp").and_then(Value::as_f64) {
            validate_target_temperature(&state.safety, temp)?;
            target_temperature = Some(temp);
        }
        if let Some(speed) = phase.params.get("stir_speed").and_then(Value::as_f64) {
            validate_stir_speed(&state.safety, speed)?;
            stirrer_rpm = Some(speed);
        }
        if let Some(speed) = phase.params.get("shake_speed").and_then(Value::as_f64) {
            validate_range("shake_speed", speed, 0.0, 60.0)?;
            shake_speed_cpm = Some(speed);
        }
        if let Some(pressure) = phase.params.get("target_pressure").and_then(Value::as_f64) {
            validate_range("target_pressure", pressure, 0.0, 10.0)?;
            target_pressure_mpa = Some(pressure);
        }
    }
    let targets = clamp_operator_targets(
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
    );
    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
    }
    state.db.insert_control_event(
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
    )?;

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

async fn start_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<StartBatchRequest>,
) -> Result<Json<Batch>, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    let targets = {
        let runtime = state.runtime.read().await;
        runtime.targets.clone()
    };
    let target_temperature_c = round2(
        payload
            .target_temperature_c
            .unwrap_or(targets.temperature_c),
    );
    let target_stirrer_rpm = round2(payload.target_stirrer_rpm.unwrap_or(targets.stirrer_rpm));
    let target_shake_speed_cpm = round2(
        payload
            .target_shake_speed_cpm
            .unwrap_or(targets.shake_speed_cpm),
    );
    let heating_minutes = round2(payload.heating_minutes.unwrap_or(60.0));
    let stirring_minutes = round2(payload.stirring_minutes.unwrap_or(60.0));
    let name = payload.name.unwrap_or_else(|| "batch".to_string());
    let batch = state.db.create_batch_for_process(
        payload.process_id,
        &name,
        target_temperature_c,
        target_stirrer_rpm,
        heating_minutes,
        stirring_minutes,
    )?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.active_batch_id = Some(batch.id);
        runtime.targets = clamp_operator_targets(
            &state.safety,
            ControlTargets {
                temperature_c: batch.target_temperature_c,
                heat_time_s: batch.heating_minutes * 60.0,
                hold_time_s: batch.stirring_minutes * 60.0,
                cool_time_s: targets.cool_time_s,
                stirrer_rpm: batch.target_stirrer_rpm,
                shake_speed_cpm: target_shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
            },
        );
    }
    state.db.insert_control_event(
        Some(batch.id),
        "batch_started",
        Some(&SafeCommand {
            target_temperature_c: batch.target_temperature_c,
            heat_time_s: batch.heating_minutes * 60.0,
            hold_time_s: batch.stirring_minutes * 60.0,
            cool_time_s: targets.cool_time_s,
            target_stirrer_rpm: batch.target_stirrer_rpm,
            target_shake_speed_cpm,
            target_pressure_mpa: targets.target_pressure_mpa,
            reason: "batch started and runtime targets updated".to_string(),
        }),
        "batch started and runtime targets updated",
    )?;
    Ok(Json(batch))
}

async fn finish_batch(
    State(state): State<AppState>,
    axum::extract::Path(batch_id): axum::extract::Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::StartStopProcess)?;
    state.db.finish_batch(batch_id)?;
    {
        let mut runtime = state.runtime.write().await;
        if runtime.active_batch_id == Some(batch_id) {
            runtime.active_batch_id = None;
        }
    }
    state
        .db
        .insert_control_event(Some(batch_id), "batch_finished", None, "batch finished")?;
    Ok(StatusCode::NO_CONTENT)
}

async fn product_results(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ProductResultRequest>,
) -> Result<Json<AiRecommendationEnvelope>, AppError> {
    require_permission(&headers, Permission::EditProcess)?;
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
    state.db.insert_product_result(&ProductResult {
        batch_id: payload.batch_id,
        yield_percent: round2(payload.yield_percent),
        product_ratio: round2(payload.product_ratio),
        notes: payload.notes.unwrap_or_default(),
    })?;
    state.db.insert_control_event(
        Some(payload.batch_id),
        "product_result_recorded",
        None,
        "product result saved; recommendation regeneration queued",
    )?;
    let recommendation = generate_recommendation(&state).await?;
    state.db.insert_recommendation(&recommendation)?;
    Ok(Json(recommendation_envelope(&state, recommendation).await))
}

async fn set_auto(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<AutoRequest>,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.auto_enabled = payload.enabled;
    }
    state.db.insert_control_event(
        None,
        if payload.enabled {
            "auto_enabled"
        } else {
            "auto_disabled"
        },
        None,
        "operator changed automatic control state",
    )?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_manual_lock(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ManualLockRequest>,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.manual_lock = payload.locked;
    }
    state.db.insert_control_event(
        None,
        if payload.locked {
            "manual_lock_on"
        } else {
            "manual_lock_off"
        },
        None,
        "operator changed manual lock state",
    )?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<TargetRequest>,
) -> Result<Json<ControlTargets>, AppError> {
    require_permission(&headers, Permission::SetSafeTargets)?;
    let targets = clamp_operator_targets(&state.safety, {
        let current = state.runtime.read().await.targets.clone();
        ControlTargets {
            temperature_c: payload.temperature_c,
            heat_time_s: current.heat_time_s,
            hold_time_s: current.hold_time_s,
            cool_time_s: current.cool_time_s,
            stirrer_rpm: payload.stirrer_rpm,
            shake_speed_cpm: payload.shake_speed_cpm.unwrap_or(current.shake_speed_cpm),
            target_pressure_mpa: current.target_pressure_mpa,
        }
    });
    ensure_targets_allowed(&state.safety, &targets)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
    }
    state.db.insert_control_event(
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
            reason: "operator target request after safety clamp".to_string(),
        }),
        "operator changed desired targets; values clamped to configured safety limits",
    )?;
    Ok(Json(targets))
}

async fn emergency_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::EmergencyStop)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.emergency_stop = true;
        runtime.auto_enabled = false;
    }
    state.db.insert_control_event(
        None,
        "emergency_stop",
        None,
        "operator triggered emergency stop; automatic control disabled",
    )?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_emergency_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    require_permission(&headers, Permission::EmergencyStop)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.emergency_stop = false;
    }
    state.db.insert_control_event(
        None,
        "emergency_stop_reset",
        None,
        "operator reset emergency stop flag",
    )?;
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
    let recommendation = generate_recommendation(&state).await?;
    state.db.insert_recommendation(&recommendation)?;
    Ok(Json(Some(
        recommendation_envelope(&state, recommendation).await,
    )))
}

async fn api_not_found() -> AppError {
    AppError::not_found("api route not found")
}

async fn test_reset(State(state): State<AppState>) -> Result<StatusCode, AppError> {
    if !state.test_reset_enabled {
        return Err(AppError::not_found("not found"));
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
    ApiJson(payload): ApiJson<PipelineSampleRequest>,
) -> Result<Json<SensorSnapshot>, AppError> {
    if !state.test_reset_enabled {
        return Err(AppError::not_found("not found"));
    }

    let sample = accept_pipeline_sample(&state, payload).await?;
    Ok(Json(sample))
}

async fn accept_pipeline_sample(
    state: &AppState,
    payload: PipelineSampleRequest,
) -> Result<SensorSnapshot, AppError> {
    let sample = pipeline_sample_from_request(payload)?;
    let active_batch_id = {
        let mut runtime = state.runtime.write().await;
        runtime.latest_sample = Some(sample.clone());
        runtime.last_sensor_error = None;
        runtime.last_control_error = None;
        runtime.active_batch_id
    };
    state
        .db
        .insert_sample_sqlx(active_batch_id, &sample)
        .await?;
    Ok(sample)
}

fn pipeline_sample_from_request(
    payload: PipelineSampleRequest,
) -> Result<SensorSnapshot, AppError> {
    let captured_at = Utc::now();
    let tilt_state = validate_tilt_state(payload.tilt_state)?;
    let shake_speed_cpm = round2_finite("shake_speed_cpm", payload.shake_speed_cpm)?;
    Ok(SensorSnapshot {
        temperature_c: round2_finite("temperature_c", payload.temperature_c)?,
        pressure_mpa: round2_finite("pressure_mpa", payload.pressure_mpa)?,
        stirrer_rpm: round2_finite("stirrer_rpm", payload.stirrer_rpm)?,
        shake_speed_cpm,
        tilt_state,
        tilt_angle_deg: fit_tilt_angle_deg(tilt_state, shake_speed_cpm, captured_at),
        flow_rate_l_min: round2_finite("flow_rate_l_min", payload.flow_rate_l_min)?,
        product_concentration_percent: round2_finite(
            "product_concentration_percent",
            payload.product_concentration_percent,
        )?,
        ph: round2_finite("ph", payload.ph)?,
        captured_at,
    })
}

fn ensure_fresh_sample(state: &AppState, runtime: &RuntimeState) -> Result<(), AppError> {
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
    let max_age = Duration::milliseconds(state.safety.control.sensor_timeout_ms);
    let age = Utc::now().signed_duration_since(sample.captured_at);
    if age > max_age {
        return Err(AppError::service_unavailable(format!(
            "sensor data stale; last data pipeline sample is {} ms old",
            age.num_milliseconds()
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
    let trimmed = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    trimmed.chars().take(max_chars).collect()
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
    let ramp_rate_c_min = payload.ramp_rate_c_min.unwrap_or(0.0);
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
    let target_shake_speed_cpm = payload.target_shake_speed_cpm.unwrap_or(30.0);
    validate_range("target_shake_speed_cpm", target_shake_speed_cpm, 0.0, 60.0)?;
    let target_pressure_mpa = payload.target_pressure_mpa.unwrap_or(0.5);
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
    let first = steps
        .first()
        .ok_or_else(|| AppError::bad_request("process must contain at least one step"))?;
    let hold = steps.get(1).unwrap_or(first);
    let last = steps.last().unwrap_or(first);
    let heat_time_s =
        (first.duration_minutes * 60.0).min(safety.optimizer.max_heating_minutes * 60.0);
    let hold_time_s =
        (hold.duration_minutes * 60.0).min(safety.optimizer.max_stirring_minutes * 60.0);
    let cool_time_s = if steps.len() > 2 {
        (last.duration_minutes * 60.0).min(3600.0)
    } else {
        180.0
    };
    let targets = clamp_operator_targets(
        safety,
        ControlTargets {
            temperature_c: first.target_temperature_c,
            heat_time_s,
            hold_time_s,
            cool_time_s,
            stirrer_rpm: hold.target_stirrer_rpm,
            shake_speed_cpm: hold.target_shake_speed_cpm,
            target_pressure_mpa: hold.target_pressure_mpa,
        },
    );
    ensure_targets_allowed(safety, &targets)?;
    Ok(targets)
}

fn seconds_to_minutes(seconds: Option<f64>) -> f64 {
    round2(seconds.unwrap_or(3600.0) / 60.0)
}

fn validate_v1_control_params(
    safety: &SafetyConfig,
    params: &V1ControlParams,
    running_state: bool,
) -> Result<ControlTargets, AppError> {
    let heat_time_s = params.heat_time.unwrap_or(300.0);
    let hold_time_s = params.hold_time.unwrap_or(600.0);
    let cool_time_s = params.cool_time.unwrap_or(180.0);
    let stirrer_rpm = params.stir_speed.unwrap_or(800.0);
    let shake_speed_cpm = params.shake_speed.unwrap_or(30.0);
    let target_temperature_c = params.target_temp.unwrap_or(120.0);
    let target_pressure_mpa = params.target_pressure.unwrap_or(0.5);

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

fn round2_finite(field: &str, value: f64) -> Result<f64, AppError> {
    if !value.is_finite() {
        return Err(AppError::bad_request(format!("{field} must be finite")));
    }
    Ok(round2(value))
}

fn validate_tilt_state(value: u8) -> Result<u8, AppError> {
    if value <= 1 {
        Ok(value)
    } else {
        Err(AppError::bad_request(
            "tilt_state must be 0 or 1 for the shake vessel binary tilt sensor",
        ))
    }
}

fn validate_target_temperature(safety: &SafetyConfig, value: f64) -> Result<(), AppError> {
    validate_range("target_temp", value, 0.0, 500.0)?;
    if value > safety.temperature.max_c {
        return Err(AppError::bad_request(format!(
            "target_temp exceeds device maximum temperature {:.1}",
            safety.temperature.max_c
        )));
    }
    Ok(())
}

fn validate_stir_speed(safety: &SafetyConfig, value: f64) -> Result<(), AppError> {
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

fn validate_range(field: &str, value: f64, min: f64, max: f64) -> Result<(), AppError> {
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

fn device_status_item(
    device_id: &str,
    device_role: &str,
    state: &AppState,
    runtime: &RuntimeState,
) -> DeviceStatusItem {
    let now = Utc::now();
    let stale_after_ms = state.safety.control.sensor_timeout_ms;
    let bridge_status = runtime.device_status.as_ref();
    let (last_seen_at, last_seen_age_ms, sample_fresh) = match &runtime.latest_sample {
        Some(sample) => {
            let age = now
                .signed_duration_since(sample.captured_at)
                .num_milliseconds();
            (
                Some(sample.captured_at.to_rfc3339()),
                Some(age),
                age <= stale_after_ms,
            )
        }
        None => (None, None, false),
    };
    let (last_seen_at, last_seen_age_ms) =
        if let Some(last_seen) = bridge_status.and_then(|status| status.last_seen_at.as_ref()) {
            (
                Some(last_seen.to_rfc3339()),
                Some(now.signed_duration_since(last_seen).num_milliseconds()),
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
                    .map(|last_seen| {
                        now.signed_duration_since(last_seen).num_milliseconds() <= stale_after_ms
                    })
                    .unwrap_or(false)
        })
        .unwrap_or(sample_fresh);
    let status = if runtime.emergency_stop {
        "error"
    } else if bridge_status
        .map(|status| !status.connected || !status.last_frame_ok)
        .unwrap_or(false)
    {
        "error"
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
        online: bridge_online && !runtime.emergency_stop,
        status: status.to_string(),
        last_seen_at,
        last_seen_age_ms,
        stale_after_ms,
        active_batch_id: runtime.active_batch_id,
        emergency_stop: runtime.emergency_stop,
        last_sensor_error: runtime.last_sensor_error.clone(),
        last_control_error: runtime.last_control_error.clone(),
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
            } else if status
                .map(|status| !status.connected || !status.last_frame_ok)
                .unwrap_or(false)
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

fn device_status(runtime: &RuntimeState) -> &'static str {
    if runtime.emergency_stop
        || runtime.last_sensor_error.is_some()
        || runtime.last_control_error.is_some()
    {
        "error"
    } else if runtime.active_batch_id.is_some() {
        "running"
    } else {
        "idle"
    }
}

fn phase_for(runtime: &RuntimeState) -> &'static str {
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
    runtime: &RuntimeState,
    sample: Option<&SensorSnapshot>,
    memory: &AiMemory,
) -> Vec<Value> {
    let mut alarms = Vec::new();
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
