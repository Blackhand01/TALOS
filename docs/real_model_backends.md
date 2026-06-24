# Real Model Backends

TALOS can wrap real inference backends without letting the backend participate in scheduling decisions.

The control path remains:

```text
telemetry -> state machine -> admission -> GPU lease -> external model backend -> JSONL log
```

## Supported Backends

| Backend | Command | Requirement |
| --- | --- | --- |
| `tensorrt-engine` | `trtexec --loadEngine=...` | prebuilt TensorRT `.engine` file on the Jetson |
| `tensorrt-onnx` | `trtexec --onnx=...` | ONNX model plus TensorRT tooling |
| `smolvlm-cuda` | `python3 scripts/talos_smolvlm_probe.py` | PyTorch, Transformers, PIL, CUDA, SmolVLM model availability |

## Fast Path: Tiny Vision ONNX

This is the recommended first real-model validation. It creates a small static
ONNX convolutional graph directly on the Jetson and runs it through TensorRT
inside the normal TALOS control path.

```bash
make jetson-run-tiny-vision-trt
```

What this proves:

```text
real ONNX graph -> TensorRT trtexec -> TALOS admission -> GPU lease -> JSONL telemetry log
```

It does not prove model accuracy; it proves real TensorRT integration under
TALOS control.

## TensorRT Engine

```bash
make jetson-run-real-model REAL_MODEL_ARGS='--backend tensorrt-engine --model /home/ste/models/vision.engine --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-trt-engine.jsonl --no-csv'
```

## ONNX Through TensorRT

```bash
make jetson-run-trt-onnx TRT_ONNX_ARGS='--backend tensorrt-onnx --model /home/ste/models/vision.onnx --backend-arg --fp16 --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-trt-onnx.jsonl --no-csv'
```

## SmolVLM CUDA

```bash
make jetson-run-smolvlm SMOLVLM_ARGS='--backend smolvlm-cuda --model HuggingFaceTB/SmolVLM-256M-Instruct --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-smolvlm-cuda.jsonl --no-csv'
```

## Logging Contract

Every run writes the standard TALOS fields plus real-model fields:

```text
real_model_backend
real_model_name
real_model_exit_code
real_model_peak_cuda_mb
runtime_ok
latency_ms
telemetry_source
telemetry_valid
memory_usage_percent
temperature_c
gpu_utilization
```

If the backend is missing, TALOS still records the admitted task and writes `runtime_ok=false`. This is intentional: backend failures are evidence, not hidden exceptions.

## Boundary

The backend does not see queue pressure, scheduler state, task priority, or lease state. It only runs after TALOS has admitted the task and acquired the GPU lease.
