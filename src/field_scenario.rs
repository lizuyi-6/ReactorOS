use serde::{Deserialize, Serialize};

use crate::{
    config::DeviceMode,
    db::{Batch, BatchOutcome, ProcessDefinition},
    memory::AiMemory,
    state::RuntimeState,
};

const ENV_FIELD_SCENARIO: &str = "XINGSHU_FIELD_SCENARIO";
const ENV_PRODUCTION_LINE: &str = "XINGSHU_PRODUCTION_LINE";
const ENV_FIELD_SITE_LABEL: &str = "XINGSHU_FIELD_SITE_LABEL";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldScenarioKind {
    LabResearch,
    PilotScale,
    LegacyRetrofit,
    OfflineDemo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionLineKind {
    GeneralChemistry,
    PetrochemicalRefining,
    Biopharmaceutical,
    FineChemical,
    MaterialSynthesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptationSource {
    Auto,
    EnvironmentOverride,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldScenarioProfile {
    pub kind: FieldScenarioKind,
    pub source: AdaptationSource,
    pub label: &'static str,
    pub confidence: f64,
    pub device_mode: &'static str,
    pub site_label: Option<String>,
    pub signals: Vec<String>,
    pub actions: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProductionLineProfile {
    pub kind: ProductionLineKind,
    pub source: AdaptationSource,
    pub label: &'static str,
    pub confidence: f64,
    pub site_label: Option<String>,
    pub signals: Vec<String>,
    pub actions: Vec<String>,
    pub notes: Vec<String>,
    pub special_handling_required: bool,
    pub petrochemical_handling_required: bool,
}

pub struct FieldScenarioContext<'a> {
    pub device_mode: &'a DeviceMode,
    pub runtime: Option<&'a RuntimeState>,
    pub include_runtime_signals: bool,
    pub memory: &'a AiMemory,
    pub processes: &'a [ProcessDefinition],
    pub recent_batches: &'a [Batch],
    pub recent_outcomes: &'a [BatchOutcome],
}

impl<'a> FieldScenarioContext<'a> {
    pub fn config_only(device_mode: &'a DeviceMode, memory: &'a AiMemory) -> Self {
        Self {
            device_mode,
            runtime: None,
            include_runtime_signals: false,
            memory,
            processes: &[],
            recent_batches: &[],
            recent_outcomes: &[],
        }
    }
}

pub fn detect_field_scenario(context: FieldScenarioContext<'_>) -> FieldScenarioProfile {
    let site_label = env_trimmed(ENV_FIELD_SITE_LABEL);
    if let Some(kind) = scenario_override_from_env() {
        let mut profile = scenario_profile_for(
            kind,
            AdaptationSource::EnvironmentOverride,
            context.device_mode,
        );
        profile.confidence = 1.0;
        profile.site_label = site_label;
        profile.signals.push(format!(
            "{ENV_FIELD_SCENARIO} override selected {}",
            kind.as_str()
        ));
        return profile;
    }

    let text = scenario_text(&context, site_label.as_deref(), false);
    let text_kind = classify_scenario_from_text(&text);
    let kind = if context.include_runtime_signals
        && matches!(context.device_mode, DeviceMode::Pipeline)
        && context
            .runtime
            .and_then(|runtime| runtime.latest_sample.as_ref())
            .is_none()
        && context.recent_batches.is_empty()
        && context.recent_outcomes.is_empty()
    {
        FieldScenarioKind::OfflineDemo
    } else if matches!(text_kind, Some(FieldScenarioKind::OfflineDemo)) {
        FieldScenarioKind::OfflineDemo
    } else if context.recent_batches.len() >= 5
        || context.recent_outcomes.len() >= 3
        || matches!(text_kind, Some(FieldScenarioKind::PilotScale))
    {
        FieldScenarioKind::PilotScale
    } else if matches!(
        context.device_mode,
        DeviceMode::Modbus | DeviceMode::JsonBridge
    ) || matches!(text_kind, Some(FieldScenarioKind::LegacyRetrofit))
    {
        FieldScenarioKind::LegacyRetrofit
    } else {
        text_kind.unwrap_or(FieldScenarioKind::LabResearch)
    };

    let mut profile = scenario_profile_for(kind, AdaptationSource::Auto, context.device_mode);
    profile.site_label = site_label;
    profile.confidence = scenario_confidence_for(&profile, &context);
    append_scenario_signals(&mut profile, &context);
    profile
}

pub fn detect_production_line(context: FieldScenarioContext<'_>) -> ProductionLineProfile {
    let site_label = env_trimmed(ENV_FIELD_SITE_LABEL);
    if let Some(kind) = production_line_override_from_env() {
        let mut profile = production_line_profile_for(kind, AdaptationSource::EnvironmentOverride);
        profile.confidence = 1.0;
        profile.site_label = site_label;
        profile.signals.push(format!(
            "{ENV_PRODUCTION_LINE} override selected {}",
            kind.as_str()
        ));
        return profile;
    }

    let text = scenario_text(&context, site_label.as_deref(), true);
    let kind =
        classify_production_line_from_text(&text).unwrap_or(ProductionLineKind::GeneralChemistry);
    let mut profile = production_line_profile_for(kind, AdaptationSource::Auto);
    profile.site_label = site_label;
    profile.confidence = production_line_confidence_for(kind);
    append_production_line_signals(&mut profile, &context);
    profile
}

fn scenario_override_from_env() -> Option<FieldScenarioKind> {
    let value = env_trimmed(ENV_FIELD_SCENARIO)?;
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    parse_scenario_kind(&value)
}

fn production_line_override_from_env() -> Option<ProductionLineKind> {
    let value = env_trimmed(ENV_PRODUCTION_LINE).or_else(|| env_trimmed(ENV_FIELD_SCENARIO))?;
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    parse_production_line_kind(&value)
}

fn parse_scenario_kind(value: &str) -> Option<FieldScenarioKind> {
    match normalize_token(value).as_str() {
        "labresearch" | "lab" | "research" => Some(FieldScenarioKind::LabResearch),
        "pilotscale" | "pilot" | "scaleup" => Some(FieldScenarioKind::PilotScale),
        "legacyretrofit" | "legacy" | "retrofit" | "modbus" => {
            Some(FieldScenarioKind::LegacyRetrofit)
        }
        "offlinedemo" | "demo" | "offline" => Some(FieldScenarioKind::OfflineDemo),
        _ => None,
    }
}

fn parse_production_line_kind(value: &str) -> Option<ProductionLineKind> {
    match normalize_token(value).as_str() {
        "generalchemistry" | "general" | "chemistry" => Some(ProductionLineKind::GeneralChemistry),
        "petrochemicalrefining"
        | "petrochemical"
        | "petrochem"
        | "refinery"
        | "oil"
        | "shiyouhua"
        | "石油炼化"
        | "石油化"
        | "炼化"
        | "炼油" => Some(ProductionLineKind::PetrochemicalRefining),
        "biopharmaceutical" | "biopharma" | "biotech" | "pharma" | "生物制药" | "发酵" => {
            Some(ProductionLineKind::Biopharmaceutical)
        }
        "finechemical" | "finechem" | "精细化工" => Some(ProductionLineKind::FineChemical),
        "materialsynthesis" | "material" | "polymer" | "材料合成" | "材料" | "聚合" => {
            Some(ProductionLineKind::MaterialSynthesis)
        }
        _ => None,
    }
}

fn scenario_profile_for(
    kind: FieldScenarioKind,
    source: AdaptationSource,
    device_mode: &DeviceMode,
) -> FieldScenarioProfile {
    let (label, actions, notes) = match kind {
        FieldScenarioKind::LabResearch => (
            "Lab research",
            vec![
                "keep conservative optimizer bounds and require verified sample freshness".to_string(),
                "prefer operator confirmation before applying AI recommendations".to_string(),
            ],
            vec!["bench-scale validation; production-line chemistry is evaluated separately".to_string()],
        ),
        FieldScenarioKind::PilotScale => (
            "Pilot scale",
            vec![
                "treat recent finished batches as the active tuning window".to_string(),
                "watch batch closure and product-result recording before recommendation updates".to_string(),
            ],
            vec!["multi-batch operation detected; production-line chemistry is evaluated separately".to_string()],
        ),
        FieldScenarioKind::LegacyRetrofit => (
            "Legacy retrofit",
            vec![
                "verify bridge heartbeat and downstream command receipt before production control".to_string(),
                "keep manual fallback ready for mixed hardware integration".to_string(),
            ],
            vec!["field hardware is mediated through a retrofit bridge or industrial bus".to_string()],
        ),
        FieldScenarioKind::OfflineDemo => (
            "Offline demo",
            vec![
                "block production decisions until fresh field samples are restored".to_string(),
                "use demo context only for workflow validation, not control".to_string(),
            ],
            vec!["pipeline mode has no fresh persisted sample; fail-closed behavior remains expected".to_string()],
        ),
    };

    FieldScenarioProfile {
        kind,
        source,
        label,
        confidence: 0.65,
        device_mode: device_mode_label(device_mode),
        site_label: None,
        signals: Vec::new(),
        actions,
        notes,
    }
}

fn production_line_profile_for(
    kind: ProductionLineKind,
    source: AdaptationSource,
) -> ProductionLineProfile {
    let (label, actions, notes, special_handling_required, petrochemical_handling_required) =
        match kind {
            ProductionLineKind::GeneralChemistry => (
                "General chemistry",
                vec![
                    "apply the base reactor safety envelope and require verified product-result records"
                        .to_string(),
                ],
                vec!["no production-line-specific material family was detected".to_string()],
                false,
                false,
            ),
            ProductionLineKind::PetrochemicalRefining => (
                "Petrochemical refining",
                vec![
                    "require petroleum material review before accepting AI or operator target changes".to_string(),
                    "tighten field verification around pressure, temperature, venting, and product handling".to_string(),
                    "keep control fail-closed until site-specific petroleum hazards are validated".to_string(),
                ],
                vec!["petroleum-like refining product signal detected; deployment scenario remains independent".to_string()],
                true,
                true,
            ),
            ProductionLineKind::Biopharmaceutical => (
                "Biopharmaceutical",
                vec![
                    "separate sterility, contamination, and cleaning validation from reactor control recommendations".to_string(),
                    "require batch traceability before using outcomes for optimization".to_string(),
                ],
                vec!["bio/pharma signal detected; deployment scenario remains independent".to_string()],
                true,
                false,
            ),
            ProductionLineKind::FineChemical => (
                "Fine chemical",
                vec![
                    "keep selectivity and impurity notes attached to product-result records".to_string(),
                    "review forbidden zones against the validated chemistry window".to_string(),
                ],
                vec!["fine-chemical signal detected; deployment scenario remains independent".to_string()],
                false,
                false,
            ),
            ProductionLineKind::MaterialSynthesis => (
                "Material synthesis",
                vec![
                    "track viscosity, mixing, and solid/loading assumptions outside the base sensor envelope".to_string(),
                    "review product concentration interpretation before optimization".to_string(),
                ],
                vec!["material synthesis signal detected; deployment scenario remains independent".to_string()],
                true,
                false,
            ),
        };

    ProductionLineProfile {
        kind,
        source,
        label,
        confidence: 0.65,
        site_label: None,
        signals: Vec::new(),
        actions,
        notes,
        special_handling_required,
        petrochemical_handling_required,
    }
}

fn scenario_confidence_for(
    profile: &FieldScenarioProfile,
    context: &FieldScenarioContext<'_>,
) -> f64 {
    match profile.kind {
        FieldScenarioKind::OfflineDemo => 0.86,
        FieldScenarioKind::LegacyRetrofit
            if matches!(
                context.device_mode,
                DeviceMode::Modbus | DeviceMode::JsonBridge
            ) =>
        {
            0.82
        }
        FieldScenarioKind::PilotScale
            if context.recent_batches.len() >= 5 || context.recent_outcomes.len() >= 3 =>
        {
            0.78
        }
        _ => 0.64,
    }
}

fn production_line_confidence_for(kind: ProductionLineKind) -> f64 {
    match kind {
        ProductionLineKind::GeneralChemistry => 0.55,
        ProductionLineKind::PetrochemicalRefining => 0.9,
        ProductionLineKind::Biopharmaceutical => 0.88,
        ProductionLineKind::FineChemical => 0.76,
        ProductionLineKind::MaterialSynthesis => 0.76,
    }
}

fn append_scenario_signals(profile: &mut FieldScenarioProfile, context: &FieldScenarioContext<'_>) {
    profile.signals.push(format!(
        "device_mode={}",
        device_mode_label(context.device_mode)
    ));
    profile
        .signals
        .push(format!("recent_batches={}", context.recent_batches.len()));
    profile
        .signals
        .push(format!("recent_outcomes={}", context.recent_outcomes.len()));
    if let Some(runtime) = context.runtime {
        profile.signals.push(format!(
            "runtime_sample={}",
            if runtime.latest_sample.is_some() {
                "present"
            } else {
                "missing"
            }
        ));
        if runtime.active_batch_id.is_some() {
            profile.signals.push("active_batch=true".to_string());
        }
    }
}

fn append_production_line_signals(
    profile: &mut ProductionLineProfile,
    context: &FieldScenarioContext<'_>,
) {
    profile.signals.push(format!(
        "memory_material_family={}",
        context.memory.profile.material_family
    ));
    profile
        .signals
        .push(format!("process_count={}", context.processes.len()));
    profile
        .signals
        .push(format!("recent_batches={}", context.recent_batches.len()));
}

fn scenario_text(
    context: &FieldScenarioContext<'_>,
    site_label: Option<&str>,
    include_memory: bool,
) -> String {
    let mut fields = Vec::new();
    if include_memory {
        fields.extend([
            context.memory.profile.name.as_str(),
            context.memory.profile.reactor_model.as_str(),
            context.memory.profile.material_family.as_str(),
            context.memory.objective.optimize_for.as_str(),
            context.memory.objective.notes.as_str(),
        ]);
        fields.extend(
            context
                .memory
                .operator_notes
                .iter()
                .flat_map(|note| [note.topic.as_str(), note.note.as_str()]),
        );
        fields.extend(
            context
                .memory
                .reference_batches
                .iter()
                .map(|batch| batch.notes.as_str()),
        );
        fields.extend(
            context
                .memory
                .forbidden_zones
                .iter()
                .flat_map(|zone| [zone.name.as_str(), zone.reason.as_str()]),
        );
    }
    fields.extend(context.processes.iter().flat_map(|process| {
        [
            process.name.as_str(),
            process.description.as_str(),
            process.status.as_str(),
        ]
    }));
    fields.extend(
        context
            .recent_batches
            .iter()
            .map(|batch| batch.name.as_str()),
    );
    if let Some(label) = site_label {
        fields.push(label);
    }
    fields.join(" ")
}

fn classify_scenario_from_text(text: &str) -> Option<FieldScenarioKind> {
    if matches_any(text, &OFFLINE_KEYWORDS) {
        return Some(FieldScenarioKind::OfflineDemo);
    }
    if matches_any(text, &LEGACY_KEYWORDS) {
        return Some(FieldScenarioKind::LegacyRetrofit);
    }
    if matches_any(text, &PILOT_KEYWORDS) {
        return Some(FieldScenarioKind::PilotScale);
    }
    if matches_any(text, &LAB_KEYWORDS) {
        return Some(FieldScenarioKind::LabResearch);
    }
    None
}

fn classify_production_line_from_text(text: &str) -> Option<ProductionLineKind> {
    if matches_any(text, &PETROCHEMICAL_KEYWORDS) {
        return Some(ProductionLineKind::PetrochemicalRefining);
    }
    if matches_any(text, &BIOPHARMA_KEYWORDS) {
        return Some(ProductionLineKind::Biopharmaceutical);
    }
    if matches_any(text, &MATERIAL_KEYWORDS) {
        return Some(ProductionLineKind::MaterialSynthesis);
    }
    if matches_any(text, &FINE_CHEMICAL_KEYWORDS) {
        return Some(ProductionLineKind::FineChemical);
    }
    None
}

fn matches_any(text: &str, keywords: &[&str]) -> bool {
    let normalized = text.to_ascii_lowercase();
    let compact = normalize_token(text);
    keywords
        .iter()
        .any(|keyword| normalized.contains(keyword) || compact.contains(&normalize_token(keyword)))
}

fn normalize_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || !ch.is_ascii())
        .flat_map(char::to_lowercase)
        .collect()
}

fn env_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn device_mode_label(mode: &DeviceMode) -> &'static str {
    match mode {
        DeviceMode::Pipeline => "pipeline",
        DeviceMode::Modbus => "modbus",
        DeviceMode::Esp32Serial => "esp32_serial",
        DeviceMode::JsonBridge => "json_bridge",
    }
}

impl FieldScenarioKind {
    fn as_str(self) -> &'static str {
        match self {
            FieldScenarioKind::LabResearch => "lab_research",
            FieldScenarioKind::PilotScale => "pilot_scale",
            FieldScenarioKind::LegacyRetrofit => "legacy_retrofit",
            FieldScenarioKind::OfflineDemo => "offline_demo",
        }
    }
}

impl ProductionLineKind {
    fn as_str(self) -> &'static str {
        match self {
            ProductionLineKind::GeneralChemistry => "general_chemistry",
            ProductionLineKind::PetrochemicalRefining => "petrochemical_refining",
            ProductionLineKind::Biopharmaceutical => "biopharmaceutical",
            ProductionLineKind::FineChemical => "fine_chemical",
            ProductionLineKind::MaterialSynthesis => "material_synthesis",
        }
    }
}

const PETROCHEMICAL_KEYWORDS: &[&str] = &[
    "petroleum",
    "petrochemical",
    "petrochem",
    "refinery",
    "refining",
    "crude oil",
    "oil phase",
    "hydrocarbon",
    "naphtha",
    "diesel",
    "gasoline",
    "kerosene",
    "\u{77f3}\u{6cb9}",
    "\u{77f3}\u{6cb9}\u{5316}",
    "\u{70bc}\u{5316}",
    "\u{70bc}\u{6cb9}",
    "\u{539f}\u{6cb9}",
    "\u{6cb9}\u{54c1}",
    "\u{70c3}",
];

const BIOPHARMA_KEYWORDS: &[&str] = &[
    "biopharma",
    "biopharmaceutical",
    "biotech",
    "pharma",
    "pharmaceutical",
    "fermentation",
    "enzyme",
    "cell culture",
    "\u{751f}\u{7269}\u{5236}\u{836f}",
    "\u{751f}\u{7269}\u{53cd}\u{5e94}",
    "\u{53d1}\u{9175}",
    "\u{9176}",
];

const FINE_CHEMICAL_KEYWORDS: &[&str] = &[
    "fine chemical",
    "finechem",
    "specialty chemical",
    "intermediate",
    "selectivity",
    "\u{7cbe}\u{7ec6}\u{5316}\u{5de5}",
    "\u{4e2d}\u{95f4}\u{4f53}",
    "\u{9009}\u{62e9}\u{6027}",
];

const MATERIAL_KEYWORDS: &[&str] = &[
    "material",
    "polymer",
    "resin",
    "slurry",
    "solid loading",
    "viscosity",
    "\u{6750}\u{6599}",
    "\u{805a}\u{5408}",
    "\u{6811}\u{8102}",
    "\u{6d46}\u{6599}",
    "\u{9ecf}\u{5ea6}",
];

const OFFLINE_KEYWORDS: &[&str] = &[
    "offline",
    "demo",
    "simulation",
    "sim",
    "training",
    "preview",
    "\u{6f14}\u{793a}",
    "\u{4eff}\u{771f}",
    "\u{79bb}\u{7ebf}",
    "\u{6c99}\u{76d8}",
];

const LEGACY_KEYWORDS: &[&str] = &[
    "legacy",
    "retrofit",
    "bridge",
    "adapter",
    "upgrade",
    "old line",
    "oldline",
    "\u{6539}\u{9020}",
    "\u{8001}\u{7ebf}",
    "\u{517c}\u{5bb9}",
    "\u{7f51}\u{5173}",
];

const PILOT_KEYWORDS: &[&str] = &[
    "pilot",
    "pilot-scale",
    "pilot scale",
    "scale-up",
    "scaleup",
    "trial",
    "mid-scale",
    "\u{4e2d}\u{8bd5}",
    "\u{8bd5}\u{9a8c}\u{7ebf}",
    "\u{653e}\u{5927}",
];

const LAB_KEYWORDS: &[&str] = &[
    "lab",
    "laboratory",
    "research",
    "bench",
    "r&d",
    "rd",
    "development",
    "\u{7814}\u{53d1}",
    "\u{5b9e}\u{9a8c}\u{5ba4}",
    "\u{5c0f}\u{8bd5}",
];
