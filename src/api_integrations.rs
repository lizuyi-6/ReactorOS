use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    api_auth::{audit_actor_for_user, require_permission, AuthUser, Permission},
    clean_label, ensure_target_update_interlock_clear, ensure_targets_allowed,
    optional_present_i64, optional_present_number, start_process_lifecycle, stop_process_lifecycle,
    success, validate_range, validate_stir_speed, validate_target_temperature, ApiJson, AppError,
    AppState, TargetUpdateInterlockMode, V1Envelope,
};
use crate::{
    control::SafeCommand,
    db::{AuditActor, IntegrationTask},
    number::round2,
    state::ControlTargets,
};

#[derive(Debug, Deserialize)]
pub(super) struct AinasTaskQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AinasTaskRequest {
    #[serde(default, deserialize_with = "super::deserialize_optional_string_field")]
    pub external_task_id: Option<String>,
    pub action: String,
    #[serde(
        default,
        deserialize_with = "super::deserialize_optional_i64_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub process_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_number_field")]
    pub target_temperature_c: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_number_field")]
    pub target_stirrer_rpm: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_number_field")]
    pub target_shake_speed_cpm: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_number_field")]
    pub target_pressure_mpa: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_number_field")]
    pub heat_time_s: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_number_field")]
    pub hold_time_s: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_number_field")]
    pub cool_time_s: Option<Option<f64>>,
    #[serde(default, deserialize_with = "super::deserialize_optional_string_field")]
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
    // Gate the integration dispatch path first. The integration path is
    // reachable from AINAS remote clients and from MQTT, so it requires a
    // separate permission that operator does NOT carry; otherwise operator
    // would inherit SetSafeTargets / StartStopProcess and could push a
    // remote action through /api/integrations/ainas/tasks. Engineer and
    // admin are still subject to the action-specific check below.
    require_permission(&headers, Permission::ApplyIntegrationTask)?;
    let user = require_ainas_action_permission(&headers, action)?;
    Ok(Json(success(
        execute_integration_task(&state, "ainas", payload, &audit_actor_for_user(&user)).await?,
    )))
}

pub(crate) async fn execute_integration_task(
    state: &AppState,
    source: &str,
    mut payload: AinasTaskRequest,
    actor: &AuditActor,
) -> Result<IntegrationTask, AppError> {
    let action = normalize_ainas_action(&payload.action)?;
    payload.action = action.to_string();
    validate_integration_action_shape(action, &payload)?;
    payload.external_task_id = clean_optional_text(payload.external_task_id.as_deref(), 120);
    payload.reason = clean_optional_text(payload.reason.as_deref(), 240);
    let request = serde_json::to_value(&payload).map_err(|err| {
        AppError::from(anyhow::anyhow!(
            "failed to serialize integration task request: {err}"
        ))
    })?;
    let source = clean_optional_text(Some(source), 40).unwrap_or_else(|| "integration".to_string());
    let task = state
        .db
        .create_integration_task_sqlx(
            &source,
            payload.external_task_id.as_deref(),
            action,
            &request,
        )
        .await?;
    ensure_integration_task_payload_matches(&task, action, &request)?;
    if task.status != "received" {
        return Ok(task);
    }
    let Some(task) = state
        .db
        .mark_integration_task_executing_sqlx(task.id)
        .await?
    else {
        return Err(AppError::not_found("AINAS task not found before execution"));
    };
    if task.status != "executing" {
        return Ok(task);
    }

    match execute_ainas_task(state, action, &payload, actor).await {
        Ok(response) => {
            let update_result = state
                .db
                .update_integration_task_sqlx(task.id, "executed", &response)
                .await;
            let Some(task) = (match update_result {
                Ok(task) => task,
                Err(err) => {
                    latch_integration_receipt_failure_after_action(state, action, &err.to_string())
                        .await;
                    return Err(err.into());
                }
            }) else {
                let err = AppError::not_found("AINAS task not found after execution");
                latch_integration_receipt_failure_after_action(state, action, err.message()).await;
                return Err(err);
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

fn ensure_integration_task_payload_matches(
    task: &IntegrationTask,
    action: &str,
    request: &Value,
) -> Result<(), AppError> {
    if task.action == action && task.request == *request {
        return Ok(());
    }
    Err(AppError::conflict(format!(
        "integration task external_task_id {:?} already exists for source '{}' with a different request; use a new external_task_id",
        task.external_task_id, task.source
    )))
}

async fn latch_integration_receipt_failure_after_action(state: &AppState, action: &str, err: &str) {
    let mut runtime = state.runtime.write().await;
    match action {
        "start_process" | "stop_process" => {
            runtime.latch_audit_failure_after_device_action(
                &format!("integration task {action} receipt"),
                err,
            );
        }
        "set_targets" => {
            runtime.latch_control_fault(format!(
                "integration task set_targets receipt failed after target intent commit: {err}"
            ));
        }
        _ => {}
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

fn require_ainas_action_permission(
    headers: &HeaderMap,
    action: &str,
) -> Result<AuthUser, AppError> {
    let permission = match action {
        "set_targets" => Permission::SetSafeTargets,
        "start_process" | "stop_process" => Permission::StartStopProcess,
        _ => {
            return Err(AppError::bad_request(
                "AINAS action must be set_targets, start_process, or stop_process",
            ));
        }
    };
    require_permission(headers, permission)
}

async fn execute_ainas_task(
    state: &AppState,
    action: &str,
    payload: &AinasTaskRequest,
    actor: &AuditActor,
) -> Result<Value, AppError> {
    match action {
        "set_targets" => {
            let targets = apply_ainas_targets(state, payload, actor).await?;
            Ok(json!({
                "action": "set_targets",
                "status": "executed",
                "safety": "validated_against_configured_limits",
                "targets": targets
            }))
        }
        "start_process" => {
            let process_id = optional_present_i64(payload.process_id, "process_id")?
                .ok_or_else(|| AppError::bad_request("process_id is required for start_process"))?;
            let response =
                start_process_lifecycle(state, process_id, "ainas_process_started", actor).await?;
            json_response(response)
        }
        "stop_process" => {
            let process_id = optional_present_i64(payload.process_id, "process_id")?;
            let response = stop_process_lifecycle(
                state,
                process_id,
                "ainas_process_stopped",
                payload.reason.clone(),
                actor,
            )
            .await?;
            json_response(response)
        }
        _ => Err(AppError::bad_request(
            "AINAS action must be set_targets, start_process, or stop_process",
        )),
    }
}

fn validate_integration_action_shape(
    action: &str,
    payload: &AinasTaskRequest,
) -> Result<(), AppError> {
    match action {
        "start_process" => {
            optional_present_i64(payload.process_id, "process_id")?
                .ok_or_else(|| AppError::bad_request("process_id is required for start_process"))?;
        }
        "set_targets" => {
            if optional_present_i64(payload.process_id, "process_id")?.is_some() {
                return Err(AppError::bad_request(
                    "process_id is not accepted for set_targets",
                ));
            }
        }
        "stop_process" => {
            optional_present_i64(payload.process_id, "process_id")?;
        }
        _ => {}
    }
    Ok(())
}

async fn apply_ainas_targets(
    state: &AppState,
    payload: &AinasTaskRequest,
    actor: &AuditActor,
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
    validate_integration_target_payload(state, payload)?;
    let targets = validate_integration_targets(
        state,
        ControlTargets {
            temperature_c: optional_present_number(
                payload.target_temperature_c,
                "target_temperature_c",
            )?
            .unwrap_or(current.temperature_c),
            heat_time_s: optional_present_number(payload.heat_time_s, "heat_time_s")?
                .unwrap_or(current.heat_time_s),
            hold_time_s: optional_present_number(payload.hold_time_s, "hold_time_s")?
                .unwrap_or(current.hold_time_s),
            cool_time_s: optional_present_number(payload.cool_time_s, "cool_time_s")?
                .unwrap_or(current.cool_time_s),
            stirrer_rpm: optional_present_number(payload.target_stirrer_rpm, "target_stirrer_rpm")?
                .unwrap_or(current.stirrer_rpm),
            shake_speed_cpm: optional_present_number(
                payload.target_shake_speed_cpm,
                "target_shake_speed_cpm",
            )?
            .unwrap_or(current.shake_speed_cpm),
            target_pressure_mpa: optional_present_number(
                payload.target_pressure_mpa,
                "target_pressure_mpa",
            )?
            .unwrap_or(current.target_pressure_mpa),
        },
    )?;
    ensure_targets_allowed(&state.safety, &targets)?;
    let acknowledged_safety_latches = {
        let runtime = {
            let mut runtime = state.runtime.write().await;
            runtime.auto_enabled = false;
            runtime.clone()
        };
        ensure_target_update_interlock_clear(
            state,
            &runtime,
            TargetUpdateInterlockMode::DesiredTargets,
        )?;
        crate::api::SafetyLatchGenerations::from_runtime(&runtime)
    };
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
            actor,
        )
        .await?;
    crate::api::commit_targets_after_final_interlock(
        state,
        &targets,
        TargetUpdateInterlockMode::DesiredTargets,
        Some(&current),
        Some(acknowledged_safety_latches),
    )
    .await?;
    Ok(targets)
}

fn validate_integration_target_payload(
    state: &AppState,
    payload: &AinasTaskRequest,
) -> Result<(), AppError> {
    if let Some(value) =
        optional_present_number(payload.target_temperature_c, "target_temperature_c")?
    {
        validate_target_temperature(&state.safety, value)?;
    }
    if let Some(value) = optional_present_number(payload.target_stirrer_rpm, "target_stirrer_rpm")?
    {
        validate_stir_speed(&state.safety, value)?;
    }
    if let Some(value) =
        optional_present_number(payload.target_shake_speed_cpm, "target_shake_speed_cpm")?
    {
        validate_range("target_shake_speed_cpm", value, 0.0, 60.0)?;
    }
    if let Some(value) =
        optional_present_number(payload.target_pressure_mpa, "target_pressure_mpa")?
    {
        validate_range("target_pressure_mpa", value, 0.0, 10.0)?;
    }
    if let Some(value) = optional_present_number(payload.heat_time_s, "heat_time_s")? {
        validate_range(
            "heat_time_s",
            value,
            0.0,
            state.safety.optimizer.max_heating_minutes * 60.0,
        )?;
    }
    if let Some(value) = optional_present_number(payload.hold_time_s, "hold_time_s")? {
        validate_range(
            "hold_time_s",
            value,
            0.0,
            state.safety.optimizer.max_stirring_minutes * 60.0,
        )?;
    }
    if let Some(value) = optional_present_number(payload.cool_time_s, "cool_time_s")? {
        validate_range("cool_time_s", value, 0.0, 3600.0)?;
    }
    Ok(())
}

fn validate_integration_targets(
    state: &AppState,
    targets: ControlTargets,
) -> Result<ControlTargets, AppError> {
    validate_target_temperature(&state.safety, targets.temperature_c)
        .map_err(|err| err.with_message_prefix("target_temperature_c"))?;
    validate_stir_speed(&state.safety, targets.stirrer_rpm)
        .map_err(|err| err.with_message_prefix("target_stirrer_rpm"))?;
    validate_range("target_shake_speed_cpm", targets.shake_speed_cpm, 0.0, 60.0)?;
    validate_range(
        "target_pressure_mpa",
        targets.target_pressure_mpa,
        0.0,
        10.0,
    )?;
    validate_range(
        "heat_time_s",
        targets.heat_time_s,
        0.0,
        state.safety.optimizer.max_heating_minutes * 60.0,
    )?;
    validate_range(
        "hold_time_s",
        targets.hold_time_s,
        0.0,
        state.safety.optimizer.max_stirring_minutes * 60.0,
    )?;
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

fn clean_optional_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    value.and_then(|value| {
        let cleaned = clean_label(Some(value.to_string()), "", max_chars);
        (!cleaned.is_empty()).then_some(cleaned)
    })
}

fn json_response<T: Serialize>(value: T) -> Result<Value, AppError> {
    serde_json::to_value(value)
        .map_err(|err| AppError::from(anyhow::anyhow!("failed to serialize response: {err}")))
}
