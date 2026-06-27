use anyhow::Result;

use crate::{config::SafetyConfig, db::Db, state::RuntimeState};

pub async fn recover_runtime_from_db(db: &Db, safety: &SafetyConfig) -> Result<RuntimeState> {
    let mut runtime = RuntimeState::from_safety(safety);
    let unfinished = db.unfinished_batches_sqlx(100).await?;
    if let Some(batch) = unfinished.first() {
        let ids = unfinished
            .iter()
            .map(|batch| batch.id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        runtime.active_batch_id = Some(batch.id);
        runtime.auto_enabled = false;
        runtime.latch_control_fault(format!(
            "daemon restarted with unfinished batch {}; unfinished batch ids [{}] require field verification and stop/finish repair before production control resumes",
            batch.id, ids
        ));
    }
    Ok(runtime)
}
