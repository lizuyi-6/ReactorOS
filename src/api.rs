use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    ai_provider::{
        fallback_envelope, local_envelope, stepfun_envelope, AiProvider, AiRecommendationEnvelope,
        AiRecommendationProvider,
    },
    config::SafetyConfig,
    control::{clamp_operator_targets, SafeCommand},
    db::{Batch, BatchOutcome, ControlEvent, Db, ProductResult},
    memory::{AiMemory, AiMemorySummary, LimitLevel, SensorLimit},
    optimizer::{recommend_with_memory, Recommendation},
    state::{ControlTargets, RuntimeState, SensorSnapshot, SharedState},
};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub runtime: SharedState,
    pub safety: Arc<SafetyConfig>,
    pub ai_memory: Arc<AiMemory>,
    pub ai_provider: Option<Arc<AiProvider>>,
    pub test_reset_enabled: bool,
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
    pub recent_samples: Vec<SensorSnapshot>,
    pub recent_batches: Vec<Batch>,
    pub recent_outcomes: Vec<BatchOutcome>,
    pub recent_events: Vec<ControlEvent>,
    pub ai_memory: AiMemorySummary,
}

#[derive(Debug, Serialize)]
pub struct V1Envelope<T> {
    pub code: i32,
    pub message: String,
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub struct StartBatchRequest {
    pub name: Option<String>,
    pub target_temperature_c: Option<f64>,
    pub target_stirrer_rpm: Option<f64>,
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
pub struct V1HistoryQuery {
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub interval: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
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
    pub flow_rate_l_min: f64,
    pub product_concentration_percent: f64,
    pub ph: f64,
}

pub fn router(state: AppState, assets: PathBuf) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/live", get(live))
        .route("/api/v1/reactor/:device_id/control", post(v1_control))
        .route(
            "/api/v1/reactor/:device_id/samples",
            post(v1_pipeline_sample),
        )
        .route("/api/v1/reactor/:device_id/realtime", get(v1_realtime))
        .route("/api/v1/reactor/:device_id/history", get(v1_history))
        .route("/api/v1/reactor/:device_id/process", post(v1_process))
        .route("/ws/v1/reactor/:device_id/realtime", get(v1_realtime_ws))
        .route("/api/batches/start", post(start_batch))
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
        .nest_service(
            "/",
            ServeDir::new(&assets).not_found_service(ServeFile::new(assets.join("index.html"))),
        )
        .with_state(state)
}

pub async fn serve(state: AppState, assets: PathBuf, bind: SocketAddr) -> Result<()> {
    let router = router(state, assets);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("listening on http://{bind}");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "reactor-edge-daemon",
    })
}

async fn live(State(state): State<AppState>) -> Result<Json<LiveResponse>, AppError> {
    let runtime = state.runtime.read().await.clone();
    ensure_fresh_sample(&state, &runtime)?;
    let recent_samples = state.db.recent_samples(480)?;
    let recent_batches = state.db.recent_batches(20)?;
    let recent_outcomes = state.db.recent_batch_outcomes(20)?;
    let recent_events = state.db.recent_control_events(20)?;
    let ai_memory = AiMemorySummary::from(state.ai_memory.as_ref());
    let recommendation = current_cached_or_local_recommendation(&state)?;
    let ai_provider = local_provider_for(&state);
    Ok(Json(LiveResponse {
        runtime,
        latest_recommendation: Some(recommendation),
        ai_provider,
        recent_samples,
        recent_batches,
        recent_outcomes,
        recent_events,
        ai_memory,
    }))
}

async fn v1_control(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(payload): Json<V1ControlRequest>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
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
    Json(payload): Json<PipelineSampleRequest>,
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
        .map(|sample| {
            json!({
                "device_id": device_id,
                "timestamp": sample.captured_at.to_rfc3339(),
                "data": {
                    "current_temp": sample.temperature_c,
                    "current_pressure": sample.pressure_mpa,
                    "stir_speed": sample.stirrer_rpm,
                    "shake_speed": sample.shake_speed_cpm,
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
        "items": rows
    }))))
}

async fn v1_process(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(payload): Json<V1ProcessRequest>,
) -> Result<Json<V1Envelope<Value>>, AppError> {
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
    Json(payload): Json<StartBatchRequest>,
) -> Result<Json<Batch>, AppError> {
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
    let heating_minutes = round2(payload.heating_minutes.unwrap_or(60.0));
    let stirring_minutes = round2(payload.stirring_minutes.unwrap_or(60.0));
    let name = payload.name.unwrap_or_else(|| "batch".to_string());
    let batch = state.db.create_batch(
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
                shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
            },
        );
    }
    state.db.insert_control_event(
        Some(batch.id),
        "batch_started",
        None,
        "batch started and runtime targets updated",
    )?;
    Ok(Json(batch))
}

async fn finish_batch(
    State(state): State<AppState>,
    axum::extract::Path(batch_id): axum::extract::Path<i64>,
) -> Result<StatusCode, AppError> {
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
    Json(payload): Json<ProductResultRequest>,
) -> Result<Json<AiRecommendationEnvelope>, AppError> {
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
    let recommendation = generate_recommendation(&state).await?;
    state.db.insert_recommendation(&recommendation)?;
    state.db.insert_control_event(
        Some(payload.batch_id),
        "product_result_recorded",
        None,
        "product result saved and recommendation regenerated",
    )?;
    Ok(Json(recommendation_envelope(&state, recommendation).await))
}

async fn set_auto(
    State(state): State<AppState>,
    Json(payload): Json<AutoRequest>,
) -> Result<StatusCode, AppError> {
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
    Json(payload): Json<ManualLockRequest>,
) -> Result<StatusCode, AppError> {
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
    Json(payload): Json<TargetRequest>,
) -> Result<Json<ControlTargets>, AppError> {
    let targets = clamp_operator_targets(&state.safety, {
        let current = state.runtime.read().await.targets.clone();
        ControlTargets {
            temperature_c: payload.temperature_c,
            heat_time_s: current.heat_time_s,
            hold_time_s: current.hold_time_s,
            cool_time_s: current.cool_time_s,
            stirrer_rpm: payload.stirrer_rpm,
            shake_speed_cpm: current.shake_speed_cpm,
            target_pressure_mpa: current.target_pressure_mpa,
        }
    });
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

async fn emergency_stop(State(state): State<AppState>) -> Result<StatusCode, AppError> {
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

async fn reset_emergency_stop(State(state): State<AppState>) -> Result<StatusCode, AppError> {
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
    Json(payload): Json<PipelineSampleRequest>,
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
        runtime.last_control_error = None;
        runtime.active_batch_id
    };
    state.db.insert_sample(active_batch_id, &sample)?;
    Ok(sample)
}

fn pipeline_sample_from_request(
    payload: PipelineSampleRequest,
) -> Result<SensorSnapshot, AppError> {
    Ok(SensorSnapshot {
        temperature_c: round2_finite("temperature_c", payload.temperature_c)?,
        pressure_mpa: round2_finite("pressure_mpa", payload.pressure_mpa)?,
        stirrer_rpm: round2_finite("stirrer_rpm", payload.stirrer_rpm)?,
        shake_speed_cpm: round2_finite("shake_speed_cpm", payload.shake_speed_cpm)?,
        flow_rate_l_min: round2_finite("flow_rate_l_min", payload.flow_rate_l_min)?,
        product_concentration_percent: round2_finite(
            "product_concentration_percent",
            payload.product_concentration_percent,
        )?,
        ph: round2_finite("ph", payload.ph)?,
        captured_at: Utc::now(),
    })
}

fn ensure_fresh_sample(state: &AppState, runtime: &RuntimeState) -> Result<(), AppError> {
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

fn local_recommendation(state: &AppState) -> Result<Recommendation, AppError> {
    let outcomes = state.db.batch_outcomes()?;
    Ok(recommend_with_memory(
        &state.safety.optimizer,
        Some(&state.ai_memory),
        &outcomes,
    ))
}

fn current_cached_or_local_recommendation(state: &AppState) -> Result<Recommendation, AppError> {
    if let Some(recommendation) = state.db.latest_recommendation()? {
        return Ok(recommendation);
    }
    local_recommendation(state)
}

async fn generate_recommendation(state: &AppState) -> Result<Recommendation, AppError> {
    let outcomes = state.db.batch_outcomes()?;
    let fallback =
        recommend_with_memory(&state.safety.optimizer, Some(&state.ai_memory), &outcomes);
    let Some(provider) = &state.ai_provider else {
        return Ok(fallback);
    };
    match provider
        .recommend(
            &state.safety.optimizer,
            &state.ai_memory,
            &outcomes,
            &fallback,
        )
        .await
    {
        Ok(recommendation) => Ok(recommendation),
        Err(err) => {
            tracing::warn!("StepFun recommendation failed; falling back to local optimizer: {err}");
            let mut recommendation = fallback;
            recommendation.rationale = format!(
                "StepFun 调用失败，已回退本地优化器：{}",
                recommendation.rationale
            );
            Ok(recommendation)
        }
    }
}

async fn recommendation_envelope(
    state: &AppState,
    recommendation: Recommendation,
) -> AiRecommendationEnvelope {
    let Some(provider) = &state.ai_provider else {
        return local_envelope(recommendation);
    };
    if recommendation.rationale.starts_with("StepFun 调用失败") {
        fallback_envelope(
            recommendation,
            provider.model_name(),
            "StepFun request failed; local optimizer fallback was used",
        )
    } else if recommendation.rationale.starts_with("StepFun:") {
        stepfun_envelope(recommendation, provider.model_name())
    } else {
        fallback_envelope(
            recommendation,
            provider.model_name(),
            "StepFun output was rejected by safety validation; local optimizer fallback was used",
        )
    }
}

fn local_provider_for(state: &AppState) -> AiRecommendationProvider {
    if let Some(provider) = &state.ai_provider {
        AiRecommendationProvider {
            mode: "stepfun_configured_cached_or_local".to_string(),
            model: provider.model_name().to_string(),
            fallback_reason: None,
        }
    } else {
        AiRecommendationProvider {
            mode: "local_optimizer".to_string(),
            model: "local-tpe-lite".to_string(),
            fallback_reason: None,
        }
    }
}

fn success<T>(data: T) -> V1Envelope<T> {
    V1Envelope {
        code: 0,
        message: "success".to_string(),
        data,
    }
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

fn device_status(runtime: &RuntimeState) -> &'static str {
    if runtime.emergency_stop || runtime.last_control_error.is_some() {
        "error"
    } else if runtime.active_batch_id.is_some() {
        "running"
    } else {
        "idle"
    }
}

fn phase_for(runtime: &RuntimeState) -> &'static str {
    if runtime.emergency_stop || runtime.last_control_error.is_some() {
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

fn alarms_for(
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
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
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
