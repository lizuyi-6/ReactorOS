use reactor_edge_daemon::{
    control::SafeCommand,
    device::{build_esp32_command, build_esp32_sample_frame, checksum_hex, parse_esp32_frame},
    state::SensorSnapshot,
};

use chrono::Utc;

#[test]
fn parses_valid_esp32_rx_frame_with_checksum() {
    let body = "RX|v=1|seq=123|ms=456789|temp=175.4|pressure=0.21|stir_speed=450|shake_speed=30|tilt_state=1|flow_rate=2.5|product_concentration=62.4|ph=7.18";
    let frame = format!("{body}|chk={}", checksum_hex(body.as_bytes()));

    let sample = parse_esp32_frame(&frame, "RX", true).unwrap();

    assert_eq!(sample.temperature_c, 175.4);
    assert_eq!(sample.pressure_mpa, 0.21);
    assert_eq!(sample.stirrer_rpm, 450.0);
    assert_eq!(sample.shake_speed_cpm, 30.0);
    assert_eq!(sample.tilt_state, 1);
    assert!(sample.tilt_angle_deg >= 0.0);
    assert_eq!(sample.flow_rate_l_min, 2.5);
    assert_eq!(sample.product_concentration_percent, 62.4);
    assert_eq!(sample.ph, 7.18);
}

#[test]
fn rejects_esp32_frame_with_bad_checksum() {
    let frame = "RX|v=1|seq=123|ms=456789|temp=175.4|pressure=0.21|stir_speed=450|shake_speed=30|tilt_state=1|flow_rate=2.5|chk=00";

    let err = parse_esp32_frame(frame, "RX", true)
        .unwrap_err()
        .to_string();

    assert!(err.contains("checksum mismatch"));
}

#[test]
fn rejects_esp32_frame_missing_required_sensor() {
    let body = "RX|v=1|seq=123|ms=456789|temp=175.4|pressure=0.21|stir_speed=450|shake_speed=30|flow_rate=2.5";
    let frame = format!("{body}|chk={}", checksum_hex(body.as_bytes()));

    let err = parse_esp32_frame(&frame, "RX", true)
        .unwrap_err()
        .to_string();

    assert!(err.contains("missing field tilt_state"));
}

#[test]
fn parses_minimum_document_rx_frame_without_optional_lab_fields() {
    let body = "RX|v=1|seq=124|ms=456999|temp=176.1|pressure=0.22|stir_speed=455|shake_speed=31|tilt_state=0|flow_rate=2.6";
    let frame = format!("{body}|chk={}", checksum_hex(body.as_bytes()));

    let sample = parse_esp32_frame(&frame, "RX", true).unwrap();

    assert_eq!(sample.temperature_c, 176.1);
    assert_eq!(sample.pressure_mpa, 0.22);
    assert_eq!(sample.stirrer_rpm, 455.0);
    assert_eq!(sample.shake_speed_cpm, 31.0);
    assert_eq!(sample.tilt_state, 0);
    assert!(sample.tilt_angle_deg <= 0.0);
    assert_eq!(sample.flow_rate_l_min, 2.6);
    assert_eq!(sample.product_concentration_percent, 0.0);
    assert_eq!(sample.ph, 7.0);
}

#[test]
fn keeps_legacy_esp32_rx_aliases_for_existing_benches() {
    let body = "RX|v=1|seq=125|ms=457111|temp=175.4|pressure=0.21|rpm=450|shake=30|tilt=1|flow=2.5|conc=62.4|ph=7.18";
    let frame = format!("{body}|chk={}", checksum_hex(body.as_bytes()));

    let sample = parse_esp32_frame(&frame, "RX", true).unwrap();

    assert_eq!(sample.stirrer_rpm, 450.0);
    assert_eq!(sample.shake_speed_cpm, 30.0);
    assert_eq!(sample.tilt_state, 1);
    assert_eq!(sample.flow_rate_l_min, 2.5);
    assert_eq!(sample.product_concentration_percent, 62.4);
    assert_eq!(sample.ph, 7.18);
}

#[test]
fn builds_tx_command_with_checksum() {
    let command = SafeCommand {
        target_temperature_c: 192.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        target_stirrer_rpm: 520.0,
        target_shake_speed_cpm: 35.0,
        target_pressure_mpa: 0.5,
        reason: "test".to_string(),
    };

    let frame = build_esp32_command("TX", &command, true);
    let body = "TX|v=1|heat_time=300.00|hold_time=600.00|cool_time=180.00|target_temp=192.00|stir_speed=520.00|shake_speed=35.00|target_pressure=0.50";
    let expected = format!("{body}|chk={}\n", checksum_hex(body.as_bytes()));

    assert_eq!(frame, expected);
}

#[test]
fn simulated_sample_frame_round_trips_through_esp32_parser() {
    let sample = SensorSnapshot {
        temperature_c: 64.25,
        pressure_mpa: 0.0955,
        stirrer_rpm: 320.0,
        shake_speed_cpm: 30.0,
        tilt_state: 1,
        tilt_angle_deg: 12.5,
        flow_rate_l_min: 2.5,
        product_concentration_percent: 18.75,
        ph: 7.21,
        captured_at: Utc::now(),
    };

    let frame = build_esp32_sample_frame("RX", &sample, true);
    let parsed = parse_esp32_frame(&frame, "RX", true).unwrap();

    assert_eq!(parsed.temperature_c, sample.temperature_c);
    assert_eq!(parsed.pressure_mpa, 0.1);
    assert_eq!(parsed.stirrer_rpm, sample.stirrer_rpm);
    assert_eq!(parsed.shake_speed_cpm, sample.shake_speed_cpm);
    assert_eq!(parsed.tilt_state, sample.tilt_state);
    assert!(parsed.tilt_angle_deg >= 0.0);
    assert_eq!(parsed.flow_rate_l_min, sample.flow_rate_l_min);
    assert_eq!(
        parsed.product_concentration_percent,
        sample.product_concentration_percent
    );
    assert_eq!(parsed.ph, sample.ph);
}
