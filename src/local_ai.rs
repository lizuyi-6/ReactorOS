use std::{
    env,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wait_timeout::ChildExt;

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
    /// When set, run_inference / run_training POST to this llama.cpp
    /// HTTP endpoint instead of spawning a local binary. Operators can
    /// point the daemon at a `llama-server` instance elsewhere on the
    /// network without rebuilding the model binaries.
    http_endpoint: Option<String>,
    http_train_endpoint: Option<String>,
}

pub struct LocalAiInferenceRequest {
    pub prompt: String,
    pub max_tokens: u32,
    pub timeout: Duration,
}

pub struct LocalAiTrainingRequest {
    pub dataset: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub dry_run: bool,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiCommandReport {
    pub status: LocalAiStatus,
    pub program: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub parsed_stdout: Option<Value>,
}

impl LocalAiStatus {
    pub fn from_env() -> Self {
        LocalAiConfig::from_env().status()
    }
}

pub fn run_inference_from_env(request: LocalAiInferenceRequest) -> Result<LocalAiCommandReport> {
    LocalAiConfig::from_env().run_inference(request)
}

pub fn run_training_from_env(request: LocalAiTrainingRequest) -> Result<LocalAiCommandReport> {
    LocalAiConfig::from_env().run_training(request)
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
            http_endpoint: env::var("XINGSHU_LOCAL_AI_HTTP_URL").ok().filter(|s| !s.trim().is_empty()),
            http_train_endpoint: env::var("XINGSHU_LOCAL_AI_TRAIN_URL").ok().filter(|s| !s.trim().is_empty()),
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

    fn run_inference(&self, request: LocalAiInferenceRequest) -> Result<LocalAiCommandReport> {
        self.ensure_enabled("local AI inference")?;
        if request.prompt.trim().is_empty() {
            return Err(anyhow!("local AI inference prompt must not be empty"));
        }
        if let Some(endpoint) = self.http_endpoint.as_deref() {
            return self.run_inference_http(endpoint, &request);
        }
        let program = self.required_path("XINGSHU_LOCAL_AI_BIN", &self.inference_bin)?;
        let model = self.required_path("XINGSHU_LOCAL_AI_GGUF", &self.gguf_model)?;
        let mut args = vec![
            "-m".to_string(),
            model.display().to_string(),
            "-p".to_string(),
            request.prompt,
            "-n".to_string(),
            request.max_tokens.max(1).to_string(),
        ];
        if let Some(lora) = self.lora_adapter.as_ref().filter(|path| path.is_file()) {
            args.push("--lora".to_string());
            args.push(lora.display().to_string());
        }
        self.run_command(program, args, request.timeout)
    }

    fn run_training(&self, request: LocalAiTrainingRequest) -> Result<LocalAiCommandReport> {
        self.ensure_enabled("local LoRA training")?;
        if let Some(endpoint) = self.http_train_endpoint.as_deref() {
            return self.run_training_http(endpoint, &request);
        }
        let program = self.required_path("XINGSHU_LOCAL_AI_TRAIN_SCRIPT", &self.training_script)?;
        let model = self.required_path("XINGSHU_LOCAL_AI_GGUF", &self.gguf_model)?;
        let conversion =
            self.required_path("XINGSHU_LOCAL_AI_CONVERT_SCRIPT", &self.conversion_script)?;
        let mut args = vec![
            "--model".to_string(),
            model.display().to_string(),
            "--convert-script".to_string(),
            conversion.display().to_string(),
        ];
        if let Some(lora) = self.lora_adapter.as_ref().filter(|path| path.is_file()) {
            args.push("--lora".to_string());
            args.push(lora.display().to_string());
        }
        if let Some(dataset) = request.dataset {
            args.push("--dataset".to_string());
            args.push(dataset.display().to_string());
        }
        if let Some(output_dir) = request.output_dir {
            args.push("--output-dir".to_string());
            args.push(output_dir.display().to_string());
        }
        if request.dry_run {
            args.push("--dry-run".to_string());
        }
        self.run_command(program, args, request.timeout)
    }

    fn run_command(
        &self,
        program: PathBuf,
        args: Vec<String>,
        timeout: Duration,
    ) -> Result<LocalAiCommandReport> {
        let mut command = Command::new(&program);
        command
            .args(&args)
            .env("XINGSHU_LOCAL_AI_RUNTIME", &self.runtime)
            .env("XINGSHU_LOCAL_AI_MODEL_FAMILY", &self.model_family)
            .env_path("XINGSHU_LOCAL_AI_BIN", &self.inference_bin)
            .env_path("XINGSHU_LOCAL_AI_GGUF", &self.gguf_model)
            .env_path("XINGSHU_LOCAL_AI_LORA", &self.lora_adapter)
            .env_path("XINGSHU_LOCAL_AI_TRAIN_SCRIPT", &self.training_script)
            .env_path("XINGSHU_LOCAL_AI_CONVERT_SCRIPT", &self.conversion_script)
            .env_path("XINGSHU_LOCAL_AI_RK_REPORT", &self.rk_report)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to start local AI command {}", program.display()))?;
        let stdout = child
            .stdout
            .take()
            .map(read_pipe_on_thread)
            .ok_or_else(|| anyhow!("local AI command stdout was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .map(read_pipe_on_thread)
            .ok_or_else(|| anyhow!("local AI command stderr was not captured"))?;
        let status = match child.wait_timeout(timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_reader(stdout)?;
                let stderr = join_reader(stderr)?;
                return Err(anyhow!(
                    "local AI command exceeded timeout of {}ms: {}\nstdout: {}\nstderr: {}",
                    timeout.as_millis(),
                    program.display(),
                    truncate(&stdout),
                    truncate(&stderr)
                ));
            }
        };
        let stdout = join_reader(stdout)?;
        let stderr = join_reader(stderr)?;
        let parsed_stdout = serde_json::from_str(stdout.trim()).ok();
        let report = LocalAiCommandReport {
            status: self.status(),
            program: program.display().to_string(),
            args,
            exit_code: status.code(),
            stdout,
            stderr,
            parsed_stdout,
        };
        if !status.success() {
            return Err(anyhow!(
                "local AI command exited with {status}: {}\nstdout: {}\nstderr: {}",
                report.program,
                truncate(&report.stdout),
                truncate(&report.stderr)
            ));
        }
        Ok(report)
    }

    fn required_path(&self, name: &'static str, path: &Option<PathBuf>) -> Result<PathBuf> {
        match path {
            Some(path) if path.is_file() => Ok(path.clone()),
            Some(path) => Err(anyhow!(
                "{name} is configured but is not a readable file: {}",
                path.display()
            )),
            None => Err(anyhow!(
                "local AI assets are not ready; missing local AI asset: {name}"
            )),
        }
    }

    /// Talk to a llama.cpp HTTP server (e.g. `llama-server --model ...`).
    /// Contract: POST {endpoint}/v1/chat/completions with an OpenAI-shaped
    /// request body. The server may also expose /completion; we stick to
    /// chat/completions because that is the documented public surface.
    fn run_inference_http(
        &self,
        endpoint: &str,
        request: &LocalAiInferenceRequest,
    ) -> Result<LocalAiCommandReport> {
        let url = format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model_family,
            "messages": [
                { "role": "user", "content": request.prompt }
            ],
            "max_tokens": request.max_tokens.max(1),
            "stream": false
        });
        let body_bytes = serde_json::to_vec(&body).context("serialize llama.cpp request")?;
        let started = Instant::now();
        let response = self.http_post_json(&url, &body_bytes, request.timeout)?;
        let elapsed = started.elapsed();
        let parsed: Option<Value> = serde_json::from_slice(&response).ok();
        let stdout = String::from_utf8_lossy(&response).into_owned();
        let report = LocalAiCommandReport {
            status: self.status(),
            program: url.clone(),
            args: vec![format!("max_tokens={}", request.max_tokens)],
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            parsed_stdout: parsed,
        };
        if elapsed > request.timeout {
            return Err(anyhow!(
                "llama.cpp HTTP inference at {url} exceeded timeout of {}ms (elapsed {}ms)",
                request.timeout.as_millis(),
                elapsed.as_millis()
            ));
        }
        Ok(report)
    }

    /// Talk to a PEFT / training HTTP service. Contract: POST with
    /// {dataset, output_dir, dry_run, base_model, lora_adapter} JSON. The
    /// service is expected to return 200 + JSON {status, log_path}.
    fn run_training_http(
        &self,
        endpoint: &str,
        request: &LocalAiTrainingRequest,
    ) -> Result<LocalAiCommandReport> {
        let body = serde_json::json!({
            "base_model": self.gguf_model.as_ref().map(|p| p.display().to_string()),
            "lora_adapter": self.lora_adapter.as_ref().map(|p| p.display().to_string()),
            "dataset": request.dataset.as_ref().map(|p| p.display().to_string()),
            "output_dir": request.output_dir.as_ref().map(|p| p.display().to_string()),
            "dry_run": request.dry_run,
            "model_family": self.model_family
        });
        let body_bytes = serde_json::to_vec(&body).context("serialize training request")?;
        let response = self.http_post_json(endpoint, &body_bytes, request.timeout)?;
        let parsed: Option<Value> = serde_json::from_slice(&response).ok();
        Ok(LocalAiCommandReport {
            status: self.status(),
            program: endpoint.to_string(),
            args: vec![format!("dry_run={}", request.dry_run)],
            exit_code: Some(0),
            stdout: String::from_utf8_lossy(&response).into_owned(),
            stderr: String::new(),
            parsed_stdout: parsed,
        })
    }

    /// Minimal blocking HTTP POST with a JSON body and a timeout. This
    /// is intentionally tiny: llama.cpp's /v1/chat/completions is plain
    /// HTTP/1.1 with chunked or content-length JSON; we read the response
    /// fully before returning.
    fn http_post_json(&self, url: &str, body: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        let (scheme, rest) = url.split_once("://").ok_or_else(|| {
            anyhow!("llama.cpp endpoint must be http(s)://host:port/path; got {url}")
        })?;
        let (authority, path) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rfind(':') {
            Some(idx) if !authority.starts_with('[') => (&authority[..idx], authority[idx + 1..].to_string()),
            _ => (authority, if scheme == "https" { "443" } else { "80" }.to_string()),
        };
        let port: u16 = port.parse().with_context(|| format!("invalid port in {url}"))?;
        let stream_addr = format!("{host}:{port}");
        let started = Instant::now();
        let mut stream = TcpStream::connect_timeout(&stream_addr.as_str().parse().unwrap(), timeout)
            .with_context(|| format!("failed to connect to llama.cpp HTTP {stream_addr}"))?;
        stream.set_read_timeout(Some(timeout)).ok();
        stream.set_write_timeout(Some(timeout)).ok();
        let path_query = if path.contains('?') { path.to_string() } else { path.to_string() };
        let mut request = format!(
            "POST {path_query} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(body))
            .context("write llama.cpp request")?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .context("read llama.cpp response")?;
        if started.elapsed() > timeout {
            return Err(anyhow!(
                "llama.cpp HTTP at {stream_addr} exceeded timeout of {}ms",
                timeout.as_millis()
            ));
        }
        let text = String::from_utf8_lossy(&raw);
        let header_end = text
            .find("\r\n\r\n")
            .ok_or_else(|| anyhow!("llama.cpp HTTP response had no header terminator"))?;
        let status_line = &text[..text.find("\r\n").unwrap_or(header_end)];
        if !status_line.contains(" 200 ") {
            return Err(anyhow!(
                "llama.cpp HTTP at {stream_addr} returned non-200: {status_line}"
            ));
        }
        Ok(raw[header_end + 4..].to_vec())
    }

    fn ensure_enabled(&self, action: &str) -> Result<()> {
        if self.enabled {
            Ok(())
        } else {
            Err(anyhow!(
                "{action} is disabled; set XINGSHU_LOCAL_AI_ENABLED=true"
            ))
        }
    }
}

trait CommandEnvPathExt {
    fn env_path(&mut self, name: &str, path: &Option<PathBuf>) -> &mut Self;
}

impl CommandEnvPathExt for Command {
    fn env_path(&mut self, name: &str, path: &Option<PathBuf>) -> &mut Self {
        if let Some(path) = path {
            self.env(name, path);
        }
        self
    }
}

fn read_pipe_on_thread<T>(mut pipe: T) -> thread::JoinHandle<std::io::Result<String>>
where
    T: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = String::new();
        pipe.read_to_string(&mut output)?;
        Ok(output)
    })
}

fn join_reader(handle: thread::JoinHandle<std::io::Result<String>>) -> Result<String> {
    handle
        .join()
        .map_err(|_| anyhow!("local AI command output reader panicked"))?
        .map_err(Into::into)
}

fn truncate(value: &str) -> String {
    const LIMIT: usize = 2000;
    if value.len() <= LIMIT {
        return value.to_string();
    }
    format!("{}...<truncated>", &value[..LIMIT])
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
