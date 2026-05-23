use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::{
    config::OptimizerBounds,
    db::BatchOutcome,
    memory::{AiMemory, ForbiddenZone, ReferenceBatch},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub based_on_batch_count: i64,
    pub target_temperature_c: f64,
    pub target_stirrer_rpm: f64,
    pub heating_minutes: f64,
    pub stirring_minutes: f64,
    pub expected_score: f64,
    pub rationale: String,
}

pub fn recommend(bounds: &OptimizerBounds, outcomes: &[BatchOutcome]) -> Recommendation {
    recommend_with_memory(bounds, None, outcomes)
}

pub fn recommend_with_memory(
    bounds: &OptimizerBounds,
    memory: Option<&AiMemory>,
    outcomes: &[BatchOutcome],
) -> Recommendation {
    let effective_bounds = memory
        .map(|memory| memory.effective_optimizer_bounds(bounds))
        .unwrap_or_else(|| bounds.clone());
    let objective_weights = memory
        .map(|memory| memory.objective.weights())
        .unwrap_or((0.8, 0.2));
    let mut memory_outcomes = Vec::new();
    if let Some(memory) = memory {
        if memory.recommendation.enabled && memory.recommendation.use_reference_batches {
            memory_outcomes = memory
                .reference_batches
                .iter()
                .map(reference_to_outcome)
                .collect();
        }
    }

    let mut combined = memory_outcomes.clone();
    combined.extend_from_slice(outcomes);
    if combined.len() < 3 {
        return midpoint_recommendation(
            &effective_bounds,
            outcomes.len() as i64,
            memory,
            memory_outcomes.len(),
        );
    }

    let forbidden_zones: &[ForbiddenZone] = memory
        .filter(|memory| memory.recommendation.enabled)
        .map(|memory| memory.forbidden_zones.as_slice())
        .unwrap_or(&[]);

    let mut sorted = combined;
    sorted.sort_by(|a, b| {
        score_with_weights(b, objective_weights)
            .total_cmp(&score_with_weights(a, objective_weights))
    });
    let elite_count = ((sorted.len() as f64) * 0.25).ceil().max(1.0) as usize;
    let elites = &sorted[..elite_count.min(sorted.len())];
    let best = &elites[0];

    let mut rng = rand::thread_rng();
    let candidate = sample_allowed_candidate(&mut rng, elites, &effective_bounds, forbidden_zones);
    let (temp, rpm, heating, stirring, forbidden_note) = match candidate {
        Some(candidate) => (
            candidate.temperature_c,
            candidate.stirrer_rpm,
            candidate.heating_minutes,
            candidate.stirring_minutes,
            "",
        ),
        None => {
            let fallback = safe_midpoint(&effective_bounds, forbidden_zones);
            (
                fallback.temperature_c,
                fallback.stirrer_rpm,
                fallback.heating_minutes,
                fallback.stirring_minutes,
                " All sampled candidates matched a forbidden zone; using the nearest safe midpoint.",
            )
        }
    };

    Recommendation {
        based_on_batch_count: outcomes.len() as i64,
        target_temperature_c: round2(temp),
        target_stirrer_rpm: round2(rpm),
        heating_minutes: round2(heating),
        stirring_minutes: round2(stirring),
        expected_score: round2(score_with_weights(best, objective_weights)),
        rationale: rationale(
            best,
            elite_count,
            outcomes.len(),
            memory_outcomes.len(),
            memory,
            forbidden_note,
        ),
    }
}

fn midpoint_recommendation(
    bounds: &OptimizerBounds,
    count: i64,
    memory: Option<&AiMemory>,
    memory_outcome_count: usize,
) -> Recommendation {
    let reference_note = if memory_outcome_count > 0 {
        format!(" Loaded {memory_outcome_count} file reference batches for context.")
    } else {
        String::new()
    };
    let profile_note = memory
        .filter(|memory| memory.recommendation.enabled)
        .map(|memory| {
            format!(
                " Memory profile: {} v{}.",
                memory.profile.name, memory.profile.version
            )
        })
        .unwrap_or_default();
    Recommendation {
        based_on_batch_count: count,
        target_temperature_c: round2((bounds.min_temperature_c + bounds.max_temperature_c) / 2.0),
        target_stirrer_rpm: round2((bounds.min_stirrer_rpm + bounds.max_stirrer_rpm) / 2.0),
        heating_minutes: round2((bounds.min_heating_minutes + bounds.max_heating_minutes) / 2.0),
        stirring_minutes: round2((bounds.min_stirring_minutes + bounds.max_stirring_minutes) / 2.0),
        expected_score: 0.0,
        rationale: format!(
            "Not enough finished batches yet; using the center of configured safe optimizer bounds.{profile_note}{reference_note}"
        ),
    }
}

fn score_with_weights(outcome: &BatchOutcome, weights: (f64, f64)) -> f64 {
    let (yield_weight, ratio_weight) = weights;
    outcome.yield_percent * yield_weight + outcome.product_ratio * 100.0 * ratio_weight
}

fn mean(items: &[BatchOutcome], f: impl Fn(&BatchOutcome) -> f64) -> f64 {
    items.iter().map(f).sum::<f64>() / items.len() as f64
}

fn spread(items: &[BatchOutcome], f: impl Fn(&BatchOutcome) -> f64 + Copy, floor: f64) -> f64 {
    let avg = mean(items, f);
    let variance = items
        .iter()
        .map(|item| {
            let delta = f(item) - avg;
            delta * delta
        })
        .sum::<f64>()
        / items.len() as f64;
    variance.sqrt().max(floor)
}

fn sample_around(rng: &mut impl Rng, center: f64, spread: f64, min: f64, max: f64) -> f64 {
    let jitter = rng.gen_range(-spread..=spread);
    (center + jitter).clamp(min, max)
}

#[derive(Debug, Clone)]
struct Candidate {
    temperature_c: f64,
    stirrer_rpm: f64,
    heating_minutes: f64,
    stirring_minutes: f64,
}

fn sample_allowed_candidate(
    rng: &mut impl Rng,
    elites: &[BatchOutcome],
    bounds: &OptimizerBounds,
    forbidden_zones: &[ForbiddenZone],
) -> Option<Candidate> {
    for _ in 0..64 {
        let candidate = Candidate {
            temperature_c: sample_around(
                rng,
                mean(elites, |x| x.target_temperature_c),
                spread(elites, |x| x.target_temperature_c, 4.0),
                bounds.min_temperature_c,
                bounds.max_temperature_c,
            ),
            stirrer_rpm: sample_around(
                rng,
                mean(elites, |x| x.target_stirrer_rpm),
                spread(elites, |x| x.target_stirrer_rpm, 50.0),
                bounds.min_stirrer_rpm,
                bounds.max_stirrer_rpm,
            ),
            heating_minutes: sample_around(
                rng,
                mean(elites, |x| x.heating_minutes),
                spread(elites, |x| x.heating_minutes, 10.0),
                bounds.min_heating_minutes,
                bounds.max_heating_minutes,
            ),
            stirring_minutes: sample_around(
                rng,
                mean(elites, |x| x.stirring_minutes),
                spread(elites, |x| x.stirring_minutes, 10.0),
                bounds.min_stirring_minutes,
                bounds.max_stirring_minutes,
            ),
        };
        if !is_forbidden(&candidate, forbidden_zones) {
            return Some(candidate);
        }
    }
    None
}

fn safe_midpoint(bounds: &OptimizerBounds, forbidden_zones: &[ForbiddenZone]) -> Candidate {
    let midpoint = Candidate {
        temperature_c: (bounds.min_temperature_c + bounds.max_temperature_c) / 2.0,
        stirrer_rpm: (bounds.min_stirrer_rpm + bounds.max_stirrer_rpm) / 2.0,
        heating_minutes: (bounds.min_heating_minutes + bounds.max_heating_minutes) / 2.0,
        stirring_minutes: (bounds.min_stirring_minutes + bounds.max_stirring_minutes) / 2.0,
    };
    if !is_forbidden(&midpoint, forbidden_zones) {
        return midpoint;
    }

    let anchors = [
        (0.25, 0.25, 0.25, 0.25),
        (0.75, 0.25, 0.25, 0.25),
        (0.25, 0.75, 0.25, 0.25),
        (0.25, 0.25, 0.75, 0.25),
        (0.25, 0.25, 0.25, 0.75),
        (0.75, 0.75, 0.75, 0.75),
    ];
    for (temp, rpm, heating, stirring) in anchors {
        let candidate = Candidate {
            temperature_c: interpolate(bounds.min_temperature_c, bounds.max_temperature_c, temp),
            stirrer_rpm: interpolate(bounds.min_stirrer_rpm, bounds.max_stirrer_rpm, rpm),
            heating_minutes: interpolate(
                bounds.min_heating_minutes,
                bounds.max_heating_minutes,
                heating,
            ),
            stirring_minutes: interpolate(
                bounds.min_stirring_minutes,
                bounds.max_stirring_minutes,
                stirring,
            ),
        };
        if !is_forbidden(&candidate, forbidden_zones) {
            return candidate;
        }
    }

    midpoint
}

fn is_forbidden(candidate: &Candidate, forbidden_zones: &[ForbiddenZone]) -> bool {
    forbidden_zones.iter().any(|zone| {
        zone.contains(
            candidate.temperature_c,
            candidate.stirrer_rpm,
            candidate.heating_minutes,
            candidate.stirring_minutes,
        )
    })
}

fn interpolate(min: f64, max: f64, ratio: f64) -> f64 {
    min + (max - min) * ratio
}

fn reference_to_outcome(reference: &ReferenceBatch) -> BatchOutcome {
    BatchOutcome {
        batch_id: stable_negative_id(&reference.id),
        target_temperature_c: reference.target_temperature_c,
        target_stirrer_rpm: reference.target_stirrer_rpm,
        heating_minutes: reference.heating_minutes,
        stirring_minutes: reference.stirring_minutes,
        yield_percent: reference.yield_percent,
        product_ratio: reference.product_ratio,
    }
}

fn stable_negative_id(id: &str) -> i64 {
    let mut hash = 0_i64;
    for byte in id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as i64);
    }
    -(hash.abs() % 1_000_000 + 1)
}

fn rationale(
    best: &BatchOutcome,
    elite_count: usize,
    real_outcome_count: usize,
    memory_outcome_count: usize,
    memory: Option<&AiMemory>,
    forbidden_note: &str,
) -> String {
    let memory_note = memory
        .filter(|memory| memory.recommendation.enabled)
        .map(|memory| {
            format!(
                " Memory profile {} v{} applied; {} forbidden zones checked.",
                memory.profile.name,
                memory.profile.version,
                memory.forbidden_zones.len()
            )
        })
        .unwrap_or_default();
    let reference_note = if memory_outcome_count > 0 {
        format!(" Included {memory_outcome_count} file reference batches.")
    } else {
        String::new()
    };

    format!(
        "Based on the top {elite_count} batches from {real_outcome_count} recorded and {memory_outcome_count} file reference outcomes; best batch {} reached yield {:.2}% and product ratio {:.3}.{memory_note}{reference_note}{forbidden_note}",
        best.batch_id, best.yield_percent, best.product_ratio
    )
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
