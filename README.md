# TALOS (Tensor Allocation Layer for Onboard Systems)
**Safety-aware resource arbitration and execution isolation for edge robotics. Built for environments where GPU starvation is not an option.**

[![Rust](https://img.shields.io/badge/Rust-Control_Plane-orange.svg)]()
[![C++](https://img.shields.io/badge/C++20-Data_Plane-blue.svg)]()
[![CUDA](https://img.shields.io/badge/TensorRT-Zero_Copy-green.svg)]()
[![Target](https://img.shields.io/badge/Target-Orin_Nano_8GB-lightgrey.svg)]()

## Jetson Synchronization

Use the Makefile to sync and validate TALOS on the connected Jetson Orin Nano. The default target is `ste@192.168.55.1:/home/ste/TALOS`.

```bash
make jetson-ping
make jetson-deps-check
make jetson-network-check
make jetson-install-rust   # if only Rust/Cargo are missing
make jetson-install-deps   # if C++ tools, curl, or Python are also missing
make jetson-deps-check
make jetson-update
```

For the common path, use only `make jetson-update`: it starts or reuses a persistent SSH connection, syncs once, bootstraps Rust/Cargo if missing, then runs tests, hardening status, Phase 6, and Phase 8. Use `make jetson-ssh-stop` only when you want to close the persistent SSH socket.

`jetson-update` intentionally runs only controlled SITL benchmarks. Hardware-in-the-loop telemetry is launched separately so real, noisy Jetson readings do not contaminate the synthetic stress logs:

```bash
make jetson-run-hitl
make jetson-run-hitl-heavy
make jetson-run-thermal-soak
make jetson-run-thermal-max
make jetson-run-resource-max
make jetson-run-gpu-resource-max
```

`make jetson-sync` uses the same conservative `rsync -avhR` pattern as the previous Edge-VLA-Micro Jetson workflow: it copies the TALOS source files but does not delete remote-only files or copy the 268 MB DTU dataset. Copy the dataset only when you need `edge_node --demo-dtu` on-device:

```bash
make jetson-sync-data
```

If the previous `~/Edge-VLA-Micro` repo is still on the Jetson SSD and you want to free that space, archive it first and remove it only when intentional:

```bash
make jetson-backup-old-repo
make jetson-clean-old-repo CONFIRM=1
```

If an old TALOS directory already exists on the Jetson, inspect it first with `make jetson-status`, back it up with `make jetson-backup`, then clean only when intentional:

```bash
make jetson-clean CONFIRM=1
make jetson-sync
```

## Phase 1.5 Observability

TALOS writes one JSONL observation for every task decision or execution. The CSV mirror is enabled by default and can be disabled.

```bash
cargo run --bin edge_node -- \
  --demo-dtu data/dtu_wind_turbine \
  --max-tasks 10 \
  --log-jsonl logs/talos_tasks.jsonl \
  --log-csv logs/talos_tasks.csv
```

Stress and benchmark modes are available through `talos_bench`:

```bash
cargo run --bin talos_bench -- --mode cv-flood --tasks 100
cargo run --bin talos_bench -- --mode vlm-burst --tasks 100
cargo run --bin talos_bench -- --mode thermal-spike --tasks 100
cargo run --bin talos_bench -- --mode mixed-contention --tasks 100
```

## Phase 2 CV Runtime

The C++ runtime exposes a dedicated `run_cv_features` entrypoint. It remains stateless and policy-free, but now performs deterministic CV feature extraction over the frame payload and returns a TensorRT-ready fixed-width feature summary:

- normalized byte mean, variance, min, and max
- edge-density proxy
- entropy
- FNV-1a payload checksum

Execution observations include `runtime_ok`, `feature_dim`, `input_bytes`, `feature_checksum`, `feature_mean`, `feature_entropy`, and `feature_edge_density` in JSONL and CSV output.

## Phase 3 Jetson Telemetry

The edge node can sample telemetry from a synthetic source, `sysfs`, `tegrastats`, or `jtop`:

```bash
cargo run --bin edge_node -- \
  --demo-dtu data/dtu_wind_turbine \
  --max-tasks 10 \
  --profile hitl \
  --telemetry sysfs

cargo run --bin edge_node -- \
  --demo-dtu data/dtu_wind_turbine \
  --max-tasks 10 \
  --telemetry tegrastats

cargo run --bin edge_node -- \
  --demo-dtu data/dtu_wind_turbine \
  --max-tasks 10 \
  --telemetry jtop
```

Telemetry remains read-only. TALOS uses it only to compute scheduler state and admission decisions. `sysfs` reads `/sys/class/thermal/thermal_zone0/temp` and `/proc/meminfo`, with GPU utilization set to `0.0`; use `tegrastats` when real GPU utilization is required. If a real telemetry source is unavailable, the monitor keeps the last good sample and marks `telemetry_valid=false` in observations.

## Execution Profiles

TALOS keeps simulation-in-the-loop and hardware-in-the-loop runs separate:

- `SITL`: `talos_bench` uses synthetic telemetry for deterministic stress, fault injection, Phase 6, and Phase 8. Default logs use the `logs/sitl-*.jsonl` prefix.
- `HITL`: `talos_hitl` reads real Jetson telemetry through `sysfs` by default, optionally `tegrastats` or `jtop`, and never injects thermal faults. Default log: `logs/hitl-orinnano-baseline.jsonl`.
- `HITL heavy`: `talos_hitl --workload heavy` pushes large CV/change-detection payloads through the Rust/C++ data plane with faster telemetry sampling. This stresses CPU, memory bandwidth, FFI, and admission logging; real GPU load still requires replacing the runtime stub with TensorRT/CUDA.
- `Thermal soak`: `make jetson-run-thermal-soak` runs a mixed TALOS workload with internal CPU burn threads and live `tegrastats`. It targets 70C and stops at 78C by default. Use it only for short, supervised hardware stress runs.
- `Resource max`: `make jetson-run-resource-max` adds guarded real RAM pressure on top of CPU burn. This is the HITL run that should show VLM deferrals/rejections from real telemetry while high-priority CV continues.
- `GPU resource max`: `make jetson-run-gpu-resource-max` builds a local CUDA burn helper on the Jetson and runs it concurrently with TALOS resource pressure. This is the supervised run intended to push `GR3D_FREQ`, SoC power, and temperature while TALOS gates low-priority VLM from real telemetry.

```bash
cargo run --bin talos_bench -- \
  --mode phase8-optimization \
  --tasks 120 \
  --log-jsonl logs/sitl-phase8-optimization.jsonl \
  --no-csv

cargo run --bin talos_hitl -- \
  --tasks 60 \
  --telemetry sysfs \
  --log-jsonl logs/hitl-orinnano-baseline.jsonl \
  --no-csv

cargo run --bin talos_hitl -- \
  --workload heavy \
  --tasks 10000 \
  --duration-secs 60 \
  --progress-every 5 \
  --telemetry sysfs \
  --sample-ms 20 \
  --inter-task-ms 0 \
  --payload-bytes 16777216 \
  --log-jsonl logs/hitl-orinnano-heavy-60s.jsonl \
  --no-csv

make jetson-run-thermal-soak
make jetson-run-thermal-max
make jetson-run-resource-max
make jetson-run-gpu-resource-max
```

## Phase 4 Change Detection

TALOS supports a `CHANGE_DETECTION` workload built on top of the Phase 2 CV feature embedding. The C++ runtime still only extracts stateless features; Rust owns the previous embedding, computes the embedding distance, and applies the change heuristic.

```bash
cargo run --bin edge_node -- \
  --demo-dtu data/dtu_wind_turbine \
  --max-tasks 10 \
  --workload change-detection \
  --log-jsonl logs/change_detection.jsonl

cargo run --bin talos_bench -- --mode change-detection --tasks 20
```

Change observations include `change_baseline_ready`, `change_score`, and `change_detected`.

## Phase 5 VLM Workload

TALOS supports a gated `VLM_QUERY` workload using a quantized VLM profile. Admission is still decided in Rust before execution: the gate checks input size, memory pressure, thermal pressure, high-load state, degraded/throttle state, and active CV bursts.

```bash
cargo run --bin edge_node -- \
  --demo-dtu data/dtu_wind_turbine \
  --max-tasks 5 \
  --workload vlm \
  --log-jsonl logs/vlm_query.jsonl

cargo run --bin talos_bench -- --mode vlm-query --tasks 10
cargo run --bin talos_bench -- --mode vlm-burst --tasks 10
```

The C++ `run_vlm_query` entrypoint is stateless and policy-free. It currently provides a deterministic quantized-runtime stub with token count, confidence, and answer code; it can be replaced by a real INT4 VLM engine without changing admission control. VLM observations include `vlm_model`, `vlm_quantization_bits`, `vlm_gate_reason`, `vlm_output_tokens`, `vlm_confidence`, and `vlm_answer_code`.

## Phase 6 Contention And Thermal Simulation

TALOS includes a deterministic contention and thermal stress simulation. It creates concurrent workload pressure, holds GPU leases long enough to force deferrals, injects rising thermal and memory pressure, and records state transitions through the normal JSONL/CSV observability path.

```bash
cargo run --bin talos_bench -- \
  --mode phase6-contention \
  --tasks 60 \
  --log-jsonl logs/sitl-phase6-contention.jsonl \
  --no-csv
```

The summary reports `high_load_samples`, `throttle_samples`, and `degraded_samples` in addition to admission, rejection, defer, VLM, and change-detection counters. On Jetson Orin Nano, this remains a SITL validation run because thermal and memory pressure are injected. Use `make jetson-run-hitl` when you want live telemetry instead of injected thermal simulation.

## Phase 7 Jetson Deployment Hardening

TALOS includes a Jetson hardening CLI for repeatable benchmark setup. By default it only prints the plan; applying changes requires an explicit `--apply`.

```bash
cargo run --bin jetson_harden
cargo run --bin jetson_harden -- --status
cargo run --bin jetson_harden -- --apply --mode 0
cargo run --bin jetson_harden -- --restore-clocks --apply
```

On the connected Orin Nano, use the Makefile wrappers:

```bash
make jetson-harden-status
make jetson-harden-plan
make jetson-harden-apply
make jetson-run-phase6
make jetson-harden-restore
```

The default hardening plan sets `nvpmodel` mode `0`, runs `jetson_clocks`, then records release, architecture, power mode, clock state, and a short `tegrastats` sample. Override the power profile with `JETSON_HARDEN_ARGS='--mode N'`; use `--no-nvpmodel` or `--no-clocks` for partial plans.

## Phase 8 Optimization

TALOS includes an optimization pass for latency, memory, and throughput tuning. The benchmark aggregates execution p50/p95, runtime p95, throughput, admission/defer/reject rates, peak memory, peak temperature, and queue pressure, then emits deterministic recommendations.

```bash
cargo run --bin talos_bench -- \
  --mode phase8-optimization \
  --tasks 120 \
  --log-jsonl logs/sitl-phase8-optimization.jsonl \
  --no-csv

make bench-phase8
make jetson-run-phase8
make jetson-run-hitl
make jetson-run-hitl-heavy
make jetson-run-thermal-soak
```

Recommendations are intentionally operational rather than self-mutating: examples include `apply_jetson_hardening`, `lower_vlm_soft_memory_gate`, `reduce_input_resolution`, `increase_pool_slots`, `reduce_source_rate`, and `investigate_runtime_backend`.
