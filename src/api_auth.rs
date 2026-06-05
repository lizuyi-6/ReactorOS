use std::env;

use axum::http::HeaderMap;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::AppError;

#[derive(Debug, Deserialize)]
pub(crate) struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct LoginResponse {
    pub token: String,
    pub user: AuthUser,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AuthUser {
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
pub(crate) enum Permission {
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

pub(crate) fn login_response(payload: LoginRequest) -> Result<LoginResponse, AppError> {
    let username = payload.username.trim().to_ascii_lowercase();
    let Some(role) = role_for_login(&username, &payload.password) else {
        return Err(AppError::unauthorized("invalid username or password"));
    };
    let expires_at = Utc::now() + Duration::hours(12);
    let token = issue_auth_token(&username, role, expires_at.timestamp());
    Ok(LoginResponse {
        token,
        user: auth_user(&username, role),
        expires_at: expires_at.to_rfc3339(),
    })
}

pub(crate) fn permission_policy() -> Value {
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

pub(crate) fn authenticated_user(headers: &HeaderMap) -> Result<AuthUser, AppError> {
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

pub(crate) fn require_permission(
    headers: &HeaderMap,
    permission: Permission,
) -> Result<AuthUser, AppError> {
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

pub(crate) fn require_admin(headers: &HeaderMap) -> Result<AuthUser, AppError> {
    let user = authenticated_user(headers)?;
    let role = role_from_name(&user.role)
        .ok_or_else(|| AppError::unauthorized("invalid bearer session role"))?;
    if role != AuthRole::Admin {
        return Err(AppError::forbidden(
            "modbus debug writes require admin role",
        ));
    }
    Ok(user)
}

fn permission_names_for_role(role: AuthRole) -> Vec<&'static str> {
    all_permissions()
        .into_iter()
        .filter(|permission| role_allows(role, *permission))
        .map(permission_name)
        .collect()
}

fn blocked_permission_names_for_role(role: AuthRole) -> Vec<&'static str> {
    all_permissions()
        .into_iter()
        .filter(|permission| !role_allows(role, *permission))
        .map(permission_name)
        .collect()
}

fn all_permissions() -> [Permission; 13] {
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

fn issue_auth_token(username: &str, role: AuthRole, expires_at: i64) -> String {
    let payload = format!("{}:{}:{}", username, role_name(role), expires_at);
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
