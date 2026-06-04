# Local AI Adapter Status Addendum

Date: 2026-06-04

Scope: upper-computer boundary for the PRD requirement "Qwen3.5-2B + LoRA / GGUF / RK-side latency validation".

## What is now implemented

- `src/local_ai.rs` exposes structured local AI asset readiness.
- `/api/config/summary` includes a `local_ai` object with:
  - `enabled`
  - `runtime`
  - `model_family`
  - `ready_for_inference`
  - `ready_for_training`
  - `missing`
  - per-stage status for inference, LoRA adapter, training, GGUF conversion, and RK validation.
- `xingshu ai model` prints the `local_ai` object beside the active provider and AI memory summary.
- `xingshu ai train` remains a negative readiness check and lists missing local model/training assets.
- The HMI shows Local Model Boundary / Local Qwen LoRA status in the AI, Settings, and Modbus integration views.

## Environment contract

```powershell
$env:XINGSHU_LOCAL_AI_ENABLED='true'
$env:XINGSHU_LOCAL_AI_BIN='C:\path\to\llama-cli.exe'
$env:XINGSHU_LOCAL_AI_GGUF='C:\path\to\qwen3.5-2b.gguf'
$env:XINGSHU_LOCAL_AI_LORA='C:\path\to\adapter.gguf'
$env:XINGSHU_LOCAL_AI_TRAIN_SCRIPT='C:\path\to\train_lora.py'
$env:XINGSHU_LOCAL_AI_CONVERT_SCRIPT='C:\path\to\convert_to_gguf.py'
$env:XINGSHU_LOCAL_AI_RK_REPORT='C:\path\to\rk_latency_report.md'
```

## What is still missing

This is a readiness boundary, not a fake model implementation.

Still required from algorithm/hardware owners:

- Real Qwen3.5-2B weights or approved compatible local model.
- LoRA adapter artifact.
- PEFT/LoRA training entrypoint and dataset contract.
- GGUF conversion script/tooling.
- RK-side latency validation report.
- A daemon-side training/inference execution API after the above artifacts are available.
