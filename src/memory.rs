use std::{fs, path::Path};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::OptimizerBounds;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiMemory {
    #[serde(default)]
    pub profile: MemoryProfile,
    #[serde(default)]
    pub objective: ObjectiveMemory,
    #[serde(default)]
    pub recommendation: RecommendationMemory,
    #[serde(default)]
    pub sensor_limits: SensorLimits,
    #[serde(default)]
    pub reference_batches: Vec<ReferenceBatch>,
    #[serde(default)]
    pub forbidden_zones: Vec<ForbiddenZone>,
    #[serde(default)]
    pub operator_notes: Vec<OperatorNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProfile {
    pub name: String,
    pub version: String,
    pub reactor_model: String,
    pub material_family: String,
}

impl Default for MemoryProfile {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            version: "0".to_string(),
            reactor_model: "unknown".to_string(),
            material_family: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectiveMemory {
    pub optimize_for: String,
    pub yield_weight: f64,
    pub product_ratio_weight: f64,
    pub notes: String,
}

impl Default for ObjectiveMemory {
    fn default() -> Self {
        Self {
            optimize_for: "yield_percent_with_product_ratio".to_string(),
            yield_weight: 0.8,
            product_ratio_weight: 0.2,
            notes: String::new(),
        }
    }
}

impl ObjectiveMemory {
    pub fn weights(&self) -> (f64, f64) {
        let yield_weight = finite_or(self.yield_weight, 0.8).max(0.0);
        let ratio_weight = finite_or(self.product_ratio_weight, 0.2).max(0.0);
        if yield_weight + ratio_weight <= f64::EPSILON {
            (0.8, 0.2)
        } else {
            (yield_weight, ratio_weight)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationMemory {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub use_reference_batches: bool,
    #[serde(default)]
    pub bounds: MemoryOptimizerBounds,
}

impl Default for RecommendationMemory {
    fn default() -> Self {
        Self {
            enabled: false,
            use_reference_batches: false,
            bounds: MemoryOptimizerBounds::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryOptimizerBounds {
    pub min_temperature_c: Option<f64>,
    pub max_temperature_c: Option<f64>,
    pub min_stirrer_rpm: Option<f64>,
    pub max_stirrer_rpm: Option<f64>,
    pub min_heating_minutes: Option<f64>,
    pub max_heating_minutes: Option<f64>,
    pub min_stirring_minutes: Option<f64>,
    pub max_stirring_minutes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceBatch {
    pub id: String,
    pub target_temperature_c: f64,
    pub target_stirrer_rpm: f64,
    pub heating_minutes: f64,
    pub stirring_minutes: f64,
    pub yield_percent: f64,
    pub product_ratio: f64,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForbiddenZone {
    pub name: String,
    pub reason: String,
    pub min_temperature_c: Option<f64>,
    pub max_temperature_c: Option<f64>,
    pub min_stirrer_rpm: Option<f64>,
    pub max_stirrer_rpm: Option<f64>,
    pub min_heating_minutes: Option<f64>,
    pub max_heating_minutes: Option<f64>,
    pub min_stirring_minutes: Option<f64>,
    pub max_stirring_minutes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SensorLimits {
    pub temperature_c: Option<SensorLimit>,
    pub pressure_mpa: Option<SensorLimit>,
    pub stirrer_rpm: Option<SensorLimit>,
    pub shake_speed_cpm: Option<SensorLimit>,
    pub tilt_angle_deg: Option<SensorLimit>,
    pub flow_rate_l_min: Option<SensorLimit>,
    pub product_concentration_percent: Option<SensorLimit>,
    pub ph: Option<SensorLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SensorLimit {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub unit: String,
    pub normal_min: Option<f64>,
    pub normal_max: Option<f64>,
    pub hard_min: Option<f64>,
    pub hard_max: Option<f64>,
    #[serde(default)]
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LimitLevel {
    Warning,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitAlarm {
    pub level: LimitLevel,
    pub current_value: f64,
    pub limit_value: f64,
    pub message: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperatorNote {
    pub topic: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiMemorySummary {
    pub profile_name: String,
    pub profile_version: String,
    pub enabled: bool,
    pub objective: String,
    pub reference_batch_count: usize,
    pub forbidden_zone_count: usize,
    pub sensor_limit_count: usize,
}

impl From<&AiMemory> for AiMemorySummary {
    fn from(memory: &AiMemory) -> Self {
        Self {
            profile_name: memory.profile.name.clone(),
            profile_version: memory.profile.version.clone(),
            enabled: memory.recommendation.enabled,
            objective: memory.objective.optimize_for.clone(),
            reference_batch_count: memory.reference_batches.len(),
            forbidden_zone_count: memory.forbidden_zones.len(),
            sensor_limit_count: memory.sensor_limits.configured_count(),
        }
    }
}

impl AiMemory {
    pub fn effective_optimizer_bounds(&self, base: &OptimizerBounds) -> OptimizerBounds {
        if !self.recommendation.enabled {
            return base.clone();
        }

        let bounds = self.recommendation.bounds.tighten(base);
        if bounds.min_temperature_c > bounds.max_temperature_c
            || bounds.min_stirrer_rpm > bounds.max_stirrer_rpm
            || bounds.min_heating_minutes > bounds.max_heating_minutes
            || bounds.min_stirring_minutes > bounds.max_stirring_minutes
        {
            base.clone()
        } else {
            bounds
        }
    }

    pub fn validate_against_optimizer_bounds(&self, base: &OptimizerBounds) -> Result<()> {
        self.validate()?;
        let bounds = self.recommendation.bounds.tighten(base);
        if bounds.min_temperature_c > bounds.max_temperature_c {
            return Err(anyhow!(
                "AI memory temperature bounds do not intersect configured safety optimizer bounds"
            ));
        }
        if bounds.min_stirrer_rpm > bounds.max_stirrer_rpm {
            return Err(anyhow!(
                "AI memory stirrer bounds do not intersect configured safety optimizer bounds"
            ));
        }
        if bounds.min_heating_minutes > bounds.max_heating_minutes {
            return Err(anyhow!(
                "AI memory heating bounds do not intersect configured safety optimizer bounds"
            ));
        }
        if bounds.min_stirring_minutes > bounds.max_stirring_minutes {
            return Err(anyhow!(
                "AI memory stirring bounds do not intersect configured safety optimizer bounds"
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if !self.objective.yield_weight.is_finite()
            || !self.objective.product_ratio_weight.is_finite()
        {
            return Err(anyhow!("AI memory objective weights must be finite"));
        }
        self.recommendation
            .bounds
            .validate("recommendation.bounds")?;
        for batch in &self.reference_batches {
            validate_finite(
                "reference_batches.target_temperature_c",
                batch.target_temperature_c,
            )?;
            validate_finite(
                "reference_batches.target_stirrer_rpm",
                batch.target_stirrer_rpm,
            )?;
            validate_finite("reference_batches.heating_minutes", batch.heating_minutes)?;
            validate_finite("reference_batches.stirring_minutes", batch.stirring_minutes)?;
            validate_range(
                "reference_batches.yield_percent",
                batch.yield_percent,
                0.0,
                100.0,
            )?;
            validate_range(
                "reference_batches.product_ratio",
                batch.product_ratio,
                0.0,
                1.0,
            )?;
        }
        for zone in &self.forbidden_zones {
            zone.validate()?;
        }
        self.sensor_limits.validate()?;
        Ok(())
    }
}

impl MemoryOptimizerBounds {
    fn tighten(&self, base: &OptimizerBounds) -> OptimizerBounds {
        OptimizerBounds {
            min_temperature_c: self
                .min_temperature_c
                .map_or(base.min_temperature_c, |value| {
                    base.min_temperature_c.max(value)
                }),
            max_temperature_c: self
                .max_temperature_c
                .map_or(base.max_temperature_c, |value| {
                    base.max_temperature_c.min(value)
                }),
            min_stirrer_rpm: self.min_stirrer_rpm.map_or(base.min_stirrer_rpm, |value| {
                base.min_stirrer_rpm.max(value)
            }),
            max_stirrer_rpm: self.max_stirrer_rpm.map_or(base.max_stirrer_rpm, |value| {
                base.max_stirrer_rpm.min(value)
            }),
            min_heating_minutes: self
                .min_heating_minutes
                .map_or(base.min_heating_minutes, |value| {
                    base.min_heating_minutes.max(value)
                }),
            max_heating_minutes: self
                .max_heating_minutes
                .map_or(base.max_heating_minutes, |value| {
                    base.max_heating_minutes.min(value)
                }),
            min_stirring_minutes: self
                .min_stirring_minutes
                .map_or(base.min_stirring_minutes, |value| {
                    base.min_stirring_minutes.max(value)
                }),
            max_stirring_minutes: self
                .max_stirring_minutes
                .map_or(base.max_stirring_minutes, |value| {
                    base.max_stirring_minutes.min(value)
                }),
        }
    }

    fn validate(&self, label: &str) -> Result<()> {
        validate_optional_pair(
            label,
            "temperature_c",
            self.min_temperature_c,
            self.max_temperature_c,
        )?;
        validate_optional_pair(
            label,
            "stirrer_rpm",
            self.min_stirrer_rpm,
            self.max_stirrer_rpm,
        )?;
        validate_optional_pair(
            label,
            "heating_minutes",
            self.min_heating_minutes,
            self.max_heating_minutes,
        )?;
        validate_optional_pair(
            label,
            "stirring_minutes",
            self.min_stirring_minutes,
            self.max_stirring_minutes,
        )?;
        Ok(())
    }
}

impl ForbiddenZone {
    pub fn contains(
        &self,
        target_temperature_c: f64,
        target_stirrer_rpm: f64,
        heating_minutes: f64,
        stirring_minutes: f64,
    ) -> bool {
        let mut has_dimension = false;
        let mut matches_all = true;
        check_dimension(
            self.min_temperature_c,
            self.max_temperature_c,
            target_temperature_c,
            &mut has_dimension,
            &mut matches_all,
        );
        check_dimension(
            self.min_stirrer_rpm,
            self.max_stirrer_rpm,
            target_stirrer_rpm,
            &mut has_dimension,
            &mut matches_all,
        );
        check_dimension(
            self.min_heating_minutes,
            self.max_heating_minutes,
            heating_minutes,
            &mut has_dimension,
            &mut matches_all,
        );
        check_dimension(
            self.min_stirring_minutes,
            self.max_stirring_minutes,
            stirring_minutes,
            &mut has_dimension,
            &mut matches_all,
        );
        has_dimension && matches_all
    }

    fn validate(&self) -> Result<()> {
        validate_optional_pair(
            "forbidden_zones",
            "temperature_c",
            self.min_temperature_c,
            self.max_temperature_c,
        )?;
        validate_optional_pair(
            "forbidden_zones",
            "stirrer_rpm",
            self.min_stirrer_rpm,
            self.max_stirrer_rpm,
        )?;
        validate_optional_pair(
            "forbidden_zones",
            "heating_minutes",
            self.min_heating_minutes,
            self.max_heating_minutes,
        )?;
        validate_optional_pair(
            "forbidden_zones",
            "stirring_minutes",
            self.min_stirring_minutes,
            self.max_stirring_minutes,
        )?;
        Ok(())
    }
}

impl SensorLimits {
    pub fn configured_count(&self) -> usize {
        [
            self.temperature_c.as_ref(),
            self.pressure_mpa.as_ref(),
            self.stirrer_rpm.as_ref(),
            self.shake_speed_cpm.as_ref(),
            self.tilt_angle_deg.as_ref(),
            self.flow_rate_l_min.as_ref(),
            self.product_concentration_percent.as_ref(),
            self.ph.as_ref(),
        ]
        .iter()
        .filter(|item| item.is_some())
        .count()
    }

    fn validate(&self) -> Result<()> {
        for (name, limit) in [
            ("temperature_c", self.temperature_c.as_ref()),
            ("pressure_mpa", self.pressure_mpa.as_ref()),
            ("stirrer_rpm", self.stirrer_rpm.as_ref()),
            ("shake_speed_cpm", self.shake_speed_cpm.as_ref()),
            ("tilt_angle_deg", self.tilt_angle_deg.as_ref()),
            ("flow_rate_l_min", self.flow_rate_l_min.as_ref()),
            (
                "product_concentration_percent",
                self.product_concentration_percent.as_ref(),
            ),
            ("ph", self.ph.as_ref()),
        ] {
            if let Some(limit) = limit {
                limit.validate(name)?;
            }
        }
        Ok(())
    }
}

impl SensorLimit {
    pub fn check(&self, value: f64) -> Option<LimitAlarm> {
        if let Some(limit) = self.hard_min {
            if value < limit {
                return Some(self.alarm(LimitLevel::High, value, limit, "below hard minimum"));
            }
        }
        if let Some(limit) = self.hard_max {
            if value > limit {
                return Some(self.alarm(LimitLevel::High, value, limit, "above hard maximum"));
            }
        }
        if let Some(limit) = self.normal_min {
            if value < limit {
                return Some(self.alarm(LimitLevel::Warning, value, limit, "below normal range"));
            }
        }
        if let Some(limit) = self.normal_max {
            if value > limit {
                return Some(self.alarm(LimitLevel::Warning, value, limit, "above normal range"));
            }
        }
        None
    }

    fn alarm(
        &self,
        level: LimitLevel,
        current_value: f64,
        limit_value: f64,
        direction: &str,
    ) -> LimitAlarm {
        let name = if self.label.is_empty() {
            "sensor".to_string()
        } else {
            self.label.clone()
        };
        let unit = if self.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", self.unit)
        };
        LimitAlarm {
            level,
            current_value,
            limit_value,
            message: format!(
                "{name} {direction}: current {:.2}{unit}, limit {:.2}{unit}",
                current_value, limit_value
            ),
            suggestion: self.suggestion.clone(),
        }
    }

    fn validate(&self, name: &str) -> Result<()> {
        validate_optional_pair(name, "normal", self.normal_min, self.normal_max)?;
        validate_optional_pair(name, "hard", self.hard_min, self.hard_max)?;
        Ok(())
    }
}

pub fn load_ai_memory(path: impl AsRef<Path>) -> Result<AiMemory> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read AI memory config {}", path.display()))?;
    let memory: AiMemory = toml::from_str(&raw)
        .with_context(|| format!("failed to parse AI memory config {}", path.display()))?;
    memory.validate()?;
    Ok(memory)
}

fn default_true() -> bool {
    true
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn validate_finite(name: &str, value: f64) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(anyhow!("{name} must be finite"))
    }
}

fn validate_range(name: &str, value: f64, min: f64, max: f64) -> Result<()> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(anyhow!("{name} must be between {min} and {max}"))
    }
}

fn validate_optional_pair(
    label: &str,
    field: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<()> {
    if let Some(value) = min {
        validate_finite(&format!("{label}.{field}.min"), value)?;
    }
    if let Some(value) = max {
        validate_finite(&format!("{label}.{field}.max"), value)?;
    }
    if let (Some(min), Some(max)) = (min, max) {
        if min > max {
            return Err(anyhow!("{label}.{field} min cannot exceed max"));
        }
    }
    Ok(())
}

fn check_dimension(
    min: Option<f64>,
    max: Option<f64>,
    value: f64,
    has_dimension: &mut bool,
    matches_all: &mut bool,
) {
    if min.is_none() && max.is_none() {
        return;
    }
    *has_dimension = true;
    if let Some(min) = min {
        if value < min {
            *matches_all = false;
        }
    }
    if let Some(max) = max {
        if value > max {
            *matches_all = false;
        }
    }
}
