use reactor_edge_daemon::{
    config::OptimizerBounds,
    db::BatchOutcome,
    memory::{
        AiMemory, ForbiddenZone, MemoryOptimizerBounds, RecommendationMemory, ReferenceBatch,
    },
    optimizer::{recommend, recommend_with_memory},
};

fn bounds() -> OptimizerBounds {
    OptimizerBounds {
        min_temperature_c: 35.0,
        max_temperature_c: 140.0,
        min_stirrer_rpm: 100.0,
        max_stirrer_rpm: 1000.0,
        min_heating_minutes: 15.0,
        max_heating_minutes: 240.0,
        min_stirring_minutes: 15.0,
        max_stirring_minutes: 240.0,
    }
}

#[test]
fn recommendation_uses_midpoint_until_enough_batches_exist() {
    let rec = recommend(&bounds(), &[]);

    assert_eq!(rec.target_temperature_c, 87.5);
    assert_eq!(rec.target_stirrer_rpm, 550.0);
    assert_eq!(rec.based_on_batch_count, 0);
}

#[test]
fn recommendation_stays_inside_configured_bounds() {
    let outcomes = vec![
        outcome(1, 50.0, 300.0, 60.0, 60.0, 60.0, 0.6),
        outcome(2, 80.0, 500.0, 80.0, 80.0, 85.0, 0.9),
        outcome(3, 120.0, 900.0, 120.0, 100.0, 70.0, 0.7),
        outcome(4, 78.0, 520.0, 90.0, 85.0, 88.0, 0.91),
    ];

    for _ in 0..50 {
        let rec = recommend(&bounds(), &outcomes);
        assert!((35.0..=140.0).contains(&rec.target_temperature_c));
        assert!((100.0..=1000.0).contains(&rec.target_stirrer_rpm));
        assert!((15.0..=240.0).contains(&rec.heating_minutes));
        assert!((15.0..=240.0).contains(&rec.stirring_minutes));
        assert_two_decimal_parameters([
            rec.target_temperature_c,
            rec.target_stirrer_rpm,
            rec.heating_minutes,
            rec.stirring_minutes,
            rec.expected_score,
        ]);
        assert_eq!(rec.based_on_batch_count, 4);
    }
}

#[test]
fn file_memory_reference_batches_can_seed_recommendations_before_real_history() {
    let memory = AiMemory {
        recommendation: RecommendationMemory {
            enabled: true,
            use_reference_batches: true,
            bounds: MemoryOptimizerBounds {
                min_temperature_c: Some(70.0),
                max_temperature_c: Some(100.0),
                min_stirrer_rpm: Some(400.0),
                max_stirrer_rpm: Some(700.0),
                min_heating_minutes: Some(50.0),
                max_heating_minutes: Some(130.0),
                min_stirring_minutes: Some(40.0),
                max_stirring_minutes: Some(120.0),
            },
        },
        reference_batches: vec![
            reference("a", 72.0, 420.0, 80.0, 60.0, 70.0, 0.70),
            reference("b", 92.0, 560.0, 95.0, 70.0, 88.0, 0.91),
            reference("c", 98.0, 620.0, 105.0, 80.0, 82.0, 0.84),
        ],
        ..AiMemory::default()
    };

    for _ in 0..50 {
        let rec = recommend_with_memory(&bounds(), Some(&memory), &[]);

        assert!((70.0..=100.0).contains(&rec.target_temperature_c));
        assert!((400.0..=700.0).contains(&rec.target_stirrer_rpm));
        assert!((50.0..=130.0).contains(&rec.heating_minutes));
        assert!((40.0..=120.0).contains(&rec.stirring_minutes));
        assert_eq!(rec.based_on_batch_count, 0);
        assert!(rec.rationale.contains("file reference"));
    }
}

#[test]
fn file_memory_forbidden_zones_are_avoided() {
    let memory = AiMemory {
        recommendation: RecommendationMemory {
            enabled: true,
            use_reference_batches: true,
            bounds: MemoryOptimizerBounds {
                min_temperature_c: Some(80.0),
                max_temperature_c: Some(120.0),
                min_stirrer_rpm: Some(300.0),
                max_stirrer_rpm: Some(600.0),
                min_heating_minutes: Some(60.0),
                max_heating_minutes: Some(120.0),
                min_stirring_minutes: Some(50.0),
                max_stirring_minutes: Some(100.0),
            },
        },
        reference_batches: vec![
            reference("a", 100.0, 450.0, 90.0, 70.0, 90.0, 0.92),
            reference("b", 102.0, 455.0, 92.0, 72.0, 89.0, 0.90),
            reference("c", 98.0, 440.0, 88.0, 69.0, 88.0, 0.89),
        ],
        forbidden_zones: vec![ForbiddenZone {
            name: "avoid-best-cluster".to_string(),
            reason: "bench operator flagged this parameter island".to_string(),
            min_temperature_c: Some(95.0),
            max_temperature_c: Some(105.0),
            min_stirrer_rpm: Some(400.0),
            max_stirrer_rpm: Some(500.0),
            min_heating_minutes: None,
            max_heating_minutes: None,
            min_stirring_minutes: None,
            max_stirring_minutes: None,
        }],
        ..AiMemory::default()
    };

    let rec = recommend_with_memory(&bounds(), Some(&memory), &[]);

    assert!(
        !memory.forbidden_zones[0].contains(
            rec.target_temperature_c,
            rec.target_stirrer_rpm,
            rec.heating_minutes,
            rec.stirring_minutes,
        ),
        "recommendation landed in forbidden zone: {rec:?}"
    );
}

fn outcome(
    batch_id: i64,
    target_temperature_c: f64,
    target_stirrer_rpm: f64,
    heating_minutes: f64,
    stirring_minutes: f64,
    yield_percent: f64,
    product_ratio: f64,
) -> BatchOutcome {
    BatchOutcome {
        batch_id,
        target_temperature_c,
        target_stirrer_rpm,
        heating_minutes,
        stirring_minutes,
        yield_percent,
        product_ratio,
    }
}

fn reference(
    id: &str,
    target_temperature_c: f64,
    target_stirrer_rpm: f64,
    heating_minutes: f64,
    stirring_minutes: f64,
    yield_percent: f64,
    product_ratio: f64,
) -> ReferenceBatch {
    ReferenceBatch {
        id: id.to_string(),
        target_temperature_c,
        target_stirrer_rpm,
        heating_minutes,
        stirring_minutes,
        yield_percent,
        product_ratio,
        notes: String::new(),
    }
}

fn assert_two_decimal_parameters(values: impl IntoIterator<Item = f64>) {
    for value in values {
        let scaled = value * 100.0;
        assert!(
            (scaled - scaled.round()).abs() < 1e-9,
            "parameter should be rounded to two decimals: {value}"
        );
    }
}
