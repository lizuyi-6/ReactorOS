use chrono::Utc;
use reactor_edge_daemon::{
    db::{Db, ProductResult},
    optimizer::recommend,
    state::SensorSnapshot,
};

#[test]
fn batch_result_and_recommendation_round_trip() {
    let db = Db::open_memory().unwrap();
    let batch = db.create_batch("test", 70.0, 400.0, 45.0, 60.0).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: batch.id,
        yield_percent: 72.5,
        product_ratio: 0.82,
        notes: "ok".to_string(),
    })
    .unwrap();

    let outcomes = db.batch_outcomes().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].batch_id, batch.id);

    let rec = recommend(
        &reactor_edge_daemon::config::OptimizerBounds {
            min_temperature_c: 35.0,
            max_temperature_c: 140.0,
            min_stirrer_rpm: 100.0,
            max_stirrer_rpm: 1000.0,
            min_heating_minutes: 15.0,
            max_heating_minutes: 240.0,
            min_stirring_minutes: 15.0,
            max_stirring_minutes: 240.0,
        },
        &outcomes,
    );
    db.insert_recommendation(&rec).unwrap();

    let latest = db.latest_recommendation().unwrap().unwrap();
    assert_eq!(latest.based_on_batch_count, 1);
}

#[test]
fn persists_extended_sensor_sample() {
    let db = Db::open_memory().unwrap();
    let batch = db.create_batch("esp32", 70.0, 400.0, 45.0, 60.0).unwrap();
    db.insert_sample(
        Some(batch.id),
        &SensorSnapshot {
            temperature_c: 175.4,
            pressure_mpa: 0.2102,
            stirrer_rpm: 450.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.5,
            product_concentration_percent: 62.4,
            ph: 7.18,
            captured_at: Utc::now(),
        },
    )
    .unwrap();

    let sample = db.recent_samples(1).unwrap().pop().unwrap();
    assert_eq!(sample.shake_speed_cpm, 30.0);
    assert_eq!(sample.tilt_state, 1);
    assert_eq!(sample.tilt_angle_deg, 12.5);
}
