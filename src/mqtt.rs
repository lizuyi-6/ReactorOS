use std::{fs, path::Path, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, Transport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio::time::{interval, MissedTickBehavior};

use crate::{
    api::{alarms_for, execute_integration_task, AinasTaskRequest, AppError, AppState},
    db::IntegrationTask,
    modbus_tcp::ModbusTcpConfig,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IntegrationConfig {
    #[serde(default)]
    pub mqtt: MqttBridgeConfig,
    #[serde(default)]
    pub modbus_tcp: ModbusTcpConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MqttBridgeConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub use_tls: bool,
    pub ca_cert: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub keep_alive_s: u64,
    pub queue_capacity: usize,
    pub task_topic: String,
    pub receipt_topic: String,
    pub status_topic: String,
    pub alert_topic: String,
    #[serde(default = "default_alert_interval_s")]
    pub alert_interval_s: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MqttBridgeStatus {
    pub enabled: bool,
    pub connected: bool,
    pub broker: String,
    pub client_id: String,
    pub use_tls: bool,
    pub ca_cert_configured: bool,
    pub client_cert_configured: bool,
    pub task_topic: String,
    pub receipt_topic: String,
    pub status_topic: String,
    pub alert_topic: String,
    pub alert_interval_s: u64,
    pub last_error: Option<String>,
    pub last_task_id: Option<i64>,
    pub last_alert_active_count: usize,
    pub last_alert_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MqttAlertSnapshot {
    pub device_id: &'static str,
    pub active: bool,
    pub active_count: usize,
    pub high_count: usize,
    pub warning_count: usize,
    pub emergency_stop: bool,
    pub manual_lock: bool,
    pub sensor_fresh: bool,
    pub active_batch_id: Option<i64>,
    pub alarms: Vec<Value>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MqttTaskReceipt {
    pub ok: bool,
    pub source: &'static str,
    pub task_id: Option<i64>,
    pub external_task_id: Option<String>,
    pub action: Option<String>,
    pub status: String,
    pub response: Option<Value>,
    pub error: Option<String>,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            mqtt: MqttBridgeConfig::default(),
            modbus_tcp: ModbusTcpConfig::default(),
        }
    }
}

impl Default for MqttBridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 8883,
            client_id: "xingshu-reactor-001".to_string(),
            username: None,
            password: None,
            use_tls: true,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            keep_alive_s: 30,
            queue_capacity: 16,
            task_topic: "xingshu/reactor_001/tasks".to_string(),
            receipt_topic: "xingshu/reactor_001/task_receipts".to_string(),
            status_topic: "xingshu/reactor_001/status".to_string(),
            alert_topic: "xingshu/reactor_001/alerts".to_string(),
            alert_interval_s: default_alert_interval_s(),
        }
    }
}

fn default_alert_interval_s() -> u64 {
    5
}

impl MqttBridgeStatus {
    fn from_config(config: &MqttBridgeConfig) -> Self {
        Self {
            enabled: config.enabled,
            connected: false,
            broker: format!("{}:{}", config.host, config.port),
            client_id: config.client_id.clone(),
            use_tls: config.use_tls,
            ca_cert_configured: config.ca_cert.is_some(),
            client_cert_configured: crate::tls::paired_paths(
                &config.client_cert,
                &config.client_key,
                "MQTT TLS client",
            )
            .is_ok_and(|paths| paths.is_some()),
            task_topic: config.task_topic.clone(),
            receipt_topic: config.receipt_topic.clone(),
            status_topic: config.status_topic.clone(),
            alert_topic: config.alert_topic.clone(),
            alert_interval_s: config.alert_interval_s,
            last_error: None,
            last_task_id: None,
            last_alert_active_count: 0,
            last_alert_at: None,
            updated_at: Utc::now(),
        }
    }
}

type SharedMqttStatus = Arc<RwLock<MqttBridgeStatus>>;

static MQTT_STATUS: std::sync::OnceLock<SharedMqttStatus> = std::sync::OnceLock::new();

fn status_handle() -> SharedMqttStatus {
    MQTT_STATUS
        .get_or_init(|| {
            Arc::new(RwLock::new(MqttBridgeStatus::from_config(
                &MqttBridgeConfig::default(),
            )))
        })
        .clone()
}

pub async fn mqtt_status_snapshot() -> MqttBridgeStatus {
    status_handle().read().await.clone()
}

pub fn load_integration_config(path: impl AsRef<Path>) -> Result<IntegrationConfig> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(IntegrationConfig::default());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read integration config {}", path.display()))?;
    toml::from_str(&raw)
        .with_context(|| format!("failed to parse integration config {}", path.display()))
}

pub fn start_mqtt_bridge(config: MqttBridgeConfig, state: AppState) {
    let status = status_handle();
    tokio::spawn(async move {
        set_status(&status, MqttBridgeStatus::from_config(&config)).await;
        if !config.enabled {
            tracing::info!("MQTT bridge disabled");
            return;
        }
        if let Err(err) = run_mqtt_bridge(config, state, status.clone()).await {
            update_status(&status, |snapshot| {
                snapshot.connected = false;
                snapshot.last_error = Some(err.to_string());
            })
            .await;
            tracing::warn!("MQTT bridge stopped: {err}");
        }
    });
}

async fn run_mqtt_bridge(
    config: MqttBridgeConfig,
    state: AppState,
    status: SharedMqttStatus,
) -> Result<()> {
    let mut options = MqttOptions::new(config.client_id.clone(), config.host.clone(), config.port);
    options.set_keep_alive(Duration::from_secs(config.keep_alive_s.max(1)));
    if let Some(username) = config.username.as_deref() {
        options.set_credentials(username, config.password.as_deref().unwrap_or_default());
    }
    if config.use_tls {
        options.set_transport(mqtt_tls_transport(&config)?);
    }

    let (client, mut eventloop) = AsyncClient::new(options, config.queue_capacity.max(1));
    client
        .subscribe(config.task_topic.clone(), QoS::AtLeastOnce)
        .await
        .with_context(|| format!("failed to subscribe MQTT task topic {}", config.task_topic))?;
    publish_status(&client, &config, "online").await?;
    let alert = publish_alert_snapshot(&client, &config, &state).await?;
    update_status(&status, |snapshot| {
        snapshot.connected = true;
        snapshot.last_error = None;
        snapshot.last_alert_active_count = alert.active_count;
        snapshot.last_alert_at = Some(alert.updated_at);
    })
    .await;

    let mut alert_interval = interval(Duration::from_secs(config.alert_interval_s.max(1)));
    alert_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    alert_interval.tick().await;

    loop {
        tokio::select! {
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::Publish(publish))) => {
                        if publish.topic == config.task_topic {
                            let receipt = execute_mqtt_task_payload(&state, &publish.payload).await;
                            update_status(&status, |snapshot| {
                                snapshot.last_task_id = receipt.task_id;
                                snapshot.last_error = receipt.error.clone();
                            })
                            .await;
                            client
                                .publish(
                                    config.receipt_topic.clone(),
                                    QoS::AtLeastOnce,
                                    false,
                                    serde_json::to_vec(&receipt)?,
                                )
                                .await?;
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        update_status(&status, |snapshot| {
                            snapshot.connected = false;
                            snapshot.last_error = Some(err.to_string());
                        })
                        .await;
                        return Err(err.into());
                    }
                }
            }
            _ = alert_interval.tick() => {
                let alert = publish_alert_snapshot(&client, &config, &state).await?;
                update_status(&status, |snapshot| {
                    snapshot.last_alert_active_count = alert.active_count;
                    snapshot.last_alert_at = Some(alert.updated_at);
                    snapshot.last_error = None;
                })
                .await;
            }
        }
    }
}

fn mqtt_tls_transport(config: &MqttBridgeConfig) -> Result<Transport> {
    if config.ca_cert.is_none() && config.client_cert.is_none() && config.client_key.is_none() {
        return Ok(Transport::tls_with_default_config());
    }
    crate::tls::install_rustls_provider();
    if let Some(ca_cert) = config.ca_cert.as_ref() {
        let ca_bytes = fs::read(ca_cert)
            .with_context(|| format!("failed to read MQTT CA certificate {}", ca_cert.display()))?;
        let _ = crate::tls::load_cert_chain(ca_cert)?;
        let tls_config = rumqttc::TlsConfiguration::Simple {
            ca: ca_bytes,
            alpn: None,
            client_auth: match crate::tls::paired_paths(
                &config.client_cert,
                &config.client_key,
                "MQTT TLS client",
            )? {
                Some((cert, key)) => Some((
                    fs::read(&cert).with_context(|| {
                        format!("failed to read MQTT client certificate {}", cert.display())
                    })?,
                    fs::read(&key).with_context(|| {
                        format!("failed to read MQTT client key {}", key.display())
                    })?,
                )),
                None => None,
            },
        };
        return Ok(Transport::tls_with_config(tls_config));
    }

    let _ = crate::tls::paired_paths(&config.client_cert, &config.client_key, "MQTT TLS client")?;
    anyhow::bail!(
        "MQTT TLS client certificate requires ca_cert so the broker certificate can be verified"
    )
}

async fn publish_status(
    client: &AsyncClient,
    config: &MqttBridgeConfig,
    status: &str,
) -> Result<()> {
    client
        .publish(
            config.status_topic.clone(),
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&json!({
                "device_id": "reactor_001",
                "status": status,
                "client_id": config.client_id,
                "task_topic": config.task_topic,
                "receipt_topic": config.receipt_topic,
                "updated_at": Utc::now()
            }))?,
        )
        .await?;
    Ok(())
}

async fn publish_alert_snapshot(
    client: &AsyncClient,
    config: &MqttBridgeConfig,
    state: &AppState,
) -> Result<MqttAlertSnapshot> {
    let snapshot = mqtt_alert_snapshot(state).await;
    client
        .publish(
            config.alert_topic.clone(),
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&snapshot)?,
        )
        .await?;
    Ok(snapshot)
}

pub async fn mqtt_alert_snapshot(state: &AppState) -> MqttAlertSnapshot {
    let runtime = state.runtime.read().await;
    let sample = runtime.latest_sample.as_ref();
    let sensor_fresh = sample
        .map(|sample| {
            Utc::now().signed_duration_since(sample.captured_at)
                <= chrono::Duration::milliseconds(state.safety.control.sensor_timeout_ms)
        })
        .unwrap_or(false);
    let alarms = alarms_for(&runtime, sample, state.ai_memory.as_ref());
    let high_count = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("high"))
        .count();
    let warning_count = alarms
        .iter()
        .filter(|alarm| alarm.get("level").and_then(Value::as_str) == Some("medium"))
        .count();

    MqttAlertSnapshot {
        device_id: "reactor_001",
        active: !alarms.is_empty(),
        active_count: alarms.len(),
        high_count,
        warning_count,
        emergency_stop: runtime.emergency_stop,
        manual_lock: runtime.manual_lock,
        sensor_fresh,
        active_batch_id: runtime.active_batch_id,
        alarms,
        updated_at: Utc::now(),
    }
}

pub async fn execute_mqtt_task_payload(state: &AppState, payload: &[u8]) -> MqttTaskReceipt {
    let request = match serde_json::from_slice::<AinasTaskRequest>(payload) {
        Ok(request) => request,
        Err(err) => {
            return MqttTaskReceipt {
                ok: false,
                source: "mqtt",
                task_id: None,
                external_task_id: None,
                action: None,
                status: "rejected".to_string(),
                response: None,
                error: Some(format!("invalid MQTT task JSON: {err}")),
            }
        }
    };
    receipt_from_result(execute_integration_task(state, "mqtt", request).await)
}

fn receipt_from_result(result: Result<IntegrationTask, AppError>) -> MqttTaskReceipt {
    match result {
        Ok(task) => MqttTaskReceipt {
            ok: task.status == "executed",
            source: "mqtt",
            task_id: Some(task.id),
            external_task_id: task.external_task_id,
            action: Some(task.action),
            status: task.status,
            response: Some(task.response),
            error: None,
        },
        Err(err) => MqttTaskReceipt {
            ok: false,
            source: "mqtt",
            task_id: None,
            external_task_id: None,
            action: None,
            status: if err.status_code().is_server_error() {
                "failed".to_string()
            } else {
                "rejected".to_string()
            },
            response: None,
            error: Some(err.message().to_string()),
        },
    }
}

async fn set_status(status: &SharedMqttStatus, next: MqttBridgeStatus) {
    *status.write().await = next;
}

async fn update_status(status: &SharedMqttStatus, update: impl FnOnce(&mut MqttBridgeStatus)) {
    let mut snapshot = status.write().await;
    update(&mut snapshot);
    snapshot.updated_at = Utc::now();
}
