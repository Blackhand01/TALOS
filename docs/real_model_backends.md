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

This single-task probe proves that TALOS can admit and wrap a real VLM process,
but it is not yet a defect-description benchmark.

## DTU Defect Descriptions With SmolVLM

Use this path when the goal is semantic output: real text descriptions for
annotated wind-turbine blade defects.

First check the Jetson Python/CUDA environment:

```bash
make jetson-check-smolvlm-deps
```

If the check prints a wheel like `torch=...+cu130` and
`cuda_available=False`, the installed PyTorch build is incompatible with the
Jetson CUDA driver/runtime. Replace it with the JetPack 6/CUDA 12.6 aarch64
wheel:

```bash
make jetson-install-jetpack-torch
```

If `transformers` or `PIL` are missing:

```bash
make jetson-install-smolvlm-python-deps
```

For a full one-shot SmolVLM dependency repair:

```bash
make jetson-install-smolvlm-jetson-deps
```

The generic dependency target does not install PyTorch, because CUDA-enabled
PyTorch on Jetson is tied to the JetPack/L4T version. Use
`jetson-install-jetpack-torch` or `jetson-install-smolvlm-jetson-deps` only
when you intentionally want to replace the current PyTorch wheel.
`torch.cuda.is_available()` must be true before this benchmark is meaningful.

```bash
make jetson-run-dtu-smolvlm-defects
```

Default behavior:

```text
data/test-HR.json annotations
  -> raw Nordtank 2018 images
  -> defect-context crops with red bbox overlays
  -> SmolVLM-256M-Instruct on CUDA
  -> token-limited textual damage descriptions
  -> JSONL + Markdown logs
```

Outputs:

```text
logs/hitl-dtu-smolvlm-defects.jsonl
logs/hitl-dtu-smolvlm-defects.md
logs/hitl-dtu-smolvlm-defects-tegrastats.log
tmp/dtu_smolvlm_defects/*.jpg
```

The token limit is controlled through `--max-new-tokens`, defaulting to `32`
for fast Jetson validation:

```bash
make jetson-run-dtu-smolvlm-defects DTU_SMOLVLM_ARGS='--annotations data/test-HR.json --image-root data/dtu_wind_turbine --prefer-folder "Nordtank 2018" --max-images 10 --max-new-tokens 24 --output logs/hitl-dtu-smolvlm-defects-10.jsonl --answers-md logs/hitl-dtu-smolvlm-defects-10.md --crop-dir tmp/dtu_smolvlm_defects'
```

Run a local mapping smoke test without loading the model:

```bash
make dtu-smolvlm-defects-dry-run
```

This checks the COCO annotations, raw-image lookup, crop generation, and log
format. It still requires Pillow because crops are created locally.

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
