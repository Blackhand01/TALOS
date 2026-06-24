Ecco il blueprint definitivo di **TALOS** aggiornato con la correzione ingegneristica per prevenire l'heap fragmentation su sistemi embedded vincolati come la Jetson Orin Nano.

Il data model e lo strato di ingestione sono stati modificati per implementare nativamente un **Pre-allocated Ring Buffer Pool**, portando le allocazioni sull'heap nel *hot-path* dell'inferenza a zero.

---

# 🧠 TALOS — Tensor Allocation Layer for Onboard Systems (FINAL SPEC)

## Versioning Plan (UPDATED)

```text
Phase 1     → control plane + deterministic GPU lease + pool-based ingestion + C++ dummy runtime
Phase 1.5   → full observability layer (logs, traces, benchmarks)
Phase 2     → real CV pipeline (TensorRT-ready)
Phase 3     → Jetson telemetry integration (tegrastats / jtop abstraction)
Phase 4     → change detection workload (embedding diff + heuristics)
Phase 5     → VLM workload integration (quantized model gating)
Phase 6     → contention + stress + thermal simulation
Phase 7     → Jetson deployment hardening (nvpmodel, clocks, power modes)
Phase 8     → optimization (latency, memory, throughput tuning)

```

---

# 🔒 0. SYSTEM DEFINITION (hard boundary)

TALOS is a:

> **deterministic GPU admission control + execution orchestration layer for edge AI workloads on Jetson-class devices**

### TALOS is NOT:

* robotics stack
* distributed system
* training framework
* RL system
* orchestration cluster (no Kubernetes, no fleet logic)

### TALOS IS:

* GPU scheduling authority
* safety + thermal + memory arbitration layer
* execution lease manager
* inference gating system

---

# 🧩 1. CORE INVARIANT (non-negotiable)

## Single GPU Execution Rule

```text
At any time, only ONE GPU-heavy execution may be active.

```

This is enforced via:

```rust
GpuLeaseManager (global singleton)

```

---

# 🧱 2. ARCHITECTURE LAYERS (strict separation)

```text
L4 → Telemetry (observe only)
L3 → TALOS Controller (decide)
L2 → C++ Runtime (execute only)
L1 → Frame ingestion (data pool manager)

```

### Critical rule

```text
L2 MUST NOT influence L3 decisions.
L4 MUST NOT mutate state.
L1 MUST NOT bypass FrameContext contract.

```

---

# 📦 3. DATA MODEL (finalized + minimal)

## Frame Context (Heap-allocation-free Entrypoint)

> **Antal-Fragmentation Constraint:** To prevent heap fragmentation under high-frequency ingestion (20–50 tasks/sec), frames do not allocate dynamic `Vec<u8>` variables. They utilize a pre-allocated ring buffer pool allocated once at system startup.

```rust
struct FrameContext {
    id: u64,
    timestamp: u64,
    source_path: String,
    pool_slot_id: usize, // Index of the pre-allocated buffer slot (e.g., 0..4)
    payload_len: usize,  // Actual valid payload size within the static buffer
}

```

---

## Task Model

```rust
enum TaskType {
    CV_FEATURES,
    CHANGE_DETECTION,
    VLM_QUERY,
}

```

```rust
enum TaskPriority {
    HIGH,
    MEDIUM,
    LOW,
}

```

```rust
struct TaskRequest {
    task_type: TaskType,
    priority: TaskPriority,
    memory_estimate_mb: u32,
    deadline_ms: Option<u32>,
    frame: Option<FrameContext>,
}

```

---

## Decision Model

```rust
enum DecisionStatus {
    ACCEPT,
    DEFER,
    REJECT,
}

struct Decision {
    status: DecisionStatus,
    lease_duration_ms: u32,
}

```

---

# 📊 4. TELEMETRY MODEL (read-only)

Telemetry is **observational only**.

```rust
struct SystemTelemetry {
    memory_usage_percent: f32,
    temperature_c: f32,
    gpu_utilization: f32,
    queue_length: u32,
}

```

### Derived value (IMPORTANT)

```text
queue_pressure = Σ(priority weights of queued tasks)

```

NOT part of telemetry.

---

# 🔁 5. EXECUTION MODEL (STRICT)

## Correct model

```text
1. Task arrives in Rust
2. Controller computes:
   - queue_pressure
   - scheduler_state
3. Admission decision is made
4. If GPU needed → acquire lease
5. Fetch memory slice from the static Pool via pool_slot_id
6. Call C++ via spawn_blocking passing the raw pointer context
7. Wait full completion (NO preemption)
8. Release lease & recycle Pool slot

```

---

## Hard invariant

```text
Once GPU execution starts → it cannot be interrupted.

```

---

# ⚙️ 6. GPU LEASE MODEL (CRITICAL)

```rust
struct GpuLease {
    id: uuid,
}

```

### Semantics:

* RAII enforced
* single global lease
* Drop ALWAYS releases GPU
* even on panic path (must be safe unwind)

---

# 🧠 7. STATE MACHINE (simplified but correct)

```text
NORMAL
→ HIGH_LOAD
→ THROTTLE
→ DEGRADED
→ EMERGENCY

```

---

## Transition rules

### NORMAL → HIGH_LOAD

```text
queue_pressure > THRESHOLD_Q

```

### HIGH_LOAD → THROTTLE

```text
temperature_c > 80

```

### THROTTLE → DEGRADED

```text
memory_usage_percent > 85

```

---

# 📡 8. IPC / RUNTIME BOUNDARY (NO AMBIGUITY)

## ONLY mechanism

```text
cxx (in-process FFI)

```

---

## C++ runtime contract

C++ is:

* stateless
* deterministic
* non-scheduling
* non-aware of system state

```cpp
Result run(const uint8_t* buffer, size_t len);

```

Returns:

```text
{ ok: bool, latency_ms: u32 }

```

---

## Forbidden in C++

* no queues
* no task types
* no telemetry access
* no GPU scheduling logic

---

# 📥 9. INGESTION LAYER & MEMORY POOL (DTU)

```text
data/dtu_wind_turbine/**/*.JPG

```

## Rules:

* **Pre-allocated Pool Initialization:** At startup, L1 allocates a fixed ring buffer (e.g., 5 slots $\times$ 5MB each).
* **Hot-path Execution:** The ingestor overwrites bytes inside the assigned `pool_slot_id`. Heap allocation during runtime drops to absolute zero.
* Raw byte load only into the static buffer.
* No decoding logic required / no OpenCV in Phase 1.
* Frame metadata becomes `TaskRequest`.

---

# 🔥 10. SCHEDULING LOGIC (FINAL FORM)

## Rule 1 — GPU exclusivity

```text
if GPU_LEASE_ACTIVE → defer all GPU tasks

```

---

## Rule 2 — VLM gating

```text
reject VLM if memory > 85%
defer VLM if HIGH_LOAD
reject VLM if DEGRADED

```

---

## Rule 3 — priority override

```text
HIGH CV tasks allowed in DEGRADED

```

---

## Rule 4 — queue pressure model

```text
HIGH = 10
MEDIUM = 5
LOW = 1

```

---

## Rule 5 — control ordering (CRITICAL)

```text
Telemetry
 → derived queue_pressure
 → state transition
 → admission decision
 → optional GPU lease
 → execution

```

---

# 📈 11. PHASE 1.5 (IMPORTANT)

This is what makes it “hireable”.

## Mandatory outputs per task

```text
task_id
task_type
decision
queue_pressure
scheduler_state
lease_id
pool_slot_id
latency_ms
execution_time_ms

```

---

## Logging format

```text
JSONL (NOT optional)
CSV mirror (optional)

```

---

## Stress modes

### CV flood

* 20–50 tasks/sec (forces heavy buffer recycling)

### VLM burst

* high memory, low frequency

### Thermal spike simulation

* injected telemetry only

### Mixed contention

* randomized workload interleaving

---

## Why this matters (interview signal)

This is what companies like autonomous drones, inspection robotics, and edge AI startups actually evaluate:

> “Can this system remain stable and allocation-free under sustained contention?”

---

# 🧪 12. TEST CONTRACT (STRICT)

You MUST be able to prove:

* single GPU lease invariant
* **pool exhaustion handling:** if 5 slots are full and a 6th task arrives, it must block or defer deterministically without leaking memory or panicking.
* no VLM under DEGRADED
* deterministic queue_pressure $\rightarrow$ state transition
* no C++ policy leakage
* frame ingestion correctness

---

# 📁 13. FINAL STRUCTURE (CLEANED)

```text
talos/
├── core/
│   ├── scheduler.rs
│   ├── state_machine.rs
│   ├── admission.rs
│   ├── leases.rs
│   ├── telemetry.rs
│   └── pool.rs               # NEW: Pre-allocated Frame Buffer Pool
│
├── runtime/
│   ├── cv/
│   ├── change_detection/
│   ├── vlm/
│   └── server.cpp
│
├── ipc/
│   └── cxx_bridge.rs
│
├── edge_node/
│   └── main.rs
│
├── ingestion/
│   └── dtu_ingestor.rs
│
├── evaluation/
│   ├── stress_tests.py
│   ├── latency_bench.py
│   └── contention_sim.py
│
├── telemetry/
│   └── logger.rs
│
└── docs/
    ├── architecture.md
    ├── execution_model.md
    └── state_machine.md

```

---

