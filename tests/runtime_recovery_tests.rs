use reactor_edge_daemon::{
    config::load_safety_config, db::Db, runtime_recovery::recover_runtime_from_db,
};

#[tokio::test]
async fn startup_recovers_unfinished_batch_as_fail_closed_active_state() {
    let safety = load_safety_config("config/safety.toml").unwrap();
    let db = Db::open_memory().unwrap();
    let older = db
        .create_batch_for_process_sqlx(None, "older interrupted batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let latest = db
        .create_batch_for_process_sqlx(None, "latest interrupted batch", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(older.id).await.unwrap();

    let runtime = recover_runtime_from_db(&db, &safety).await.unwrap();

    assert_eq!(runtime.active_batch_id, Some(latest.id));
    assert!(!runtime.auto_enabled);
    assert!(runtime
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("daemon restarted with unfinished batch"));
}

#[tokio::test]
async fn startup_surfaces_all_unfinished_batch_ids_in_latched_fault() {
    let safety = load_safety_config("config/safety.toml").unwrap();
    let db = Db::open_memory().unwrap();
    let older = db
        .create_batch_for_process_sqlx(None, "older orphan", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let newer = db
        .create_batch_for_process_sqlx(None, "newer orphan", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();

    let runtime = recover_runtime_from_db(&db, &safety).await.unwrap();

    assert_eq!(runtime.active_batch_id, Some(newer.id));
    let fault = runtime.last_control_error.as_deref().unwrap_or_default();
    assert!(fault.contains(&format!("unfinished batch {}", newer.id)));
    assert!(fault.contains(&older.id.to_string()));
    assert!(fault.contains(&newer.id.to_string()));
}

#[tokio::test]
async fn startup_without_unfinished_batch_stays_idle_and_unlatched() {
    let safety = load_safety_config("config/safety.toml").unwrap();
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch_for_process_sqlx(None, "finished batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(batch.id).await.unwrap();

    let runtime = recover_runtime_from_db(&db, &safety).await.unwrap();

    assert_eq!(runtime.active_batch_id, None);
    assert!(!runtime.auto_enabled);
    assert!(runtime.last_control_error.is_none());
}
