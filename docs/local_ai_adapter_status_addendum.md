# Local AI Adapter Status Addendum

Date: 2026-06-06

Scope: upper-computer boundary for the PRD requirement "Qwen3.5-2B + LoRA / GGUF / RK-side latency validation".

## What is now implemented

- `src/local_ai.rs` exposes structured local AI asset readiness.
- `/api/config/summary` includes a `local_ai` object with:
  - `enabled`
  - `runtime`
  - `model_family`
  - `mode`
  - `ready_for_base_inference`
  - `ready_for_lora_inference`
  - `ready_for_inference`
  - `ready_for_training`
  - `ready_for_prd_lora`
  - `missing`
  - `model_path`, `adapter_path`, `inference_endpoint`, and `training_endpoint`
  - per-stage status for inference, LoRA adapter, training, GGUF conversion, and RK validation.
- `ready_for_base_inference` means the daemon can reach either a local GGUF binary/model pair or a configured `llama-server` style HTTP endpoint.
- `ready_for_lora_inference` means base inference plus an actual `XINGSHU_LOCAL_AI_LORA` adapter artifact.
- `ready_for_inference` is kept for API compatibility and now aliases `ready_for_lora_inference`, not merely base-model availability.
- `ready_for_prd_lora` means LoRA inference, training boundary, and RK latency report are all present. This is the only local status that may be treated as the upper-computer side of the PRD LoRA/RK evidence boundary.
- `xingshu ai model` prints the `local_ai` object beside the active provider and AI memory summary.
- `xingshu ai train --export-only` exports supervised JSONL rows from real SQLite batches, product results, sensor samples, and control events.
- `xingshu ai train` can invoke a configured local training entrypoint through `XINGSHU_LOCAL_AI_TRAIN_SCRIPT`, passing the generated dataset and optional output directory.
- `xingshu ai train` writes a machine-readable training manifest (`xingshu.local_ai.training_manifest.v1`) with training stdout metadata, parsed metrics, candidate adapter path, promotion decision, and rollback backup path when applicable.
- `xingshu ai train --promote --min-eval-score <score>` can explicitly promote a passing candidate adapter into `XINGSHU_LOCAL_AI_LORA`; it refuses promotion when the training output lacks an evaluation score, lacks a candidate adapter path, points to a missing file, or falls below the requested threshold.
- If model/training assets are not configured, `xingshu ai train` fails after dataset export with an explicit readiness error instead of pretending LoRA training happened.
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

If inference or training is served by local services instead of local binaries:

```powershell
$env:XINGSHU_LOCAL_AI_HTTP_URL='http://127.0.0.1:8080'
$env:XINGSHU_LOCAL_AI_TRAIN_URL='http://127.0.0.1:8090/train'
```

The current bridge intentionally accepts plain `http://` endpoints only. Put TLS termination in front of the local service if production transport encryption is required.

Training export smoke:

```powershell
xingshu --db C:\path\to\xingshu.sqlite3 ai train --export-only --dataset C:\path\to\lora-training-dataset.jsonl
```

Training orchestration smoke after the algorithm side provides a compatible entrypoint:

```powershell
xingshu --db C:\path\to\xingshu.sqlite3 ai train --dataset C:\path\to\lora-training-dataset.jsonl --output-dir C:\path\to\out --manifest C:\path\to\train.manifest.json --dry-run
```

Explicit promotion smoke after the training script returns an evaluated candidate adapter:

```powershell
xingshu --db C:\path\to\xingshu.sqlite3 ai train --dataset C:\path\to\lora-training-dataset.jsonl --manifest C:\path\to\train.manifest.json --promote --min-eval-score 0.8
```

## What is still missing

This is a readiness boundary, not a fake model implementation.

Still required from algorithm/hardware owners:

- Real Qwen3.5-2B weights or approved compatible local model.
- LoRA adapter artifact.
- Production PEFT/LoRA training entrypoint and final dataset contract review.
- GGUF conversion script/tooling.
- RK-side latency validation report.
- Automatic training trigger, production evaluation policy, and model promotion approval workflow for self-evolution.
