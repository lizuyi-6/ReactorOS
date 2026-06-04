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
    let candidate = hybrid_search_candidate(
        &mut rng,
        &sorted,
        elites,
        &effective_bounds,
        forbidden_zones,
        objective_weights,
    )
    .or_else(|| {
        sample_allowed_candidate(&mut rng, elites, &effective_bounds, forbidden_zones).map(
            |candidate| SearchOutcome {
                candidate,
                note: "Local stochastic optimizer used elite-neighborhood fallback sampling."
                    .to_string(),
            },
        )
    });
    let (temp, rpm, heating, stirring, forbidden_note, optimizer_note) = match candidate {
        Some(search) => (
            search.candidate.temperature_c,
            search.candidate.stirrer_rpm,
            search.candidate.heating_minutes,
            search.candidate.stirring_minutes,
            "",
            search.note,
        ),
        None => {
            let fallback = safe_midpoint(&effective_bounds, forbidden_zones);
            (
                fallback.temperature_c,
                fallback.stirrer_rpm,
                fallback.heating_minutes,
                fallback.stirring_minutes,
                " All sampled candidates matched a forbidden zone; using the nearest safe midpoint.",
                "Local optimizer fell back to nearest safe midpoint.".to_string(),
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
            &optimizer_note,
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

#[derive(Debug, Clone)]
struct SearchOutcome {
    candidate: Candidate,
    note: String,
}

fn hybrid_search_candidate(
    rng: &mut impl Rng,
    outcomes: &[BatchOutcome],
    elites: &[BatchOutcome],
    bounds: &OptimizerBounds,
    forbidden_zones: &[ForbiddenZone],
    weights: (f64, f64),
) -> Option<SearchOutcome> {
    let mut population = seed_population(rng, elites, bounds, forbidden_zones);
    if population.is_empty() {
        return None;
    }

    for _ in 0..18 {
        population.sort_by(|a, b| {
            candidate_quality(b, outcomes, bounds, weights)
                .total_cmp(&candidate_quality(a, outcomes, bounds, weights))
        });
        population.truncate(12);
        let parents = population.clone();
        for _ in 0..12 {
            let first = &parents[rng.gen_range(0..parents.len())];
            let second = &parents[rng.gen_range(0..parents.len())];
            let child = crossover_candidate(rng, first, second, bounds);
            let child = mutate_candidate(rng, child, bounds, 1.0);
            let child = pid_correct_candidate(child, outcomes, elites, bounds);
            if !is_forbidden(&child, forbidden_zones) {
                population.push(child);
            }
        }
    }

    population.sort_by(|a, b| {
        candidate_quality(b, outcomes, bounds, weights)
            .total_cmp(&candidate_quality(a, outcomes, bounds, weights))
    });
    let mut current = population[0].clone();
    let mut current_score = candidate_quality(&current, outcomes, bounds, weights);
    let mut temperature = 8.0_f64;

    for _ in 0..32 {
        let neighbor = mutate_candidate(rng, current.clone(), bounds, temperature / 8.0);
        let neighbor = pid_correct_candidate(neighbor, outcomes, elites, bounds);
        if is_forbidden(&neighbor, forbidden_zones) {
            temperature *= 0.86;
            continue;
        }
        let neighbor_score = candidate_quality(&neighbor, outcomes, bounds, weights);
        let delta = neighbor_score - current_score;
        let accept_probability = (delta / temperature.max(0.001)).exp().clamp(0.0, 1.0);
        if delta >= 0.0 || rng.gen_bool(accept_probability) {
            current = neighbor;
            current_score = neighbor_score;
        }
        temperature *= 0.86;
    }

    Some(SearchOutcome {
        candidate: current,
        note: "Local GA/SA/PID optimizer searched with genetic crossover/mutation, simulated annealing acceptance, and PID-style error correction.".to_string(),
    })
}

fn seed_population(
    rng: &mut impl Rng,
    elites: &[BatchOutcome],
    bounds: &OptimizerBounds,
    forbidden_zones: &[ForbiddenZone],
) -> Vec<Candidate> {
    let mut population = Vec::new();
    for elite in elites {
        push_if_allowed(
            &mut population,
            outcome_candidate(elite, bounds),
            forbidden_zones,
        );
    }
    push_if_allowed(
        &mut population,
        safe_midpoint(bounds, forbidden_zones),
        forbidden_zones,
    );
    for _ in 0..32 {
        if let Some(candidate) = sample_allowed_candidate(rng, elites, bounds, forbidden_zones) {
            population.push(candidate);
        }
    }
    population
}

fn push_if_allowed(
    population: &mut Vec<Candidate>,
    candidate: Candidate,
    forbidden_zones: &[ForbiddenZone],
) {
    if !is_forbidden(&candidate, forbidden_zones) {
        population.push(candidate);
    }
}

fn outcome_candidate(outcome: &BatchOutcome, bounds: &OptimizerBounds) -> Candidate {
    Candidate {
        temperature_c: outcome
            .target_temperature_c
            .clamp(bounds.min_temperature_c, bounds.max_temperature_c),
        stirrer_rpm: outcome
            .target_stirrer_rpm
            .clamp(bounds.min_stirrer_rpm, bounds.max_stirrer_rpm),
        heating_minutes: outcome
            .heating_minutes
            .clamp(bounds.min_heating_minutes, bounds.max_heating_minutes),
        stirring_minutes: outcome
            .stirring_minutes
            .clamp(bounds.min_stirring_minutes, bounds.max_stirring_minutes),
    }
}

fn crossover_candidate(
    rng: &mut impl Rng,
    first: &Candidate,
    second: &Candidate,
    bounds: &OptimizerBounds,
) -> Candidate {
    let blend = rng.gen_range(0.25..=0.75);
    Candidate {
        temperature_c: blend_value(first.temperature_c, second.temperature_c, blend)
            .clamp(bounds.min_temperature_c, bounds.max_temperature_c),
        stirrer_rpm: blend_value(first.stirrer_rpm, second.stirrer_rpm, blend)
            .clamp(bounds.min_stirrer_rpm, bounds.max_stirrer_rpm),
        heating_minutes: blend_value(first.heating_minutes, second.heating_minutes, blend)
            .clamp(bounds.min_heating_minutes, bounds.max_heating_minutes),
        stirring_minutes: blend_value(first.stirring_minutes, second.stirring_minutes, blend)
            .clamp(bounds.min_stirring_minutes, bounds.max_stirring_minutes),
    }
}

fn mutate_candidate(
    rng: &mut impl Rng,
    candidate: Candidate,
    bounds: &OptimizerBounds,
    scale: f64,
) -> Candidate {
    let scale = scale.clamp(0.2, 1.5);
    Candidate {
        temperature_c: mutate_axis(
            rng,
            candidate.temperature_c,
            bounds.min_temperature_c,
            bounds.max_temperature_c,
            0.08 * scale,
        ),
        stirrer_rpm: mutate_axis(
            rng,
            candidate.stirrer_rpm,
            bounds.min_stirrer_rpm,
            bounds.max_stirrer_rpm,
            0.08 * scale,
        ),
        heating_minutes: mutate_axis(
            rng,
            candidate.heating_minutes,
            bounds.min_heating_minutes,
            bounds.max_heating_minutes,
            0.10 * scale,
        ),
        stirring_minutes: mutate_axis(
            rng,
            candidate.stirring_minutes,
            bounds.min_stirring_minutes,
            bounds.max_stirring_minutes,
            0.10 * scale,
        ),
    }
}

fn pid_correct_candidate(
    candidate: Candidate,
    outcomes: &[BatchOutcome],
    elites: &[BatchOutcome],
    bounds: &OptimizerBounds,
) -> Candidate {
    let best = &outcomes[0];
    let second = outcomes.get(1).unwrap_or(best);
    let elite_mean = Candidate {
        temperature_c: mean(elites, |item| item.target_temperature_c),
        stirrer_rpm: mean(elites, |item| item.target_stirrer_rpm),
        heating_minutes: mean(elites, |item| item.heating_minutes),
        stirring_minutes: mean(elites, |item| item.stirring_minutes),
    };

    Candidate {
        temperature_c: pid_axis(
            candidate.temperature_c,
            best.target_temperature_c,
            elite_mean.temperature_c,
            best.target_temperature_c - second.target_temperature_c,
            bounds.min_temperature_c,
            bounds.max_temperature_c,
        ),
        stirrer_rpm: pid_axis(
            candidate.stirrer_rpm,
            best.target_stirrer_rpm,
            elite_mean.stirrer_rpm,
            best.target_stirrer_rpm - second.target_stirrer_rpm,
            bounds.min_stirrer_rpm,
            bounds.max_stirrer_rpm,
        ),
        heating_minutes: pid_axis(
            candidate.heating_minutes,
            best.heating_minutes,
            elite_mean.heating_minutes,
            best.heating_minutes - second.heating_minutes,
            bounds.min_heating_minutes,
            bounds.max_heating_minutes,
        ),
        stirring_minutes: pid_axis(
            candidate.stirring_minutes,
            best.stirring_minutes,
            elite_mean.stirring_minutes,
            best.stirring_minutes - second.stirring_minutes,
            bounds.min_stirring_minutes,
            bounds.max_stirring_minutes,
        ),
    }
}

fn candidate_quality(
    candidate: &Candidate,
    outcomes: &[BatchOutcome],
    bounds: &OptimizerBounds,
    weights: (f64, f64),
) -> f64 {
    let temp_span = (bounds.max_temperature_c - bounds.min_temperature_c).max(1.0);
    let rpm_span = (bounds.max_stirrer_rpm - bounds.min_stirrer_rpm).max(1.0);
    let heating_span = (bounds.max_heating_minutes - bounds.min_heating_minutes).max(1.0);
    let stirring_span = (bounds.max_stirring_minutes - bounds.min_stirring_minutes).max(1.0);
    let mut weighted_score = 0.0;
    let mut total_weight = 0.0;
    for outcome in outcomes {
        let distance = (((candidate.temperature_c - outcome.target_temperature_c) / temp_span)
            .powi(2)
            + ((candidate.stirrer_rpm - outcome.target_stirrer_rpm) / rpm_span).powi(2)
            + ((candidate.heating_minutes - outcome.heating_minutes) / heating_span).powi(2)
            + ((candidate.stirring_minutes - outcome.stirring_minutes) / stirring_span).powi(2))
        .sqrt();
        let weight = 1.0 / (0.08 + distance);
        weighted_score += score_with_weights(outcome, weights) * weight;
        total_weight += weight;
    }
    if total_weight == 0.0 {
        0.0
    } else {
        weighted_score / total_weight
    }
}

fn blend_value(first: f64, second: f64, ratio: f64) -> f64 {
    first * ratio + second * (1.0 - ratio)
}

fn mutate_axis(rng: &mut impl Rng, value: f64, min: f64, max: f64, ratio: f64) -> f64 {
    let span = (max - min).max(1.0);
    (value + rng.gen_range(-(span * ratio)..=(span * ratio))).clamp(min, max)
}

fn pid_axis(
    current: f64,
    best: f64,
    integral_anchor: f64,
    derivative: f64,
    min: f64,
    max: f64,
) -> f64 {
    let proportional = best - current;
    let integral = integral_anchor - current;
    (current + proportional * 0.18 + integral * 0.06 + derivative * 0.03).clamp(min, max)
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
    optimizer_note: &str,
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
        "Based on the top {elite_count} batches from {real_outcome_count} recorded and {memory_outcome_count} file reference outcomes; best batch {} reached yield {:.2}% and product ratio {:.3}. {optimizer_note}{memory_note}{reference_note}{forbidden_note}",
        best.batch_id, best.yield_percent, best.product_ratio
    )
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
