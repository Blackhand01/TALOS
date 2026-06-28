# TALOS

TALOS is an edge-AI orchestration framework in Rust/C++ for NVIDIA Jetson-class systems with hardware-in-the-loop thermal, memory, and GPU admission control.

## Key Features

- Rust control plane for admission control, queue pressure accounting, scheduler state, telemetry sampling, and observability.
- C++20 data plane behind a typed `cxx` bridge for stateless runtime execution.
- Deterministic `ADMIT`, `DEFER`, and `REJECT` decisions for CV, change-detection, and VLM workloads.
- RAII GPU lease management to restrict GPU-heavy execution to a single active lease.
- Jetson telemetry support through Linux `sysfs`, `tegrastats`, and optional `jtop` integration.
- Hardware-in-the-loop workloads for baseline telemetry, heavy runtime pressure, thermal soak, memory pressure, GPU pressure, and VLM defer/replay recovery.
- CUDA stress helper for controlled GPU load generation.
- TensorRT and ONNX validation paths through external model adapters.
- JSONL and CSV observability for scheduler decisions, telemetry state, queue pressure, lease metadata, and runtime latency.
- Generated metrics reports and SVG assets for reproducible run summaries.

## Architecture Overview

TALOS separates scheduling policy from runtime execution. The Rust control plane owns telemetry ingestion, admission policy, queue state, scheduler transitions, GPU leases, and structured logs. The C++ runtime receives admitted payloads through a narrow typed bridge and returns execution results without access to scheduler state.

Architecture diagrams are available in [`docs/assets/talos_architecture.svg`](docs/assets/talos_architecture.svg), [`docs/assets/admission_policy.svg`](docs/assets/admission_policy.svg), [`docs/assets/hardware_summary.svg`](docs/assets/hardware_summary.svg), and [`docs/assets/hitl_defer_recovery_timeline.svg`](docs/assets/hitl_defer_recovery_timeline.svg). The full technical design is documented in [`docs/architecture.md`](docs/architecture.md).

## Quick Start / Usage

Default Jetson target:

```text
ste@192.168.55.1:/home/ste/TALOS
```

Run local tests:

```bash
make test
```

Generate metrics and documentation assets:

```bash
make report
```

Run local SITL benchmarks:

```bash
make bench-phase6
make bench-phase8
```

Run local HITL workloads only on systems that expose compatible Linux `/sys` telemetry:

```bash
make hitl-baseline
make hitl-heavy
```

Check Jetson connectivity:

```bash
make jetson-ping
```

Start and stop a persistent SSH control connection:

```bash
make jetson-ssh-start
make jetson-ssh-stop
```

Check Jetson dependencies:

```bash
make jetson-deps-check
```

Install the Rust toolchain on the Jetson if required:

```bash
make jetson-install-rust
```

Install Jetson build dependencies:

```bash
make jetson-install-deps
```

Sync source code to the Jetson:

```bash
make jetson-sync
```

Sync the DTU dataset when required by model probes:

```bash
make jetson-sync-data
```

Run the standard Jetson validation sequence:

```bash
make jetson-update
```

`jetson-update` performs source synchronization, Rust/Cargo bootstrap when missing, `cargo test`, Jetson hardening status checks, and SITL Phase 6/Phase 8 benchmark runs. HITL stress workloads are launched explicitly.

Show Jetson hardening status and available changes:

```bash
make jetson-harden-status
make jetson-harden-plan
```

Apply and restore max-performance benchmark settings:

```bash
make jetson-harden-apply
make jetson-harden-restore
```

Control Jetson fan PWM when required for controlled thermal experiments:

```bash
make jetson-fan-status
make jetson-fan-set CONFIRM=1 JETSON_FAN_PWM=90
make jetson-fan-max CONFIRM=1
```

Run real-model validation paths:

```bash
make jetson-run-tiny-vision-trt
make jetson-run-real-model REAL_MODEL_ARGS='--backend tensorrt-engine --model /home/ste/models/vision.engine --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-trt-engine.jsonl --no-csv'
make jetson-run-trt-onnx TRT_ONNX_ARGS='--backend tensorrt-onnx --model /home/ste/models/vision.onnx --backend-arg --fp16 --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-trt-onnx.jsonl --no-csv'
```

Check and install SmolVLM dependencies:

```bash
make jetson-check-smolvlm-deps
make jetson-install-jetpack-torch
make jetson-install-smolvlm-python-deps
```

Run DTU defect-description probes:

```bash
make jetson-run-dtu-smolvlm-defects
make jetson-run-dtu-smolvlm-defects DTU_SMOLVLM_ARGS='--annotations data/test-HR.json --image-root data/dtu_wind_turbine --prefer-folder "Nordtank 2018" --max-images 5 --max-new-tokens 32 --output logs/hitl-dtu-smolvlm-defects-5.jsonl --answers-md logs/hitl-dtu-smolvlm-defects-5.md --crop-dir tmp/dtu_smolvlm_defects'
make dtu-smolvlm-defects-dry-run
```

Pull Jetson logs and regenerate reports:

```bash
./scripts/pull_jetson_logs.sh
make report
```

Inspect and clean the remote Jetson workspace:

```bash
make jetson-status
make jetson-backup
make jetson-clean CONFIRM=1
```

## Reproducibility

Hardware-in-the-loop runs require an NVIDIA Jetson target with compatible Linux telemetry paths, SSH access from the host, Rust/Cargo, a C++ compiler, Python 3, and Jetson telemetry tooling. The default target can be overridden with `JETSON_USER`, `JETSON_ADDR`, and `JETSON_DIR`.

Run the baseline telemetry workload:

```bash
make jetson-run-hitl
```

Run the heavy Rust/C++ data-plane workload:

```bash
make jetson-run-hitl-heavy
```

Run thermal and resource-pressure workloads:

```bash
make jetson-run-thermal-soak
make jetson-run-resource-max
make jetson-run-gpu-resource-max
```

Run VLM thermal defer without recovery:

```bash
make jetson-run-vlm-defer-demo VLM_DEFER_TEMP_C=50
```

Run VLM thermal defer with cooldown and replay:

```bash
make jetson-run-vlm-defer-recovery VLM_DEFER_TEMP_C=50
```

Expected recovery fields in the generated JSONL/log output:

```text
vlm_deferred > 0
recovery_cooling_started
deferred_replayed == vlm_deferred
vlm_replayed == vlm_deferred
rejected=0
```

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
