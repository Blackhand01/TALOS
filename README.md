# TALOS

**Tensor Allocation Layer for Onboard Systems**

TALOS is a safety-aware inference orchestration layer for Jetson-class edge AI systems. It decides which AI workloads may run when GPU, memory, temperature, and queue pressure compete on a constrained device.

[![Rust](https://img.shields.io/badge/Rust-Control_Plane-orange.svg)]()
[![C++](https://img.shields.io/badge/C++20-Data_Plane-blue.svg)]()
[![CUDA](https://img.shields.io/badge/Jetson-Orin_Nano-lightgrey.svg)]()
[![HITL](https://img.shields.io/badge/HITL-Validated-green.svg)]()

## The Problem

On an edge robot, inference is not just model execution. A realistic system may need to run:

- CV perception for navigation or inspection.
- Change detection over image sequences.
- VLM queries from an operator.
- Telemetry monitoring.
- Logging and benchmark instrumentation.

If all of that competes for the same embedded GPU without a scheduling authority, the failure mode is usually not elegant. Latency rises, memory pressure grows, thermal headroom shrinks, and non-critical workloads can starve critical perception.

TALOS solves the control-plane part of that problem:

```text
critical CV / change detection first
VLM is useful but deferrable
real telemetry gates execution
one GPU-heavy lease at a time
every decision is logged
```

## Demo Story

Imagine a wind-turbine inspection robot running on a Jetson Orin Nano.

The robot continuously performs CV and change detection on blade images. An operator also asks VLM-style questions about defects. When the Jetson gets hot, TALOS defers low-priority VLM work, keeps perception moving, then replays the VLM queue after cooldown.

![TALOS defer and recovery animation](docs/assets/talos_defer_recovery.gif)

## Hardware Evidence

Latest HITL recovery run on a real Jetson Orin Nano:

```text
unique_tasks=240
vlm_deferred=36
vlm_replayed=36/36
rejected=0
peak_temperature_c=51.312
high_load_samples=130
```

This run used real `sysfs` telemetry and a VLM thermal gate at `50C`. TALOS deferred VLM while hot, entered cooldown, and replayed every deferred VLM task.

![HITL defer recovery timeline](docs/assets/hitl_defer_recovery_timeline.svg)

Additional hardware runs show GPU and memory-pressure gating:

![Hardware summary](docs/assets/hardware_summary.svg)

Full generated metrics:

- [docs/metrics_report.md](docs/metrics_report.md)
- [docs/metrics_summary.json](docs/metrics_summary.json)

Regenerate report assets:

```bash
make report
```

## Architecture Snapshot

![TALOS architecture](docs/assets/talos_architecture.svg)

TALOS separates policy from execution:

- Rust owns admission control, telemetry, queueing, state transitions, leases, logging, and benchmark runners.
- C++ owns the in-process runtime boundary for stateless execution.
- Python is used only for tooling, model probes, dataset handling, and report generation.
- Telemetry is read-only.
- Backends do not make scheduling decisions.

Detailed system design:

- [Architecture.md](Architecture.md)

## What Is Real

Real in this repository:

- Rust admission controller.
- Deterministic queue pressure model.
- RAII GPU lease manager.
- Real Jetson `sysfs` and `tegrastats` telemetry paths.
- HITL thermal defer and replay on Jetson Orin Nano.
- JSONL/CSV observability per decision and execution.
- Rust/C++ `cxx` runtime boundary.
- SmolVLM CUDA defect-description path over DTU wind-turbine crops.
- TensorRT/ONNX adapter path through `trtexec`.

Still prototype or helper:

- CUDA burn and CPU burn create hardware pressure; they are not mission workloads.
- The C++ CV runtime is mission-like feature extraction, not a trained detector.
- SmolVLM output quality depends on the selected model and prompt.
- SITL Phase 6/8 inject faults by design and must not be confused with HITL evidence.

## Repository Map

```text
core/          Rust control plane: admission, scheduler, telemetry, leases
runtime/       C++ stateless runtime boundary
ipc/           cxx bridge
edge_node/     Edge demo runner
hitl/          Hardware-in-the-loop runner
evaluation/    SITL benchmarks
deployment/    Jetson hardening helper
scripts/       Sync, reporting, model probes, dataset tools
docs/assets/   Generated README figures and GIF
reports/       Structured hardware-run evidence
data/          DTU metadata and optional image dataset
```

## Reproduce

Start with:

```bash
make test
make report
```

On the Jetson:

```bash
make jetson-update
make jetson-run-vlm-defer-recovery VLM_DEFER_TEMP_C=50
```

All operational commands are documented in:

- [command.md](command.md)

## Why This Matters To Edge AI Teams

TALOS demonstrates that the author can reason about inference as a systems problem:

- explicit control-plane/data-plane separation
- deterministic admission decisions
- resource ownership instead of accidental concurrency
- hardware telemetry integration
- graceful degradation and recovery
- structured evidence for every decision
- practical Rust/C++/Python boundary management

The most important claim is not that TALOS can run a model.

The important claim is:

```text
TALOS knows when a model should not run yet.
```
