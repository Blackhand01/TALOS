# TALOS Architecture

TALOS is a deterministic admission-control and execution-orchestration layer for Jetson-class edge-AI workloads. It coordinates telemetry, queue pressure, thermal state, memory pressure, and GPU execution leases before runtime invocation.

## System Scope

TALOS provides:

- GPU admission control for edge inference workloads.
- Telemetry-aware scheduling decisions.
- Thermal and memory arbitration.
- Lease-based GPU execution ownership.
- Structured observability for admission and execution events.
- Rust/C++ runtime integration through a typed in-process bridge.

TALOS does not provide robotics middleware, model training, distributed scheduling, flight control, or model accuracy benchmarking.

## Control Plane

The Rust control plane owns all mutable scheduler state:

- task metadata and task type classification
- queue pressure calculation
- telemetry sampling and validation
- scheduler state transitions
- admission decisions
- deferred-task queues
- GPU lease lifetime
- JSONL/CSV observation emission
- benchmark and HITL runners

Admission decisions use a small status vocabulary:

```text
ADMIT  -> execute immediately if a GPU lease is available
DEFER  -> retain work for a later scheduling pass
REJECT -> terminate scheduling for the task under current constraints
```

`DEFER` and `REJECT` are distinct states. Deferred work remains eligible for replay after telemetry or queue conditions recover. Rejected work is recorded with a reason and is not replayed by the scheduler.

## Data Plane

The C++ runtime is a stateless execution boundary. It receives admitted payload data, performs runtime computation, and returns a result to Rust. It does not own:

- scheduler state
- telemetry readers
- queue state
- admission thresholds
- GPU lease state
- retry policy
- deferred-task queues

This boundary keeps runtime code replaceable by CUDA, TensorRT, OpenCV, or vendor-specific C/C++ acceleration without moving policy decisions into the data plane.

## Rust/C++ Bridge

The in-process boundary uses `cxx`. The bridge connects Rust-owned task execution to C++ runtime functions through a typed ABI instead of text serialization or an untyped plugin interface.

Execution path:

```text
Rust task metadata
Rust admission policy
Rust GPU lease acquisition
Rust payload view
cxx bridge
C++ runtime execution
C++ result
Rust observation logging
Rust GPU lease release
```

Bridge properties:

- Rust remains the scheduling owner.
- C++ receives only the data required for execution.
- The runtime cannot enqueue hidden work.
- Boundary types are checked at build time.
- Latency measurement remains anchored in the Rust control plane.

## IPC And Process Boundaries

The primary runtime path is in-process Rust/C++ FFI through `cxx`. TALOS also includes external model adapter paths for TensorRT/ONNX and model validation. These adapters execute outside the core runtime boundary and are treated as integration paths, not scheduler owners.

The repository separates these concerns:

```text
core/        Rust control plane
ipc/         Rust cxx bridge declaration
runtime/     C++ execution implementation
real_model/  external backend adapter runner
hitl/        hardware-in-the-loop runner
evaluation/  simulation-in-the-loop benchmarks
```

External processes may execute model tooling, but admission, queueing, and telemetry gates remain in Rust.

## Memory Management

Rust owns scheduler data structures, task metadata, telemetry samples, observations, and GPU lease objects. C++ receives execution inputs across the bridge and returns result values without retaining scheduler-owned references.

The GPU lease model is RAII-based:

```text
acquire lease -> execute admitted GPU-heavy work -> drop lease
```

The control-plane invariant is:

```text
Only one GPU-heavy execution may hold an active lease at a time.
```

The invariant reduces unbounded memory spikes and latency jitter on constrained Jetson devices. Higher-throughput execution strategies can be added by changing lease policy, but the current implementation prioritizes deterministic ownership.

## Telemetry Model

Telemetry is read-only input to admission control. TALOS supports:

- `synthetic` telemetry for deterministic SITL tests and fault injection.
- Linux `sysfs` telemetry for temperature and memory state.
- `tegrastats` telemetry for Jetson thermal, GPU, power, and memory signals.
- optional `jtop` integration on supported Jetson systems.

When a telemetry sample fails, the control plane can retain the last valid sample and emit `telemetry_valid=false`. Telemetry validity is recorded with each observation so scheduling decisions can be audited against sensor state.

## Queue Pressure

Queue pressure is logical backlog pressure derived from TALOS-owned queued tasks. It is separate from hardware telemetry.

Default weights:

```text
HIGH   = 10
MEDIUM = 5
LOW    = 1
```

Separating logical queue pressure from physical telemetry keeps admission decisions inspectable. Thermal throttling, memory pressure, and backlog pressure are independent scheduler inputs.

## VLM Admission Policy

VLM work is scheduled as deferrable workload. TALOS can defer or reject VLM tasks based on:

- payload size
- hard memory pressure
- soft memory pressure
- VLM-specific temperature gate
- high-load scheduler state
- throttle or degraded scheduler state
- active CV burst
- active GPU lease

After cooldown or resource recovery, deferred VLM work can be replayed through the same admission path.

## Observability

TALOS emits structured observations for scheduler decisions and runtime execution. Important fields include:

- task id
- task type
- admission decision
- decision reason
- queue pressure
- scheduler state
- telemetry source
- telemetry validity
- temperature
- memory usage
- GPU utilization
- lease id
- admission latency
- runtime execution time
- backend metadata

JSONL is the primary event format. CSV output is available for analysis workflows that require tabular data.

## SITL And HITL

Simulation-in-the-loop and hardware-in-the-loop runs are separate validation modes.

SITL validates deterministic policy behavior with injected thermal spikes, injected memory pressure, contention patterns, and regression tests.

HITL validates hardware integration with Jetson telemetry, CUDA or CPU stress workloads, runtime execution, defer/replay behavior, and generated run evidence.

## Design Tradeoffs

| Decision | Technical Effect | Constraint |
| --- | --- | --- |
| Rust control plane | explicit ownership, typed state transitions, RAII leases | less direct access to ML runtime libraries |
| C++ data plane | compatible with CUDA, TensorRT, OpenCV, and vendor runtimes | requires disciplined FFI boundaries |
| `cxx` bridge | typed in-process contract | less dynamic than a plugin system |
| single GPU lease | deterministic GPU-heavy execution ownership | lower maximum concurrency |
| JSONL-first logging | event-level auditability | larger output volume |
| separate SITL/HITL runners | clean validation boundaries | more operational commands |
