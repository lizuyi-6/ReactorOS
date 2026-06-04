use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{
        rejection::JsonRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
        FromRequest, Path, Query, Request, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{any, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    ai_provider::{
        fallback_envelope, local_envelope, stepfun_envelope, AiProvider, AiRecommendationEnvelope,
        AiRecommendationProvider,
    },
    config::{DeviceConfig, DeviceMode, RegistersConfig, SafetyConfig, WriteRegister},
    control::{clamp_operator_targets, forbidden_control_zone, SafeCommand},
    db::{
        Batch, BatchOutcome, ControlEvent, Db, DemoAlarm, IntegrationTask, NewProcessStep,
        ProcessDefinition, ProcessDetail, ProcessStep, ProductResult, SensorSampleRecord,
    },
    device::{ComponentControlCommand, ComponentControlOutcome, SharedDevice},
    local_ai::LocalAiStatus,
    memory::{AiMemory, AiMemorySummary, LimitLevel, SensorLimit},
    optimizer::{recommend_with_memory, Recommendation},
    state::{fit_tilt_angle_deg, ControlTargets, RuntimeState, SensorSnapshot, SharedState},
};

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
pub struct V1Envelope<T> {
    pub code: i32,
    pub message: String,
    pub data: T,
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
pub struct AinasTaskQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AinasTaskRequest {
    pub external_task_id: Option<String>,
    pub action: String,
    pub process_id: Option<i64>,
    pub target_temperature_c: Option<f64>,
    pub target_stirrer_rpm: Option<f64>,
    pub target_shake_speed_cpm: Option<f64>,
    pub target_pressure_mpa: Option<f64>,
    pub heat_time_s: Option<f64>,
    pub hold_time_s: Option<f64>,
    pub cool_time_s: Option<f64>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: AuthUser,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthUser {
    pub username: String,
    pub role: String,
    pub permissions: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthRole {
    Operator,
    Engineer,
    Admin,
}

#[derive(Debug, Clone, Copy)]
enum Permission {
    ViewMonitor,
    ViewHistory,
    ViewAudit,
    ExportReports,
    EditProcess,
    StartStopProcess,
    SetSafeTargets,
    ApplyAiSuggestion,
    EmergencyStop,
    ModbusDebug,
    EditSystemConfig,
    DeleteData,
    ManageUsers,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct AiControlAction {
    pub action_type: String,
    pub target: String,
    pub status: String,
    pub message: String,
    pub result: Option<Value>,
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
        .route("/api/recommendations/latest", get(latest_recommendation))
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
    let recent_samples = state.db.recent_sample_records(sample_limit)?;
    let processes = if query.include_processes.unwrap_or(true) {
        state.db.list_processes()?
    } else {
        Vec::new()
    };
    let (recent_batches, recent_outcomes) = if query.include_batches.unwrap_or(true) {
        (
            state.db.recent_batches(20)?,
            state.db.recent_batch_outcomes(20)?,
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
        .latest_recommendation()?
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
            .latest_recommendation()?
            .filter(|recommendation| provider_allows_recommendation(&state, recommendation))
            .filter(|recommendation| recommendation.based_on_batch_count > 0),
        ai_provider: local_provider_for(&state),
        processes: state.db.list_processes()?,
        recent_batches: state.db.recent_batches(20)?,
        recent_outcomes: state.db.recent_batch_outcomes(20)?,
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
    let total = state.db.audit_event_count(event_type)?;
    let events = state
        .db
        .audit_events(page_size, (page - 1) * page_size, event_type)?;
    let chain = state.db.audit_chain_status()?;
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
    let events = state.db.audit_events(10_000, 0, event_type)?;
    let mut csv = String::from(
        "id,batch_id,event_type,target_temperature_c,target_stirrer_rpm,target_shake_speed_cpm,reason,created_at,previous_hash,event_hash\n",
    );
    for event in events {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            event.id,
            event
                .batch_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv_escape(&event.event_type),
            event
                .target_temperature_c
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .target_stirrer_rpm
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .target_shake_speed_cpm
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv_escape(&event.reason),
            event.created_at.to_rfc3339(),
            event.previous_hash.unwrap_or_default(),
            event.event_hash.unwrap_or_default()
        ));
    }
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
    let username = payload.username.trim().to_ascii_lowercase();
    let Some(role) = role_for_login(&username, &payload.password) else {
        return Err(AppError::unauthorized("invalid username or password"));
    };
    let expires_at = Utc::now() + Duration::hours(12);
    let token = issue_auth_token(&username, role, expires_at);
    Ok(Json(success(LoginResponse {
        token,
        user: auth_user(&username, role),
        expires_at: expires_at.to_rfc3339(),
    })))
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

async fn list_ainas_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AinasTaskQuery>,
) -> Result<Json<V1Envelope<Vec<IntegrationTask>>>, AppError> {
    require_permission(&headers, Permission::ViewAudit)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    Ok(Json(success(
        state.db.integration_tasks(Some("ainas"), limit)?,
    )))
}

async fn get_ainas_task(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<V1Envelope<IntegrationTask>>, AppError> {
    require_permission(&headers, Permission::ViewAudit)?;
    let Some(task) = state.db.integration_task(id)? else {
        return Err(AppError::not_found("AINAS task not found"));
    };
    if task.source != "ainas" {
        return Err(AppError::not_found("AINAS task not found"));
    }
    Ok(Json(success(task)))
}

async fn create_ainas_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<AinasTaskRequest>,
) -> Result<Json<V1Envelope<IntegrationTask>>, AppError> {
    let action = normalize_ainas_action(&payload.action)?;
    require_ainas_action_permission(&headers, action)?;
    Ok(Json(success(
        execute_integration_task(&state, "ainas", payload).await?,
    )))
}

pub async fn execute_integration_task(
    state: &AppState,
    source: &str,
    payload: AinasTaskRequest,
) -> Result<IntegrationTask, AppError> {
    let request = serde_json::to_value(&payload).map_err(|err| {
        AppError::from(anyhow::anyhow!(
            "failed to serialize integration task request: {err}"
        ))
    })?;
    let action = normalize_ainas_action(&payload.action)?;
    let external_task_id = clean_optional_text(payload.external_task_id.as_deref(), 120);
    let source = clean_optional_text(Some(source), 40).unwrap_or_else(|| "integration".to_string());
    let task =
        state
            .db
            .create_integration_task(&source, external_task_id.as_deref(), action, &request)?;

    match execute_ainas_task(state, action, &payload).await {
        Ok(response) => {
            let Some(task) = state
                .db
                .update_integration_task(task.id, "executed", &response)?
            else {
                return Err(AppError::not_found("AINAS task not found after execution"));
            };
            Ok(task)
        }
        Err(err) => {
            let status = if err.status.is_server_error() {
                "failed"
            } else {
                "rejected"
            };
            let message = err.message.clone();
            let response = json!({
                "code": err.status.as_u16(),
                "message": message.clone(),
                "data": { "error": message }
            });
            state
                .db
                .update_integration_task(task.id, status, &response)?;
            Err(err)
        }
    }
}

fn normalize_ainas_action(action: &str) -> Result<&'static str, AppError> {
    match action.trim().to_ascii_lowercase().as_str() {
        "set_targets" | "set-targets" | "control.set_targets" | "control:set_targets" => {
            Ok("set_targets")
        }
        "start_process" | "start-process" | "process.start" | "process:start" | "start" => {
            Ok("start_process")
        }
        "stop_process" | "stop-process" | "process.stop" | "process:stop" | "stop" => {
            Ok("stop_process")
        }
        _ => Err(AppError::bad_request(
            "AINAS action must be set_targets, start_process, or stop_process",
        )),
    }
}

fn require_ainas_action_permission(headers: &HeaderMap, action: &str) -> Result<(), AppError> {
    let permission = match action {
        "set_targets" => Permission::SetSafeTargets,
        "start_process" | "stop_process" => Permission::StartStopProcess,
        _ => {
            return Err(AppError::bad_request(
                "AINAS action must be set_targets, start_process, or stop_process",
            ))
        }
    };
    require_permission(headers, permission)?;
    Ok(())
}

async fn execute_ainas_task(
    state: &AppState,
    action: &str,
    payload: &AinasTaskRequest,
) -> Result<Value, AppError> {
    match action {
        "set_targets" => {
            let targets = apply_ainas_targets(state, payload).await?;
            Ok(json!({
                "action": "set_targets",
                "status": "executed",
                "safety": "clamped_to_configured_limits",
                "targets": targets
            }))
        }
        "start_process" => {
            let process_id = payload
                .process_id
                .ok_or_else(|| AppError::bad_request("process_id is required for start_process"))?;
            let response =
                start_process_lifecycle(state, process_id, "ainas_process_started").await?;
            json_response(response)
        }
        "stop_process" => {
            let response =
                stop_process_lifecycle(state, payload.process_id, "ainas_process_stopped").await?;
            json_response(response)
        }
        _ => Err(AppError::bad_request(
            "AINAS action must be set_targets, start_process, or stop_process",
        )),
    }
}

async fn apply_ainas_targets(
    state: &AppState,
    payload: &AinasTaskRequest,
) -> Result<ControlTargets, AppError> {
    if payload.target_temperature_c.is_none()
        && payload.target_stirrer_rpm.is_none()
        && payload.target_shake_speed_cpm.is_none()
        && payload.target_pressure_mpa.is_none()
        && payload.heat_time_s.is_none()
        && payload.hold_time_s.is_none()
        && payload.cool_time_s.is_none()
    {
        return Err(AppError::bad_request(
            "set_targets requires at least one target field",
        ));
    }

    let current = state.runtime.read().await.targets.clone();
    let targets = clamp_operator_targets(
        &state.safety,
        ControlTargets {
            temperature_c: payload
                .target_temperature_c
                .unwrap_or(current.temperature_c),
            heat_time_s: payload.heat_time_s.unwrap_or(current.heat_time_s),
            hold_time_s: payload.hold_time_s.unwrap_or(current.hold_time_s),
            cool_time_s: payload.cool_time_s.unwrap_or(current.cool_time_s),
            stirrer_rpm: payload.target_stirrer_rpm.unwrap_or(current.stirrer_rpm),
            shake_speed_cpm: payload
                .target_shake_speed_cpm
                .unwrap_or(current.shake_speed_cpm),
            target_pressure_mpa: payload
                .target_pressure_mpa
                .unwrap_or(current.target_pressure_mpa),
        },
    );
    ensure_targets_allowed(&state.safety, &targets)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
    }
    let reason = clean_label(
        payload.reason.clone(),
        "AINAS task changed desired targets",
        240,
    );
    state.db.insert_control_event(
        None,
        "ainas_targets_updated",
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
    )?;
    Ok(targets)
}

fn json_response<T: Serialize>(value: T) -> Result<Value, AppError> {
    serde_json::to_value(value)
        .map_err(|err| AppError::from(anyhow::anyhow!("failed to serialize response: {err}")))
}

fn permission_policy() -> Value {
    json!({
        "mode": "local_role_policy",
        "authentication": "bearer_session_enforced",
        "session_ttl_hours": 12,
        "note": "Local username/password login issues signed bearer sessions; write and export operations are checked against role permissions.",
        "default_users": [
            { "username": "operator", "role": "operator" },
            { "username": "engineer", "role": "engineer" },
            { "username": "admin", "role": "admin" }
        ],
        "roles": [
            {
                "role": "operator",
                "label": "Operator",
                "can": permission_names_for_role(AuthRole::Operator),
                "blocked": blocked_permission_names_for_role(AuthRole::Operator)
            },
            {
                "role": "engineer",
                "label": "Engineer",
                "can": permission_names_for_role(AuthRole::Engineer),
                "blocked": blocked_permission_names_for_role(AuthRole::Engineer)
            },
            {
                "role": "admin",
                "label": "Admin",
                "can": permission_names_for_role(AuthRole::Admin),
                "blocked": blocked_permission_names_for_role(AuthRole::Admin)
            }
        ]
    })
}

fn permission_names_for_role(role: AuthRole) -> Vec<&'static str> {
    [
        Permission::ViewMonitor,
        Permission::ViewHistory,
        Permission::ViewAudit,
        Permission::ExportReports,
        Permission::EditProcess,
        Permission::StartStopProcess,
        Permission::SetSafeTargets,
        Permission::ApplyAiSuggestion,
        Permission::EmergencyStop,
        Permission::ModbusDebug,
        Permission::EditSystemConfig,
        Permission::DeleteData,
        Permission::ManageUsers,
    ]
    .into_iter()
    .filter(|permission| role_allows(role, *permission))
    .map(permission_name)
    .collect()
}

fn blocked_permission_names_for_role(role: AuthRole) -> Vec<&'static str> {
    [
        Permission::ViewMonitor,
        Permission::ViewHistory,
        Permission::ViewAudit,
        Permission::ExportReports,
        Permission::EditProcess,
        Permission::StartStopProcess,
        Permission::SetSafeTargets,
        Permission::ApplyAiSuggestion,
        Permission::EmergencyStop,
        Permission::ModbusDebug,
        Permission::EditSystemConfig,
        Permission::DeleteData,
        Permission::ManageUsers,
    ]
    .into_iter()
    .filter(|permission| !role_allows(role, *permission))
    .map(permission_name)
    .collect()
}

fn permission_name(permission: Permission) -> &'static str {
    match permission {
        Permission::ViewMonitor => "view_monitor",
        Permission::ViewHistory => "view_history",
        Permission::ViewAudit => "view_audit",
        Permission::ExportReports => "export_reports",
        Permission::EditProcess => "edit_process",
        Permission::StartStopProcess => "start_stop_process",
        Permission::SetSafeTargets => "set_safe_targets",
        Permission::ApplyAiSuggestion => "apply_ai_suggestion",
        Permission::EmergencyStop => "emergency_stop",
        Permission::ModbusDebug => "modbus_debug",
        Permission::EditSystemConfig => "edit_system_config",
        Permission::DeleteData => "delete_data",
        Permission::ManageUsers => "manage_users",
    }
}

fn role_allows(role: AuthRole, permission: Permission) -> bool {
    match role {
        AuthRole::Operator => matches!(
            permission,
            Permission::ViewMonitor
                | Permission::ViewHistory
                | Permission::ExportReports
                | Permission::StartStopProcess
                | Permission::SetSafeTargets
                | Permission::ApplyAiSuggestion
                | Permission::EmergencyStop
        ),
        AuthRole::Engineer => matches!(
            permission,
            Permission::ViewMonitor
                | Permission::ViewHistory
                | Permission::ViewAudit
                | Permission::ExportReports
                | Permission::EditProcess
                | Permission::StartStopProcess
                | Permission::SetSafeTargets
                | Permission::ApplyAiSuggestion
                | Permission::EmergencyStop
                | Permission::ModbusDebug
        ),
        AuthRole::Admin => true,
    }
}

fn auth_user(username: &str, role: AuthRole) -> AuthUser {
    AuthUser {
        username: username.to_string(),
        role: role_name(role).to_string(),
        permissions: permission_names_for_role(role),
    }
}

fn role_for_login(username: &str, password: &str) -> Option<AuthRole> {
    let candidates = [
        (
            "operator",
            env::var("XINGSHU_OPERATOR_PASSWORD").unwrap_or_else(|_| "operator123".to_string()),
            AuthRole::Operator,
        ),
        (
            "engineer",
            env::var("XINGSHU_ENGINEER_PASSWORD").unwrap_or_else(|_| "engineer123".to_string()),
            AuthRole::Engineer,
        ),
        (
            "admin",
            env::var("XINGSHU_ADMIN_PASSWORD").unwrap_or_else(|_| "admin123".to_string()),
            AuthRole::Admin,
        ),
    ];
    candidates
        .into_iter()
        .find(|(candidate, expected, _)| username == *candidate && password == expected)
        .map(|(_, _, role)| role)
}

fn role_name(role: AuthRole) -> &'static str {
    match role {
        AuthRole::Operator => "operator",
        AuthRole::Engineer => "engineer",
        AuthRole::Admin => "admin",
    }
}

fn role_from_name(role: &str) -> Option<AuthRole> {
    match role {
        "operator" => Some(AuthRole::Operator),
        "engineer" => Some(AuthRole::Engineer),
        "admin" => Some(AuthRole::Admin),
        _ => None,
    }
}

fn auth_secret() -> String {
    env::var("XINGSHU_AUTH_SECRET")
        .unwrap_or_else(|_| "xingshu-local-rbac-session-secret".to_string())
}

fn issue_auth_token(username: &str, role: AuthRole, expires_at: DateTime<Utc>) -> String {
    let payload = format!(
        "{}:{}:{}",
        username,
        role_name(role),
        expires_at.timestamp()
    );
    let signature = auth_signature(&payload);
    format!("{payload}:{signature}")
}

fn auth_signature(payload: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(auth_secret().as_bytes());
    hasher.update(b":");
    hasher.update(payload.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn authenticated_user(headers: &HeaderMap) -> Result<AuthUser, AppError> {
    let header = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::unauthorized("missing bearer session token"))?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::unauthorized("authorization must use Bearer token"))?;
    let mut parts = token.split(':');
    let username = parts.next().unwrap_or_default();
    let role_text = parts.next().unwrap_or_default();
    let expires_text = parts.next().unwrap_or_default();
    let signature = parts.next().unwrap_or_default();
    if username.is_empty()
        || role_text.is_empty()
        || expires_text.is_empty()
        || signature.is_empty()
        || parts.next().is_some()
    {
        return Err(AppError::unauthorized("invalid bearer session token"));
    }
    let expires_at = expires_text
        .parse::<i64>()
        .map_err(|_| AppError::unauthorized("invalid bearer session expiry"))?;
    if Utc::now().timestamp() > expires_at {
        return Err(AppError::unauthorized("bearer session has expired"));
    }
    let payload = format!("{username}:{role_text}:{expires_text}");
    if auth_signature(&payload) != signature {
        return Err(AppError::unauthorized("invalid bearer session signature"));
    }
    let role = role_from_name(role_text)
        .ok_or_else(|| AppError::unauthorized("invalid bearer session role"))?;
    Ok(auth_user(username, role))
}

fn require_permission(headers: &HeaderMap, permission: Permission) -> Result<AuthUser, AppError> {
    let user = authenticated_user(headers)?;
    let role = role_from_name(&user.role)
        .ok_or_else(|| AppError::unauthorized("invalid bearer session role"))?;
    if !role_allows(role, permission) {
        return Err(AppError::forbidden(format!(
            "role '{}' lacks permission '{}'",
            user.role,
            permission_name(permission)
        )));
    }
    Ok(user)
}

async fn modbus_registers(State(state): State<AppState>) -> Json<V1Envelope<Value>> {
    let runtime = state.runtime.read().await.clone();
    let tcp_status = crate::modbus_tcp::modbus_tcp_status_snapshot().await;
    Json(success(json!({
        "device_id": "reactor_001",
        "mode": state.device_mode,
        "slave_id": state.device_config.modbus.slave_id,
        "serial": state.device_config.serial,
        "tcp": tcp_status,
        "read_registers": [
            modbus_read_register_json("temperature_c", "current temperature", &state, &runtime),
            modbus_read_register_json("stirrer_rpm", "current stirrer speed", &state, &runtime),
            modbus_read_register_json("pressure_mpa", "current pressure", &state, &runtime),
            modbus_read_register_json("shake_speed_cpm", "current shake speed", &state, &runtime),
            modbus_read_register_json("tilt_angle_deg", "current tilt angle", &state, &runtime),
            modbus_read_register_json("flow_rate_l_min", "current flow rate", &state, &runtime),
            modbus_read_register_json("product_concentration_percent", "current product concentration", &state, &runtime),
            modbus_read_register_json("ph", "current pH", &state, &runtime)
        ],
        "write_registers": [
            modbus_write_register_json("target_temperature_c", "target temperature", &state, &runtime),
            modbus_write_register_json("target_stirrer_rpm", "target stirrer speed", &state, &runtime),
            modbus_write_register_json("target_shake_speed_cpm", "target shake speed", &state, &runtime),
            modbus_write_register_json("target_pressure_mpa", "target pressure", &state, &runtime),
            modbus_write_register_json("heat_time_s", "heat time", &state, &runtime),
            modbus_write_register_json("hold_time_s", "hold time", &state, &runtime),
            modbus_write_register_json("cool_time_s", "cool time", &state, &runtime)
        ],
        "coils": modbus_coils_json(&state, &runtime),
        "discrete_inputs": modbus_discrete_inputs_json(&state, &runtime)
    })))
}

async fn modbus_register_read(
    State(state): State<AppState>,
    Path(register): Path<String>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    let runtime = state.runtime.read().await.clone();
    let value = modbus_register_value(&state, &runtime, &register)?;
    let raw = encode_modbus_raw(value.value, value.scale, value.offset)?;
    Ok(Json(success(json!({
        "device_id": "reactor_001",
        "register": register,
        "address": value.address,
        "access": value.access,
        "value": round2(value.value),
        "raw": raw,
        "scale": value.scale,
        "offset": value.offset,
        "source": value.source
    }))))
}

async fn modbus_register_write(
    State(state): State<AppState>,
    Path(register): Path<String>,
    headers: HeaderMap,
    ApiJson(payload): ApiJson<ModbusWriteRequest>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
    require_permission(&headers, Permission::ModbusDebug)?;
    let response =
        apply_modbus_register_write(&state, &register, payload.value, payload.reason).await?;
    Ok(Json(success(response)))
}

pub async fn apply_modbus_register_write(
    state: &AppState,
    register: &str,
    value: f64,
    reason: Option<String>,
) -> Result<Value, AppError> {
    if !value.is_finite() {
        return Err(AppError::bad_request("value must be finite"));
    }
    let current = state.runtime.read().await.targets.clone();
    let requested = match register {
        "target_temperature_c" => ControlTargets {
            temperature_c: value,
            ..current
        },
        "target_stirrer_rpm" => ControlTargets {
            stirrer_rpm: value,
            ..current
        },
        "target_shake_speed_cpm" => ControlTargets {
            shake_speed_cpm: value,
            ..current
        },
        "target_pressure_mpa" => ControlTargets {
            target_pressure_mpa: value,
            ..current
        },
        "heat_time_s" => ControlTargets {
            heat_time_s: value,
            ..current
        },
        "hold_time_s" => ControlTargets {
            hold_time_s: value,
            ..current
        },
        "cool_time_s" => ControlTargets {
            cool_time_s: value,
            ..current
        },
        _ => {
            return Err(AppError::bad_request(
                "register is not writable through the Modbus debug API",
            ))
        }
    };
    let targets = clamp_operator_targets(&state.safety, requested);
    ensure_targets_allowed(&state.safety, &targets)?;
    let Some(register_config) =
        modbus_write_register_config(&state.device_config.modbus.registers, register)
    else {
        return Err(AppError::bad_request(
            "register is not writable through the Modbus debug API",
        ));
    };
    let applied_value = modbus_write_register_applied_value(&targets, register)?;
    let address = register_config.address;
    let scale = register_config.scale;
    let offset = register_config.offset;
    let raw = encode_modbus_raw(applied_value, scale, offset)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
    }
    let reason = reason.unwrap_or_else(|| format!("operator wrote modbus register {register}"));
    state.db.insert_control_event(
        None,
        "modbus_register_write",
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
    )?;
    Ok(json!({
        "register": register,
        "address": address,
        "requested_value": value,
        "applied_value": round2(applied_value),
        "raw": raw,
        "scale": scale,
        "offset": offset,
        "targets": targets
    }))
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
        let error_message = err.message.clone();
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
    state.db.insert_control_event(
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
    )?;
    let Some(process) = state.db.mark_process_applied(process_id)? else {
        return Err(AppError::not_found("process not found"));
    };
    Ok(ProcessApplyResponse {
        process,
        batch,
        applied_targets: targets,
        status: "running".to_string(),
    })
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
        batches: state.db.recent_batches(100)?,
        outcomes: state.db.recent_batch_outcomes(100)?,
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
        samples: state.db.sample_records_for_batch(batch_id, 480)?,
        events: state.db.control_events_for_batch(batch_id, 100)?,
        batch,
    })))
}

async fn batches_export_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    require_permission(&headers, Permission::ExportReports)?;
    let batches = state.db.recent_batches(10_000)?;
    let outcomes = state.db.recent_batch_outcomes(10_000)?;
    let mut csv = String::from(
        "id,process_id,name,started_at,finished_at,target_temperature_c,target_stirrer_rpm,heating_minutes,stirring_minutes,yield_percent,product_ratio\n",
    );
    for batch in batches {
        let outcome = outcomes.iter().find(|outcome| outcome.batch_id == batch.id);
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\n",
            batch.id,
            batch
                .process_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv_escape(&batch.name),
            batch.started_at.to_rfc3339(),
            batch
                .finished_at
                .map(|value| value.to_rfc3339())
                .unwrap_or_default(),
            batch.target_temperature_c,
            batch.target_stirrer_rpm,
            batch.heating_minutes,
            batch.stirring_minutes,
            outcome
                .map(|value| value.yield_percent.to_string())
                .unwrap_or_default(),
            outcome
                .map(|value| value.product_ratio.to_string())
                .unwrap_or_default(),
        ));
    }
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
    let batches = state.db.recent_batches(10_000)?;
    let outcomes = state.db.recent_batch_outcomes(10_000)?;
    let workbook = build_batches_xlsx(&batches, &outcomes);
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
    let samples = state.db.sample_records_for_batch(batch_id, 10_000)?;
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
        .latest_recommendation()?
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
        state.db.insert_control_event(
            state.runtime.read().await.active_batch_id,
            "ai_master_decision",
            audit_command.as_ref(),
            &format!(
                "{decision}; {}",
                actions
                    .iter()
                    .map(|action| format!("{}={}", action.action_type, action.status))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
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
    let recent_outcomes = state.db.recent_batch_outcomes(5)?;
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
        .map(|sample| sample.product_concentration_percent >= 95.0)
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

    state.db.insert_control_event(
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
    )?;

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
) -> Result<Json<Value>, AppError> {
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
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| v1_realtime_socket(socket, state, device_id))
}

async fn v1_realtime_socket(mut socket: WebSocket, state: AppState, device_id: String) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
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
        .samples_between(start_time, end_time, page_size, offset)?;
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
    let recommendation = generate_recommendation(&state).await?;
    state.db.insert_recommendation(&recommendation)?;
    Ok(Json(Some(
        recommendation_envelope(&state, recommendation).await,
    )))
}

async fn api_not_found() -> AppError {
    AppError {
        status: StatusCode::NOT_FOUND,
        message: "api route not found".to_string(),
    }
}

async fn test_reset(State(state): State<AppState>) -> Result<StatusCode, AppError> {
    if !state.test_reset_enabled {
        return Err(AppError {
            status: StatusCode::NOT_FOUND,
            message: "not found".to_string(),
        });
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
        return Err(AppError {
            status: StatusCode::NOT_FOUND,
            message: "not found".to_string(),
        });
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
    state.db.insert_sample(active_batch_id, &sample)?;
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
    let outcomes = state.db.batch_outcomes()?;
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
        fallback_envelope(
            recommendation,
            provider.model_name(),
            "cached recommendation was not generated by StepFun; regenerate recommendation before AI master control",
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

fn success<T>(data: T) -> V1Envelope<T> {
    V1Envelope {
        code: 0,
        message: "success".to_string(),
        data,
    }
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn build_batches_xlsx(batches: &[Batch], outcomes: &[BatchOutcome]) -> Vec<u8> {
    let mut zip = SimpleZip::new();
    zip.add(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#
            .to_vec(),
    );
    zip.add(
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#
            .to_vec(),
    );
    zip.add(
        "xl/workbook.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Batches" sheetId="1" r:id="rId1"/>
    <sheet name="Results" sheetId="2" r:id="rId2"/>
    <sheet name="Summary" sheetId="3" r:id="rId3"/>
  </sheets>
</workbook>"#
            .to_vec(),
    );
    zip.add(
        "xl/_rels/workbook.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet3.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#
            .to_vec(),
    );
    zip.add(
        "xl/styles.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
  <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
  <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"#
            .to_vec(),
    );

    zip.add(
        "xl/worksheets/sheet1.xml",
        worksheet_xml(&batch_rows(batches, outcomes)).into_bytes(),
    );
    zip.add(
        "xl/worksheets/sheet2.xml",
        worksheet_xml(&result_rows(outcomes)).into_bytes(),
    );
    zip.add(
        "xl/worksheets/sheet3.xml",
        worksheet_xml(&summary_rows(batches, outcomes)).into_bytes(),
    );
    zip.finish()
}

fn batch_rows(batches: &[Batch], outcomes: &[BatchOutcome]) -> Vec<Vec<XlsxCell>> {
    let mut rows = vec![xlsx_row([
        "id",
        "process_id",
        "name",
        "started_at",
        "finished_at",
        "target_temperature_c",
        "target_stirrer_rpm",
        "heating_minutes",
        "stirring_minutes",
        "yield_percent",
        "product_ratio",
    ])];
    for batch in batches {
        let outcome = outcomes.iter().find(|outcome| outcome.batch_id == batch.id);
        rows.push(vec![
            XlsxCell::Number(batch.id as f64),
            optional_number(batch.process_id.map(|value| value as f64)),
            XlsxCell::Text(batch.name.clone()),
            XlsxCell::Text(batch.started_at.to_rfc3339()),
            optional_text(batch.finished_at.map(|value| value.to_rfc3339())),
            XlsxCell::Number(batch.target_temperature_c),
            XlsxCell::Number(batch.target_stirrer_rpm),
            XlsxCell::Number(batch.heating_minutes),
            XlsxCell::Number(batch.stirring_minutes),
            optional_number(outcome.map(|value| value.yield_percent)),
            optional_number(outcome.map(|value| value.product_ratio)),
        ]);
    }
    rows
}

fn result_rows(outcomes: &[BatchOutcome]) -> Vec<Vec<XlsxCell>> {
    let mut rows = vec![xlsx_row([
        "batch_id",
        "target_temperature_c",
        "target_stirrer_rpm",
        "heating_minutes",
        "stirring_minutes",
        "yield_percent",
        "product_ratio",
    ])];
    for outcome in outcomes {
        rows.push(vec![
            XlsxCell::Number(outcome.batch_id as f64),
            XlsxCell::Number(outcome.target_temperature_c),
            XlsxCell::Number(outcome.target_stirrer_rpm),
            XlsxCell::Number(outcome.heating_minutes),
            XlsxCell::Number(outcome.stirring_minutes),
            XlsxCell::Number(outcome.yield_percent),
            XlsxCell::Number(outcome.product_ratio),
        ]);
    }
    rows
}

fn summary_rows(batches: &[Batch], outcomes: &[BatchOutcome]) -> Vec<Vec<XlsxCell>> {
    let completed = batches
        .iter()
        .filter(|batch| batch.finished_at.is_some())
        .count();
    let avg_yield = if outcomes.is_empty() {
        None
    } else {
        Some(
            outcomes
                .iter()
                .map(|outcome| outcome.yield_percent)
                .sum::<f64>()
                / outcomes.len() as f64,
        )
    };
    let avg_ratio = if outcomes.is_empty() {
        None
    } else {
        Some(
            outcomes
                .iter()
                .map(|outcome| outcome.product_ratio)
                .sum::<f64>()
                / outcomes.len() as f64,
        )
    };
    vec![
        xlsx_row(["metric", "value"]),
        vec![
            XlsxCell::Text("total_batches".to_string()),
            XlsxCell::Number(batches.len() as f64),
        ],
        vec![
            XlsxCell::Text("completed_batches".to_string()),
            XlsxCell::Number(completed as f64),
        ],
        vec![
            XlsxCell::Text("recorded_results".to_string()),
            XlsxCell::Number(outcomes.len() as f64),
        ],
        vec![
            XlsxCell::Text("average_yield_percent".to_string()),
            optional_number(avg_yield),
        ],
        vec![
            XlsxCell::Text("average_product_ratio".to_string()),
            optional_number(avg_ratio),
        ],
    ]
}

#[derive(Clone)]
enum XlsxCell {
    Text(String),
    Number(f64),
    Blank,
}

fn xlsx_row<const N: usize>(values: [&str; N]) -> Vec<XlsxCell> {
    values
        .into_iter()
        .map(|value| XlsxCell::Text(value.to_string()))
        .collect()
}

fn optional_text(value: Option<String>) -> XlsxCell {
    value.map(XlsxCell::Text).unwrap_or(XlsxCell::Blank)
}

fn optional_number(value: Option<f64>) -> XlsxCell {
    value.map(XlsxCell::Number).unwrap_or(XlsxCell::Blank)
}

fn worksheet_xml(rows: &[Vec<XlsxCell>]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (row_index, row) in rows.iter().enumerate() {
        let row_number = row_index + 1;
        xml.push_str(&format!(r#"<row r="{row_number}">"#));
        for (col_index, cell) in row.iter().enumerate() {
            let reference = cell_reference(col_index, row_number);
            match cell {
                XlsxCell::Text(value) => xml.push_str(&format!(
                    r#"<c r="{reference}" t="inlineStr"><is><t>{}</t></is></c>"#,
                    xml_escape(value)
                )),
                XlsxCell::Number(value) if value.is_finite() => {
                    xml.push_str(&format!(r#"<c r="{reference}"><v>{value}</v></c>"#));
                }
                _ => xml.push_str(&format!(r#"<c r="{reference}"/>"#)),
            }
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData></worksheet>");
    xml
}

fn cell_reference(mut col_index: usize, row_number: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push((b'A' + (col_index % 26) as u8) as char);
        col_index /= 26;
        if col_index == 0 {
            break;
        }
        col_index -= 1;
    }
    letters.iter().rev().collect::<String>() + &row_number.to_string()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

struct SimpleZip {
    bytes: Vec<u8>,
    files: Vec<ZipCentralDirectoryEntry>,
}

struct ZipCentralDirectoryEntry {
    name: String,
    crc32: u32,
    size: u32,
    offset: u32,
}

impl SimpleZip {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            files: Vec::new(),
        }
    }

    fn add(&mut self, name: &str, data: Vec<u8>) {
        let offset = self.bytes.len() as u32;
        let crc32 = crc32(&data);
        let size = data.len() as u32;
        let name_bytes = name.as_bytes();
        write_u32(&mut self.bytes, 0x0403_4b50);
        write_u16(&mut self.bytes, 20);
        write_u16(&mut self.bytes, 0);
        write_u16(&mut self.bytes, 0);
        write_u16(&mut self.bytes, 0);
        write_u16(&mut self.bytes, 0);
        write_u32(&mut self.bytes, crc32);
        write_u32(&mut self.bytes, size);
        write_u32(&mut self.bytes, size);
        write_u16(&mut self.bytes, name_bytes.len() as u16);
        write_u16(&mut self.bytes, 0);
        self.bytes.extend_from_slice(name_bytes);
        self.bytes.extend_from_slice(&data);
        self.files.push(ZipCentralDirectoryEntry {
            name: name.to_string(),
            crc32,
            size,
            offset,
        });
    }

    fn finish(mut self) -> Vec<u8> {
        let central_offset = self.bytes.len() as u32;
        for file in &self.files {
            let name_bytes = file.name.as_bytes();
            write_u32(&mut self.bytes, 0x0201_4b50);
            write_u16(&mut self.bytes, 20);
            write_u16(&mut self.bytes, 20);
            write_u16(&mut self.bytes, 0);
            write_u16(&mut self.bytes, 0);
            write_u16(&mut self.bytes, 0);
            write_u16(&mut self.bytes, 0);
            write_u32(&mut self.bytes, file.crc32);
            write_u32(&mut self.bytes, file.size);
            write_u32(&mut self.bytes, file.size);
            write_u16(&mut self.bytes, name_bytes.len() as u16);
            write_u16(&mut self.bytes, 0);
            write_u16(&mut self.bytes, 0);
            write_u16(&mut self.bytes, 0);
            write_u16(&mut self.bytes, 0);
            write_u32(&mut self.bytes, 0);
            write_u32(&mut self.bytes, file.offset);
            self.bytes.extend_from_slice(name_bytes);
        }
        let central_size = self.bytes.len() as u32 - central_offset;
        write_u32(&mut self.bytes, 0x0605_4b50);
        write_u16(&mut self.bytes, 0);
        write_u16(&mut self.bytes, 0);
        write_u16(&mut self.bytes, self.files.len() as u16);
        write_u16(&mut self.bytes, self.files.len() as u16);
        write_u32(&mut self.bytes, central_size);
        write_u32(&mut self.bytes, central_offset);
        write_u16(&mut self.bytes, 0);
        self.bytes
    }
}

fn write_u16(target: &mut Vec<u8>, value: u16) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(target: &mut Vec<u8>, value: u32) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn build_batch_report_markdown(
    batch: &Batch,
    outcome: Option<&BatchOutcome>,
    samples: &[SensorSampleRecord],
    events: &[ControlEvent],
) -> String {
    let mut report = String::new();
    report.push_str(&format!("# Experiment Report - Batch {}\n\n", batch.id));
    report.push_str("## Summary\n\n");
    report.push_str(&format!("- Name: {}\n", markdown_escape(&batch.name)));
    report.push_str(&format!(
        "- Process ID: {}\n",
        batch
            .process_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    report.push_str(&format!("- Started: {}\n", batch.started_at.to_rfc3339()));
    report.push_str(&format!(
        "- Finished: {}\n",
        batch
            .finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "running or not finished".to_string())
    ));
    report.push_str(&format!(
        "- Target temperature: {:.2} C\n",
        batch.target_temperature_c
    ));
    report.push_str(&format!(
        "- Target stirrer speed: {:.2} RPM\n",
        batch.target_stirrer_rpm
    ));
    report.push_str(&format!(
        "- Heating / stirring: {:.2} min / {:.2} min\n\n",
        batch.heating_minutes, batch.stirring_minutes
    ));

    report.push_str("## Product Result\n\n");
    if let Some(outcome) = outcome {
        report.push_str(&format!("- Yield: {:.2}%\n", outcome.yield_percent));
        report.push_str(&format!(
            "- Product ratio: {:.3}\n\n",
            outcome.product_ratio
        ));
    } else {
        report.push_str("- Product result has not been recorded.\n\n");
    }

    report.push_str("## Sensor Statistics\n\n");
    report.push_str("| Metric | Min | Avg | Max |\n");
    report.push_str("|---|---:|---:|---:|\n");
    for (label, stats) in [
        (
            "Temperature C",
            sample_stats(samples, |sample| sample.sample.temperature_c),
        ),
        (
            "Pressure MPa",
            sample_stats(samples, |sample| sample.sample.pressure_mpa),
        ),
        (
            "Stirrer RPM",
            sample_stats(samples, |sample| sample.sample.stirrer_rpm),
        ),
        (
            "Shake CPM",
            sample_stats(samples, |sample| sample.sample.shake_speed_cpm),
        ),
        (
            "Flow L/min",
            sample_stats(samples, |sample| sample.sample.flow_rate_l_min),
        ),
        (
            "Concentration %",
            sample_stats(samples, |sample| {
                sample.sample.product_concentration_percent
            }),
        ),
        ("pH", sample_stats(samples, |sample| sample.sample.ph)),
    ] {
        report.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            label,
            stat_value(stats.map(|value| value.min)),
            stat_value(stats.map(|value| value.avg)),
            stat_value(stats.map(|value| value.max)),
        ));
    }
    report.push('\n');

    report.push_str("## Audit Events\n\n");
    if events.is_empty() {
        report.push_str("No audit events are linked to this batch.\n");
    } else {
        for event in events.iter().rev().take(50) {
            report.push_str(&format!(
                "- {} [{}] {}\n",
                event.created_at.to_rfc3339(),
                markdown_escape(&event.event_type),
                markdown_escape(&event.reason)
            ));
        }
    }
    report
}

#[derive(Clone, Copy)]
struct NumericStats {
    min: f64,
    avg: f64,
    max: f64,
}

fn sample_stats(
    samples: &[SensorSampleRecord],
    mut value: impl FnMut(&SensorSampleRecord) -> f64,
) -> Option<NumericStats> {
    let mut count = 0.0;
    let mut sum = 0.0;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for sample in samples {
        let current = value(sample);
        if !current.is_finite() {
            continue;
        }
        count += 1.0;
        sum += current;
        min = min.min(current);
        max = max.max(current);
    }
    if count == 0.0 {
        None
    } else {
        Some(NumericStats {
            min: round2(min),
            avg: round2(sum / count),
            max: round2(max),
        })
    }
}

fn stat_value(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "--".to_string())
}

fn markdown_escape(value: &str) -> String {
    value.replace('\n', " ").replace('\r', " ")
}

struct ModbusRegisterValue {
    address: u16,
    access: &'static str,
    value: f64,
    scale: f64,
    offset: f64,
    source: &'static str,
}

fn modbus_bool_point_json(
    name: &str,
    label: &str,
    address: u16,
    access: &'static str,
    value: bool,
    source: &'static str,
) -> Value {
    json!({
        "name": name,
        "label": label,
        "address": address,
        "access": access,
        "value": value,
        "raw": if value { 1 } else { 0 },
        "source": source
    })
}

fn modbus_coils_json(state: &AppState, runtime: &RuntimeState) -> Vec<Value> {
    let _ = state;
    vec![
        modbus_bool_point_json(
            "auto_enabled",
            "auto control coil",
            0,
            "read_write",
            runtime.auto_enabled,
            "runtime_state",
        ),
        modbus_bool_point_json(
            "manual_lock",
            "manual lock coil",
            1,
            "read_write",
            runtime.manual_lock,
            "runtime_state",
        ),
        modbus_bool_point_json(
            "emergency_stop",
            "emergency stop coil",
            2,
            "read_write",
            runtime.emergency_stop,
            "runtime_state",
        ),
        modbus_bool_point_json(
            "process_running",
            "process running coil",
            3,
            "read",
            runtime.active_batch_id.is_some(),
            "runtime_state",
        ),
    ]
}

fn modbus_discrete_inputs_json(state: &AppState, runtime: &RuntimeState) -> Vec<Value> {
    let sample_fresh = runtime
        .latest_sample
        .as_ref()
        .map(|sample| {
            Utc::now().signed_duration_since(sample.captured_at)
                <= Duration::milliseconds(state.safety.control.sensor_timeout_ms)
        })
        .unwrap_or(false);
    let device_connected = runtime
        .device_status
        .as_ref()
        .map(|device| device.connected)
        .unwrap_or_else(|| runtime.latest_sample.is_some());
    let alarm_active = runtime.emergency_stop
        || runtime.last_sensor_error.is_some()
        || runtime.last_control_error.is_some();
    let tilt_state = runtime
        .latest_sample
        .as_ref()
        .map(|sample| sample.tilt_state != 0)
        .unwrap_or(false);

    vec![
        modbus_bool_point_json(
            "device_connected",
            "device connected input",
            0,
            "read",
            device_connected,
            "runtime_state",
        ),
        modbus_bool_point_json(
            "sensor_fresh",
            "fresh sensor input",
            1,
            "read",
            sample_fresh,
            "runtime_state",
        ),
        modbus_bool_point_json(
            "alarm_active",
            "alarm active input",
            2,
            "read",
            alarm_active,
            "runtime_state",
        ),
        modbus_bool_point_json(
            "tilt_state",
            "tilt state input",
            3,
            "read",
            tilt_state,
            "latest_sample",
        ),
        modbus_bool_point_json(
            "active_batch",
            "active batch input",
            4,
            "read",
            runtime.active_batch_id.is_some(),
            "runtime_state",
        ),
    ]
}

fn modbus_read_register_json(
    name: &str,
    label: &str,
    state: &AppState,
    runtime: &RuntimeState,
) -> Value {
    match modbus_register_value(state, runtime, name) {
        Ok(value) => json!({
            "name": name,
            "label": label,
            "address": value.address,
            "access": value.access,
            "value": round2(value.value),
            "raw": encode_modbus_raw(value.value, value.scale, value.offset).ok(),
            "scale": value.scale,
            "offset": value.offset,
            "source": value.source
        }),
        Err(err) => json!({
            "name": name,
            "label": label,
            "access": "read",
            "status": "unavailable",
            "error": err.message
        }),
    }
}

fn modbus_write_register_json(
    name: &str,
    label: &str,
    state: &AppState,
    runtime: &RuntimeState,
) -> Value {
    match modbus_register_value(state, runtime, name) {
        Ok(value) => json!({
            "name": name,
            "label": label,
            "address": value.address,
            "access": value.access,
            "value": round2(value.value),
            "raw": encode_modbus_raw(value.value, value.scale, value.offset).ok(),
            "scale": value.scale,
            "offset": value.offset,
            "source": value.source
        }),
        Err(err) => json!({
            "name": name,
            "label": label,
            "access": "write",
            "status": "unavailable",
            "error": err.message
        }),
    }
}

fn modbus_register_value(
    state: &AppState,
    runtime: &RuntimeState,
    register: &str,
) -> Result<ModbusRegisterValue, AppError> {
    let registers = &state.device_config.modbus.registers;
    match register {
        "temperature_c" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.temperature_c.address,
                access: "read",
                value: sample.temperature_c,
                scale: registers.temperature_c.scale,
                offset: registers.temperature_c.offset,
                source: "latest_sample",
            })
        }
        "stirrer_rpm" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.stirrer_rpm.address,
                access: "read",
                value: sample.stirrer_rpm,
                scale: registers.stirrer_rpm.scale,
                offset: registers.stirrer_rpm.offset,
                source: "latest_sample",
            })
        }
        "pressure_mpa" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.pressure_mpa.address,
                access: "read",
                value: sample.pressure_mpa,
                scale: registers.pressure_mpa.scale,
                offset: registers.pressure_mpa.offset,
                source: "latest_sample",
            })
        }
        "shake_speed_cpm" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.shake_speed_cpm.address,
                access: "read",
                value: sample.shake_speed_cpm,
                scale: registers.shake_speed_cpm.scale,
                offset: registers.shake_speed_cpm.offset,
                source: "latest_sample",
            })
        }
        "tilt_angle_deg" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.tilt_angle_deg.address,
                access: "read",
                value: sample.tilt_angle_deg,
                scale: registers.tilt_angle_deg.scale,
                offset: registers.tilt_angle_deg.offset,
                source: "latest_sample",
            })
        }
        "flow_rate_l_min" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.flow_rate_l_min.address,
                access: "read",
                value: sample.flow_rate_l_min,
                scale: registers.flow_rate_l_min.scale,
                offset: registers.flow_rate_l_min.offset,
                source: "latest_sample",
            })
        }
        "product_concentration_percent" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.product_concentration_percent.address,
                access: "read",
                value: sample.product_concentration_percent,
                scale: registers.product_concentration_percent.scale,
                offset: registers.product_concentration_percent.offset,
                source: "latest_sample",
            })
        }
        "ph" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.ph.address,
                access: "read",
                value: sample.ph,
                scale: registers.ph.scale,
                offset: registers.ph.offset,
                source: "latest_sample",
            })
        }
        "target_temperature_c" => Ok(ModbusRegisterValue {
            address: registers.target_temperature_c.address,
            access: "write",
            value: runtime.targets.temperature_c,
            scale: registers.target_temperature_c.scale,
            offset: registers.target_temperature_c.offset,
            source: "runtime_targets",
        }),
        "target_stirrer_rpm" => Ok(ModbusRegisterValue {
            address: registers.target_stirrer_rpm.address,
            access: "write",
            value: runtime.targets.stirrer_rpm,
            scale: registers.target_stirrer_rpm.scale,
            offset: registers.target_stirrer_rpm.offset,
            source: "runtime_targets",
        }),
        "target_shake_speed_cpm" => Ok(ModbusRegisterValue {
            address: registers.target_shake_speed_cpm.address,
            access: "write",
            value: runtime.targets.shake_speed_cpm,
            scale: registers.target_shake_speed_cpm.scale,
            offset: registers.target_shake_speed_cpm.offset,
            source: "runtime_targets",
        }),
        "target_pressure_mpa" => Ok(ModbusRegisterValue {
            address: registers.target_pressure_mpa.address,
            access: "write",
            value: runtime.targets.target_pressure_mpa,
            scale: registers.target_pressure_mpa.scale,
            offset: registers.target_pressure_mpa.offset,
            source: "runtime_targets",
        }),
        "heat_time_s" => Ok(ModbusRegisterValue {
            address: registers.heat_time_s.address,
            access: "write",
            value: runtime.targets.heat_time_s,
            scale: registers.heat_time_s.scale,
            offset: registers.heat_time_s.offset,
            source: "runtime_targets",
        }),
        "hold_time_s" => Ok(ModbusRegisterValue {
            address: registers.hold_time_s.address,
            access: "write",
            value: runtime.targets.hold_time_s,
            scale: registers.hold_time_s.scale,
            offset: registers.hold_time_s.offset,
            source: "runtime_targets",
        }),
        "cool_time_s" => Ok(ModbusRegisterValue {
            address: registers.cool_time_s.address,
            access: "write",
            value: runtime.targets.cool_time_s,
            scale: registers.cool_time_s.scale,
            offset: registers.cool_time_s.offset,
            source: "runtime_targets",
        }),
        _ => Err(AppError::not_found("modbus register not found")),
    }
}

fn modbus_write_register_config<'a>(
    registers: &'a RegistersConfig,
    register: &str,
) -> Option<&'a WriteRegister> {
    match register {
        "target_temperature_c" => Some(&registers.target_temperature_c),
        "target_stirrer_rpm" => Some(&registers.target_stirrer_rpm),
        "target_shake_speed_cpm" => Some(&registers.target_shake_speed_cpm),
        "target_pressure_mpa" => Some(&registers.target_pressure_mpa),
        "heat_time_s" => Some(&registers.heat_time_s),
        "hold_time_s" => Some(&registers.hold_time_s),
        "cool_time_s" => Some(&registers.cool_time_s),
        _ => None,
    }
}

fn modbus_write_register_applied_value(
    targets: &ControlTargets,
    register: &str,
) -> Result<f64, AppError> {
    match register {
        "target_temperature_c" => Ok(targets.temperature_c),
        "target_stirrer_rpm" => Ok(targets.stirrer_rpm),
        "target_shake_speed_cpm" => Ok(targets.shake_speed_cpm),
        "target_pressure_mpa" => Ok(targets.target_pressure_mpa),
        "heat_time_s" => Ok(targets.heat_time_s),
        "hold_time_s" => Ok(targets.hold_time_s),
        "cool_time_s" => Ok(targets.cool_time_s),
        _ => Err(AppError::bad_request(
            "register is not writable through the Modbus debug API",
        )),
    }
}

fn encode_modbus_raw(value: f64, scale: f64, offset: f64) -> Result<u16, AppError> {
    if scale == 0.0 {
        return Err(AppError::bad_request("register scale must not be zero"));
    }
    let raw = ((value - offset) / scale).round();
    if !(0.0..=u16::MAX as f64).contains(&raw) {
        return Err(AppError::bad_request("value cannot be encoded as u16"));
    }
    Ok(raw as u16)
}

pub struct ApiJson<T>(pub T);

#[async_trait::async_trait]
impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(AppError::from_json_rejection)
    }
}

fn clean_label(value: Option<String>, fallback: &str, max_chars: usize) -> String {
    let trimmed = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    trimmed.chars().take(max_chars).collect()
}

fn clean_optional_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(max_chars).collect())
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

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
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

#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
        }
    }

    fn from_json_rejection(rejection: JsonRejection) -> Self {
        Self {
            status: rejection.status(),
            message: rejection.body_text(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(V1Envelope {
                code: self.status.as_u16() as i32,
                message: self.message.clone(),
                data: json!({ "error": self.message }),
            }),
        )
            .into_response()
    }
}
