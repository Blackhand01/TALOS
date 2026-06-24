# TALOS Command Guide

This file contains the commands needed to reproduce the project locally and on the connected Jetson Orin Nano.

Default Jetson target:

```text
ste@192.168.55.1:/home/ste/TALOS
```

## Local Validation

Run tests:

```bash
make test
```

Generate README assets and metrics:

```bash
make report
```

Run local SITL benchmarks:

```bash
make bench-phase6
make bench-phase8
```

Run local HITL only if the host exposes compatible Linux `/sys` telemetry:

```bash
make hitl-baseline
make hitl-heavy
```

## Jetson Setup

Check connectivity:

```bash
make jetson-ping
```

Start one persistent SSH session to avoid repeated passwords:

```bash
make jetson-ssh-start
```

Check dependencies:

```bash
make jetson-deps-check
```

Install Rust if needed:

```bash
make jetson-install-rust
```

Install broader system dependencies if needed:

```bash
make jetson-install-deps
```

Sync source code:

```bash
make jetson-sync
```

Sync DTU dataset only when needed:

```bash
make jetson-sync-data
```

Stop the persistent SSH session:

```bash
make jetson-ssh-stop
```

## Standard Jetson Validation

Run the normal one-command validation:

```bash
make jetson-update
```

This performs:

```text
sync
Rust/Cargo bootstrap if missing
cargo test
Jetson hardening status
SITL Phase 6
SITL Phase 8
```

`jetson-update` intentionally does not run HITL stress. HITL uses real hardware telemetry and should be launched explicitly.

## Jetson Hardening

Show current device status:

```bash
make jetson-harden-status
```

Show hardening plan:

```bash
make jetson-harden-plan
```

Apply max-performance benchmark setup:

```bash
make jetson-harden-apply
```

Restore clocks:

```bash
make jetson-harden-restore
```

Fan helpers:

```bash
make jetson-fan-status
make jetson-fan-set CONFIRM=1 JETSON_FAN_PWM=90
make jetson-fan-max CONFIRM=1
```

Use fan controls carefully. They write to Jetson sysfs fan PWM nodes through `sudo`.

## HITL Runs

Baseline real telemetry:

```bash
make jetson-run-hitl
```

Heavy Rust/C++ data-plane workload:

```bash
make jetson-run-hitl-heavy
```

Thermal soak:

```bash
make jetson-run-thermal-soak
```

RAM and queue-pressure run:

```bash
make jetson-run-resource-max
```

GPU resource pressure run:

```bash
make jetson-run-gpu-resource-max
```

VLM thermal defer without recovery:

```bash
make jetson-run-vlm-defer-demo VLM_DEFER_TEMP_C=50
```

VLM thermal defer with cooldown and replay:

```bash
make jetson-run-vlm-defer-recovery VLM_DEFER_TEMP_C=50
```

Expected recovery evidence:

```text
vlm_deferred > 0
recovery_cooling_started ...
deferred_replayed == vlm_deferred
vlm_replayed == vlm_deferred
rejected=0
```

## Real Model Paths

Generate a tiny ONNX model and run it through TensorRT:

```bash
make jetson-run-tiny-vision-trt
```

Run an existing TensorRT engine:

```bash
make jetson-run-real-model REAL_MODEL_ARGS='--backend tensorrt-engine --model /home/ste/models/vision.engine --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-trt-engine.jsonl --no-csv'
```

Run an ONNX model through `trtexec`:

```bash
make jetson-run-trt-onnx TRT_ONNX_ARGS='--backend tensorrt-onnx --model /home/ste/models/vision.onnx --backend-arg --fp16 --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-trt-onnx.jsonl --no-csv'
```

Check SmolVLM dependencies:

```bash
make jetson-check-smolvlm-deps
```

Install JetPack-compatible PyTorch for SmolVLM if CUDA is unavailable:

```bash
make jetson-install-jetpack-torch
```

Install generic SmolVLM Python dependencies:

```bash
make jetson-install-smolvlm-python-deps
```

Run DTU defect descriptions with SmolVLM:

```bash
make jetson-run-dtu-smolvlm-defects
```

Limit the run for a quick smoke test:

```bash
make jetson-run-dtu-smolvlm-defects DTU_SMOLVLM_ARGS='--annotations data/test-HR.json --image-root data/dtu_wind_turbine --prefer-folder "Nordtank 2018" --max-images 5 --max-new-tokens 32 --output logs/hitl-dtu-smolvlm-defects-5.jsonl --answers-md logs/hitl-dtu-smolvlm-defects-5.md --crop-dir tmp/dtu_smolvlm_defects'
```

Dry-run DTU annotation/image mapping without loading the VLM:

```bash
make dtu-smolvlm-defects-dry-run
```

## Logs And Reports

Common output locations:

```text
logs/*.jsonl
logs/*.csv
logs/jetson/*.jsonl
logs/jetson/*.log
docs/metrics_report.md
docs/metrics_summary.json
docs/assets/*.svg
docs/assets/*.gif
```

Pull Jetson logs if needed:

```bash
./scripts/pull_jetson_logs.sh
```

Regenerate report assets after adding new structured run evidence:

```bash
make report
```

## Cleanup

Inspect remote project:

```bash
make jetson-status
```

Back up current remote TALOS:

```bash
make jetson-backup
```

Clean remote TALOS only when intentional:

```bash
make jetson-clean CONFIRM=1
```

Back up previous Edge-VLA-Micro repo:

```bash
make jetson-backup-old-repo
```

Remove previous Edge-VLA-Micro repo only when intentional:

```bash
make jetson-clean-old-repo CONFIRM=1
```
