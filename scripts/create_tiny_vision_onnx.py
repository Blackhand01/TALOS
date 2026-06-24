#!/usr/bin/env python3
"""Create a tiny static-shape vision ONNX model for TensorRT validation.

The model is intentionally small and deterministic:

input[1,3,224,224] -> Conv -> Relu -> GlobalAveragePool -> Flatten -> Gemm -> output[1,4]

It is not trained for accuracy. Its purpose is to provide a real ONNX graph that
TensorRT can parse, build, and execute inside the TALOS admission/lease/logging
path without storing large model artifacts in git.
"""

from __future__ import annotations

import argparse
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Create TALOS tiny vision ONNX.")
    parser.add_argument("--output", default="models/vision.onnx")
    parser.add_argument("--opset", type=int, default=13)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        import numpy as np
        import onnx
        from onnx import TensorProto, helper, numpy_helper
    except Exception as exc:  # noqa: BLE001
        print(f"dependency_error: {type(exc).__name__}: {exc}")
        print("install with: python3 -m pip install --user onnx numpy")
        return 2

    rng = np.random.default_rng(seed=42)
    conv_w = rng.normal(0.0, 0.05, size=(8, 3, 3, 3)).astype(np.float32)
    conv_b = rng.normal(0.0, 0.01, size=(8,)).astype(np.float32)
    gemm_w = rng.normal(0.0, 0.05, size=(4, 8)).astype(np.float32)
    gemm_b = rng.normal(0.0, 0.01, size=(4,)).astype(np.float32)

    graph = helper.make_graph(
        nodes=[
            helper.make_node(
                "Conv",
                inputs=["input", "conv_w", "conv_b"],
                outputs=["conv"],
                pads=[1, 1, 1, 1],
                strides=[2, 2],
            ),
            helper.make_node("Relu", inputs=["conv"], outputs=["relu"]),
            helper.make_node("GlobalAveragePool", inputs=["relu"], outputs=["gap"]),
            helper.make_node("Flatten", inputs=["gap"], outputs=["flat"], axis=1),
            helper.make_node(
                "Gemm",
                inputs=["flat", "gemm_w", "gemm_b"],
                outputs=["output"],
                transB=1,
            ),
        ],
        name="talos_tiny_vision",
        inputs=[
            helper.make_tensor_value_info(
                "input", TensorProto.FLOAT, [1, 3, 224, 224]
            )
        ],
        outputs=[
            helper.make_tensor_value_info("output", TensorProto.FLOAT, [1, 4])
        ],
        initializer=[
            numpy_helper.from_array(conv_w, "conv_w"),
            numpy_helper.from_array(conv_b, "conv_b"),
            numpy_helper.from_array(gemm_w, "gemm_w"),
            numpy_helper.from_array(gemm_b, "gemm_b"),
        ],
    )

    model = helper.make_model(
        graph,
        producer_name="talos",
        opset_imports=[helper.make_operatorsetid("", args.opset)],
    )
    onnx.checker.check_model(model)

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    onnx.save(model, output)
    print(f"wrote {output} bytes={output.stat().st_size}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
