use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    api_auth::{require_permission, Permission},
    clean_label, ensure_targets_allowed, start_process_lifecycle, stop_process_lifecycle, success,
    ApiJson, AppError, AppState, V1Envelope,
};
use crate::{
    control::{clamp_operator_targets, SafeCommand},
    db::IntegrationTask,
    state::ControlTargets,
};

#[derive(Debug, Deserialize)]
pub(super) struct AinasTaskQuery {
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

pub(super) async fn list_ainas_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AinasTaskQuery>,
) -> Result<Json<V1Envelope<Vec<IntegrationTask>>>, AppError> {
    require_permission(&headers, Permission::ViewAudit)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    Ok(Json(success(
        state
            .db
            .integration_tasks_sqlx(Some("ainas"), limit)
            .await?,
    )))
}

pub(super) async fn get_ainas_task(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<V1Envelope<IntegrationTask>>, AppError> {
    require_permission(&headers, Permission::ViewAudit)?;
    let Some(task) = state.db.integration_task_sqlx(id).await? else {
        return Err(AppError::not_found("AINAS task not found"));
    };
    if task.source != "ainas" {
        return Err(AppError::not_found("AINAS task not found"));
    }
    Ok(Json(success(task)))
}

pub(super) async fn create_ainas_task(
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

pub(crate) async fn execute_integration_task(
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
    let task = state
        .db
        .create_integration_task_sqlx(&source, external_task_id.as_deref(), action, &request)
        .await?;

    match execute_ainas_task(state, action, &payload).await {
        Ok(response) => {
            let Some(task) = state
                .db
                .update_integration_task_sqlx(task.id, "executed", &response)
                .await?
            else {
                return Err(AppError::not_found("AINAS task not found after execution"));
            };
            Ok(task)
        }
        Err(err) => {
            let status = if err.status_code().is_server_error() {
                "failed"
            } else {
                "rejected"
            };
            let message = err.message().to_string();
            let response = json!({
                "code": err.status_code().as_u16(),
                "message": message.clone(),
                "data": { "error": message }
            });
            state
                .db
                .update_integration_task_sqlx(task.id, status, &response)
                .await?;
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
    state
        .db
        .insert_control_event_sqlx(
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
        )
        .await?;
    Ok(targets)
}

fn clean_optional_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(max_chars).collect())
}

fn json_response<T: Serialize>(value: T) -> Result<Value, AppError> {
    serde_json::to_value(value)
        .map_err(|err| AppError::from(anyhow::anyhow!("failed to serialize response: {err}")))
}
