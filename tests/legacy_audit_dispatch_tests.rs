use std::time::Duration;
use reactor_edge_daemon::{
    config::load_safety_config,
    control::SafeCommand,
    device::{write_targets_with_ack_deadline, AckStatus, CommandAck, ReactorDevice},
    state::SensorSnapshot,
};

struct FakeDevice { mode: &'static str }
#[async_trait::async_trait]
impl ReactorDevice for FakeDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> { std::future::pending().await }
    async fn write_targets(&self, _: &SafeCommand) -> anyhow::Result<()> { Ok(()) }
    async fn write_targets_acknowledged(&self, _: &SafeCommand, rid: &str, _: Duration) -> anyhow::Result<CommandAck> {
        match self.mode {
            "hang" => std::future::pending().await,
            "fail" => Err(anyhow::anyhow!("transport failed")),
            _ => Ok(CommandAck {
                request_id: if self.mode == "mismatch" { "old-request" } else { rid }.to_string(),
                status: AckStatus::Confirmed, accepted_targets: None,
            }),
        }
    }
}
fn command() -> SafeCommand {
    let safety = load_safety_config("config/safety.toml").unwrap();
    SafeCommand {
        target_temperature_c: safety.temperature.min_c, target_stirrer_rpm: 0.0,
        target_shake_speed_cpm: 0.0, target_pressure_mpa: 0.0,
        heat_time_s: 0.0, hold_time_s: 0.0, cool_time_s: 0.0, reason: "audit".to_string(),
    }
}
#[tokio::test]
async fn legacy_audit_adapter_cannot_ignore_ack_deadline() {
    let result = tokio::time::timeout(Duration::from_secs(1), write_targets_with_ack_deadline(
        &FakeDevice { mode: "hang" }, &command(), "new-request", Duration::from_millis(10),
    )).await.expect("supervisor must not hang");
    assert!(result.unwrap_err().to_string().contains("delivery unknown"));
}
#[tokio::test]
async fn legacy_audit_stale_ack_is_not_success() {
    let result = write_targets_with_ack_deadline(&FakeDevice { mode: "mismatch" }, &command(), "new-request", Duration::from_secs(1)).await;
    assert!(result.unwrap_err().to_string().contains("request_id mismatch"));
}
#[tokio::test]
async fn legacy_audit_matching_ack_remains_success() {
    let ack = write_targets_with_ack_deadline(&FakeDevice { mode: "ok" }, &command(), "new-request", Duration::from_secs(1)).await.unwrap();
    assert!(matches!(ack.status, AckStatus::Confirmed));
}
#[tokio::test]
async fn legacy_audit_ack_transport_error_is_preserved() {
    let result = write_targets_with_ack_deadline(&FakeDevice { mode: "fail" }, &command(), "new-request", Duration::from_secs(1)).await;
    assert!(result.unwrap_err().to_string().contains("transport failed"));
}
