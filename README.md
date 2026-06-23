# TALOS (Tensor Allocation Layer for Onboard Systems)
**Safety-aware resource arbitration and execution isolation for edge robotics. Built for environments where GPU starvation is not an option.**

[![Rust](https://img.shields.io/badge/Rust-Control_Plane-orange.svg)]()
[![C++](https://img.shields.io/badge/C++20-Data_Plane-blue.svg)]()
[![CUDA](https://img.shields.io/badge/TensorRT-Zero_Copy-green.svg)]()
[![Target](https://img.shields.io/badge/Target-Orin_Nano_8GB-lightgrey.svg)]()

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
