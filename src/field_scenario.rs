use serde::{Deserialize, Serialize};

use crate::{
    config::DeviceMode,
    db::{Batch, BatchOutcome, ProcessDefinition},
    memory::AiMemory,
    state::RuntimeState,
};

const ENV_FIELD_SCENARIO: &str = "XINGSHU_FIELD_SCENARIO";
const ENV_FIELD_SITE_LABEL: &str = "XINGSHU_FIELD_SITE_LABEL";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldScenarioKind {
    LabResearch,
    PilotScale,
    LegacyRetrofit,
    OfflineDemo,
    Petrochemical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldScenarioSource {
    Auto,
    EnvironmentOverride,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldScenarioProfile {
    pub kind: FieldScenarioKind,
    pub source: FieldScenarioSource,
    pub label: &'static str,
    pub confidence: f64,
    pub device_mode: &'static str,
    pub site_label: Option<String>,
    pub signals: Vec<String>,
    pub actions: Vec<String>,
    pub notes: Vec<String>,
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
        let mut profile = profile_for(
            kind,
            FieldScenarioSource::EnvironmentOverride,
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

    let text = scenario_text(&context, site_label.as_deref());
    let text_kind = classify_from_text(&text);
    let kind = if matches!(text_kind, Some(FieldScenarioKind::Petrochemical)) {
        FieldScenarioKind::Petrochemical
    } else if context.include_runtime_signals
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

    let mut profile = profile_for(kind, FieldScenarioSource::Auto, context.device_mode);
    profile.site_label = site_label;
    profile.confidence = confidence_for(&profile, &context);
    append_common_signals(&mut profile, &context);
    profile
}

fn scenario_override_from_env() -> Option<FieldScenarioKind> {
    let value = env_trimmed(ENV_FIELD_SCENARIO)?;
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    parse_scenario_kind(&value)
}

fn parse_scenario_kind(value: &str) -> Option<FieldScenarioKind> {
    match normalize_token(value).as_str() {
        "labresearch" | "lab" | "research" => Some(FieldScenarioKind::LabResearch),
        "pilotscale" | "pilot" | "scaleup" => Some(FieldScenarioKind::PilotScale),
        "legacyretrofit" | "legacy" | "retrofit" | "modbus" => {
            Some(FieldScenarioKind::LegacyRetrofit)
        }
        "offlinedemo" | "demo" | "offline" => Some(FieldScenarioKind::OfflineDemo),
        "petrochemical" | "petrochem" | "refinery" | "oil" | "shiyouhua" => {
            Some(FieldScenarioKind::Petrochemical)
        }
        _ => None,
    }
}

fn profile_for(
    kind: FieldScenarioKind,
    source: FieldScenarioSource,
    device_mode: &DeviceMode,
) -> FieldScenarioProfile {
    let (label, actions, notes) = match kind {
        FieldScenarioKind::LabResearch => (
            "Lab research",
            vec![
                "keep conservative optimizer bounds and require verified sample freshness".to_string(),
                "prefer operator confirmation before applying AI recommendations".to_string(),
            ],
            vec!["bench-scale validation; use recorded outcomes to build confidence".to_string()],
        ),
        FieldScenarioKind::PilotScale => (
            "Pilot scale",
            vec![
                "treat recent finished batches as the active tuning window".to_string(),
                "watch batch closure and product-result recording before recommendation updates".to_string(),
            ],
            vec!["multi-batch operation detected; keep production-state recovery visible".to_string()],
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
        FieldScenarioKind::Petrochemical => (
            "Petrochemical",
            vec![
                "require petroleum material review before accepting AI or operator target changes".to_string(),
                "tighten field verification around pressure, temperature, venting, and product handling".to_string(),
                "keep control fail-closed until site-specific petroleum hazards are validated".to_string(),
            ],
            vec![
                "petroleum-like refining product signal detected; this profile flags stricter handling and review".to_string(),
            ],
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
        petrochemical_handling_required: matches!(kind, FieldScenarioKind::Petrochemical),
    }
}

fn confidence_for(profile: &FieldScenarioProfile, context: &FieldScenarioContext<'_>) -> f64 {
    match profile.kind {
        FieldScenarioKind::Petrochemical => 0.9,
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

fn append_common_signals(profile: &mut FieldScenarioProfile, context: &FieldScenarioContext<'_>) {
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
    profile.signals.push(format!(
        "memory_material_family={}",
        context.memory.profile.material_family
    ));
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

fn scenario_text(context: &FieldScenarioContext<'_>, site_label: Option<&str>) -> String {
    let mut fields = Vec::new();
    if !context.include_runtime_signals {
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

fn classify_from_text(text: &str) -> Option<FieldScenarioKind> {
    if matches_any(text, &PETROCHEMICAL_KEYWORDS) {
        return Some(FieldScenarioKind::Petrochemical);
    }
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
            FieldScenarioKind::Petrochemical => "petrochemical",
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
