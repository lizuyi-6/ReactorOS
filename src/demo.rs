use anyhow::Result;

use crate::{
    config::SafetyConfig,
    db::{AuditActor, Db, NewProcessStep, ProductResult},
    memory::AiMemory,
    optimizer::recommend_with_memory,
};

pub fn seed_demo_context(db: &Db, safety: &SafetyConfig, memory: &AiMemory) -> Result<bool> {
    if db.demo_seed_exists()? {
        return Ok(false);
    }

    let process = db.create_process(
        "客户演示工艺 A - 温和优化",
        "DEMO: 仅用于客户演示的工艺定义；实时传感器数据仍必须来自下游管线。",
    )?;
    for step in [
        NewProcessStep {
            name: "升温".to_string(),
            target_temperature_c: 78.0,
            ramp_rate_c_min: 2.5,
            duration_minutes: 42.0,
            target_stirrer_rpm: 360.0,
            target_shake_speed_cpm: 22.0,
            target_pressure_mpa: 0.38,
            cooling_mode: "自然".to_string(),
        },
        NewProcessStep {
            name: "保温反应".to_string(),
            target_temperature_c: 96.0,
            ramp_rate_c_min: 0.0,
            duration_minutes: 88.0,
            target_stirrer_rpm: 540.0,
            target_shake_speed_cpm: 32.0,
            target_pressure_mpa: 0.42,
            cooling_mode: "自然".to_string(),
        },
        NewProcessStep {
            name: "降温取样".to_string(),
            target_temperature_c: 58.0,
            ramp_rate_c_min: -3.0,
            duration_minutes: 35.0,
            target_stirrer_rpm: 280.0,
            target_shake_speed_cpm: 12.0,
            target_pressure_mpa: 0.30,
            cooling_mode: "风冷".to_string(),
        },
    ] {
        db.add_process_step(process.id, &step)?;
    }

    let comparison = db.create_process(
        "客户演示工艺 B - 高搅拌对照",
        "DEMO: 安全阈值内但搅拌偏强，用于展示 AI 如何识别产率下降趋势。",
    )?;
    for step in [
        NewProcessStep {
            name: "快速升温".to_string(),
            target_temperature_c: 112.0,
            ramp_rate_c_min: 3.5,
            duration_minutes: 58.0,
            target_stirrer_rpm: 760.0,
            target_shake_speed_cpm: 40.0,
            target_pressure_mpa: 0.50,
            cooling_mode: "自然".to_string(),
        },
        NewProcessStep {
            name: "长时保温".to_string(),
            target_temperature_c: 118.0,
            ramp_rate_c_min: 0.0,
            duration_minutes: 125.0,
            target_stirrer_rpm: 820.0,
            target_shake_speed_cpm: 44.0,
            target_pressure_mpa: 0.56,
            cooling_mode: "水冷".to_string(),
        },
    ] {
        db.add_process_step(comparison.id, &step)?;
    }

    let demo_batches = [
        (
            Some(process.id),
            "DEMO-Batch-037 低温短时",
            72.0,
            420.0,
            70.0,
            58.0,
            61.2,
            0.66,
        ),
        (
            Some(process.id),
            "DEMO-Batch-038 搅拌不足",
            84.0,
            360.0,
            82.0,
            66.0,
            64.7,
            0.70,
        ),
        (
            Some(process.id),
            "DEMO-Batch-039 基准参数",
            92.0,
            520.0,
            95.0,
            72.0,
            74.8,
            0.81,
        ),
        (
            Some(comparison.id),
            "DEMO-Batch-040 高搅拌对照",
            118.0,
            820.0,
            125.0,
            95.0,
            67.5,
            0.74,
        ),
        (
            Some(process.id),
            "DEMO-Batch-041 温和优化",
            96.0,
            560.0,
            92.0,
            78.0,
            82.4,
            0.88,
        ),
        (
            Some(process.id),
            "DEMO-Batch-042 延长保温",
            98.0,
            580.0,
            120.0,
            110.0,
            78.1,
            0.84,
        ),
    ];

    for (process_id, name, temp, rpm, heating, stirring, yield_percent, ratio) in demo_batches {
        let batch = db.create_batch_for_process(process_id, name, temp, rpm, heating, stirring)?;
        db.finish_batch(batch.id)?;
        db.insert_product_result(&ProductResult {
            batch_id: batch.id,
            yield_percent,
            product_ratio: ratio,
            notes: "DEMO: 历史批次结果，用于演示 AI 学习；不是实时传感器数据。".to_string(),
        })?;
    }

    let outcomes = db.batch_outcomes()?;
    let mut recommendation = recommend_with_memory(&safety.optimizer, Some(memory), &outcomes);
    recommendation.rationale = format!(
        "DEMO: AI 识别到高搅拌/长保温在安全范围内但产率下降；建议靠近温和优化区域。{}",
        recommendation.rationale
    );
    db.insert_recommendation(&recommendation)?;

    db.insert_demo_alarm(
        "demo_quality_warning",
        "AI Learning",
        "medium",
        "DEMO: Batch 040 参数在安全阈值内，但高搅拌和长保温组合使产率下降。",
        Some(67.5),
        Some(74.0),
        "建议下批降低搅拌转速并缩短保温时间，由 AI 推荐值复核。",
    )?;
    db.insert_demo_alarm(
        "demo_process_notice",
        "Process",
        "low",
        "DEMO: 已加载两套演示工艺和六条历史批次结果，实时传感器仍等待管线数据。",
        None,
        None,
        "连接 state.json 或 ESP32 后，传感器区域会自动切换为真实读数。",
    )?;

    db.insert_control_event(
        None,
        "demo_seed_applied",
        None,
        "DEMO context seeded: processes, historical batch outcomes, AI recommendation, and non-sensor demo alarms only",
        &AuditActor::system(),
    )?;

    Ok(true)
}
