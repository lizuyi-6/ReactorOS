use std::{
    env,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalAiStatus {
    pub enabled: bool,
    pub runtime: String,
    pub model_family: String,
    pub inference: LocalAiStageStatus,
    pub lora_adapter: LocalAiStageStatus,
    pub training: LocalAiStageStatus,
    pub conversion: LocalAiStageStatus,
    pub rk_validation: LocalAiStageStatus,
    pub ready_for_inference: bool,
    pub ready_for_training: bool,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalAiStageStatus {
    pub configured: bool,
    pub available: bool,
    pub path: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone)]
struct LocalAiConfig {
    enabled: bool,
    runtime: String,
    model_family: String,
    inference_bin: Option<PathBuf>,
    gguf_model: Option<PathBuf>,
    lora_adapter: Option<PathBuf>,
    training_script: Option<PathBuf>,
    conversion_script: Option<PathBuf>,
    rk_report: Option<PathBuf>,
}

impl LocalAiStatus {
    pub fn from_env() -> Self {
        LocalAiConfig::from_env().status()
    }
}

impl LocalAiConfig {
    fn from_env() -> Self {
        Self {
            enabled: env_bool("XINGSHU_LOCAL_AI_ENABLED"),
            runtime: env::var("XINGSHU_LOCAL_AI_RUNTIME")
                .unwrap_or_else(|_| "llama.cpp".to_string()),
            model_family: env::var("XINGSHU_LOCAL_AI_MODEL_FAMILY")
                .unwrap_or_else(|_| "Qwen3.5-2B".to_string()),
            inference_bin: env_path("XINGSHU_LOCAL_AI_BIN"),
            gguf_model: env_path("XINGSHU_LOCAL_AI_GGUF"),
            lora_adapter: env_path("XINGSHU_LOCAL_AI_LORA"),
            training_script: env_path("XINGSHU_LOCAL_AI_TRAIN_SCRIPT"),
            conversion_script: env_path("XINGSHU_LOCAL_AI_CONVERT_SCRIPT"),
            rk_report: env_path("XINGSHU_LOCAL_AI_RK_REPORT"),
        }
    }

    fn status(&self) -> LocalAiStatus {
        let inference_bin = file_status(
            self.inference_bin.as_deref(),
            "set XINGSHU_LOCAL_AI_BIN to llama.cpp or compatible local inference binary",
            "local inference binary found",
        );
        let gguf_model = file_status(
            self.gguf_model.as_deref(),
            "set XINGSHU_LOCAL_AI_GGUF to the Qwen GGUF model file",
            "GGUF model file found",
        );
        let lora_adapter = file_status(
            self.lora_adapter.as_deref(),
            "set XINGSHU_LOCAL_AI_LORA to the LoRA adapter file",
            "LoRA adapter file found",
        );
        let training = file_status(
            self.training_script.as_deref(),
            "set XINGSHU_LOCAL_AI_TRAIN_SCRIPT to the PEFT/LoRA training entrypoint",
            "LoRA training entrypoint found",
        );
        let conversion = file_status(
            self.conversion_script.as_deref(),
            "set XINGSHU_LOCAL_AI_CONVERT_SCRIPT to the GGUF conversion entrypoint",
            "GGUF conversion entrypoint found",
        );
        let rk_validation = file_status(
            self.rk_report.as_deref(),
            "set XINGSHU_LOCAL_AI_RK_REPORT to the RK latency validation report",
            "RK validation report found",
        );

        let ready_for_inference = self.enabled && inference_bin.available && gguf_model.available;
        let ready_for_training = self.enabled && training.available && conversion.available;
        let mut missing = Vec::new();
        if !self.enabled {
            missing.push("XINGSHU_LOCAL_AI_ENABLED".to_string());
        }
        collect_missing(&mut missing, "XINGSHU_LOCAL_AI_BIN", &inference_bin);
        collect_missing(&mut missing, "XINGSHU_LOCAL_AI_GGUF", &gguf_model);
        collect_missing(&mut missing, "XINGSHU_LOCAL_AI_LORA", &lora_adapter);
        collect_missing(&mut missing, "XINGSHU_LOCAL_AI_TRAIN_SCRIPT", &training);
        collect_missing(&mut missing, "XINGSHU_LOCAL_AI_CONVERT_SCRIPT", &conversion);
        collect_missing(&mut missing, "XINGSHU_LOCAL_AI_RK_REPORT", &rk_validation);

        LocalAiStatus {
            enabled: self.enabled,
            runtime: self.runtime.clone(),
            model_family: self.model_family.clone(),
            inference: merge_inference_status(inference_bin, gguf_model),
            lora_adapter,
            training,
            conversion,
            rk_validation,
            ready_for_inference,
            ready_for_training,
            missing,
        }
    }
}

fn merge_inference_status(
    inference_bin: LocalAiStageStatus,
    gguf_model: LocalAiStageStatus,
) -> LocalAiStageStatus {
    let configured = inference_bin.configured && gguf_model.configured;
    let available = inference_bin.available && gguf_model.available;
    let path = match (&inference_bin.path, &gguf_model.path) {
        (Some(bin), Some(model)) => Some(format!("{bin} | {model}")),
        (Some(bin), None) => Some(bin.clone()),
        (None, Some(model)) => Some(model.clone()),
        (None, None) => None,
    };
    let detail = if available {
        "local GGUF inference boundary is configured".to_string()
    } else if !inference_bin.available && !gguf_model.available {
        "inference binary and GGUF model are missing".to_string()
    } else if !inference_bin.available {
        inference_bin.detail
    } else {
        gguf_model.detail
    };
    LocalAiStageStatus {
        configured,
        available,
        path,
        detail,
    }
}

fn collect_missing(missing: &mut Vec<String>, name: &str, status: &LocalAiStageStatus) {
    if !status.available {
        missing.push(name.to_string());
    }
}

fn file_status(
    path: Option<&Path>,
    missing_detail: &'static str,
    available_detail: &'static str,
) -> LocalAiStageStatus {
    match path {
        Some(path) if path.is_file() => LocalAiStageStatus {
            configured: true,
            available: true,
            path: Some(path.display().to_string()),
            detail: available_detail.to_string(),
        },
        Some(path) => LocalAiStageStatus {
            configured: true,
            available: false,
            path: Some(path.display().to_string()),
            detail: "configured path does not exist or is not a file".to_string(),
        },
        None => LocalAiStageStatus {
            configured: false,
            available: false,
            path: None,
            detail: missing_detail.to_string(),
        },
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_bool(name: &str) -> bool {
    env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_reports_not_ready_without_pretending_lora_exists() {
        let _guard = EnvGuard::clear(&[
            "XINGSHU_LOCAL_AI_ENABLED",
            "XINGSHU_LOCAL_AI_BIN",
            "XINGSHU_LOCAL_AI_GGUF",
            "XINGSHU_LOCAL_AI_LORA",
            "XINGSHU_LOCAL_AI_TRAIN_SCRIPT",
            "XINGSHU_LOCAL_AI_CONVERT_SCRIPT",
            "XINGSHU_LOCAL_AI_RK_REPORT",
        ]);
        let status = LocalAiStatus::from_env();
        assert!(!status.enabled);
        assert!(!status.ready_for_inference);
        assert!(!status.ready_for_training);
        assert!(status
            .missing
            .contains(&"XINGSHU_LOCAL_AI_GGUF".to_string()));
        assert_eq!(status.model_family, "Qwen3.5-2B");
    }

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn clear(names: &[&'static str]) -> Self {
            let saved = names
                .iter()
                .map(|name| {
                    let value = env::var(name).ok();
                    env::remove_var(name);
                    (*name, value)
                })
                .collect();
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in self.saved.drain(..) {
                if let Some(value) = value {
                    env::set_var(name, value);
                } else {
                    env::remove_var(name);
                }
            }
        }
    }
}
