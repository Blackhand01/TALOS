#!/usr/bin/env python3
"""Run a real SmolVLM CUDA inference for TALOS model-backend validation.

This script is intentionally isolated from the Rust build. TALOS can compile on
systems without PyTorch/Transformers, while Jetson HITL runs can still exercise
a real VLM backend when the dependencies and model are installed.
"""

from __future__ import annotations

import argparse
import time
from pathlib import Path


DEFAULT_MODEL_ID = "HuggingFaceTB/SmolVLM-256M-Instruct"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="TALOS SmolVLM CUDA probe.")
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--image-path", default="tmp/talos_smolvlm_probe.png")
    parser.add_argument("--image-size", type=int, default=384)
    parser.add_argument("--max-new-tokens", type=int, default=32)
    parser.add_argument(
        "--prompt",
        default=(
            "Inspect this image as an onboard vision-language system. "
            "Return one concise sentence describing visible damage or anomaly."
        ),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        import torch
        from PIL import Image, ImageDraw
        from transformers import AutoModelForVision2Seq, AutoProcessor
    except Exception as exc:  # noqa: BLE001
        print(f"dependency_error: {type(exc).__name__}: {exc}")
        return 2

    print("backend=smolvlm-cuda")
    print(f"model={args.model_id}")
    print(f"torch={torch.__version__}")
    print(f"cuda_available={torch.cuda.is_available()}")
    if not torch.cuda.is_available():
        print("runtime_error: CUDA is not available")
        return 3
    print(f"cuda_device={torch.cuda.get_device_name(0)}")

    image = load_or_create_image(args.image_path, args.image_size, Image, ImageDraw)
    processor = AutoProcessor.from_pretrained(args.model_id, size={"longest_edge": args.image_size})
    model = AutoModelForVision2Seq.from_pretrained(
        args.model_id,
        torch_dtype=torch.float16,
        _attn_implementation="eager",
        low_cpu_mem_usage=True,
    ).to("cuda")

    messages = [
        {
            "role": "user",
            "content": [
                {"type": "image"},
                {"type": "text", "text": args.prompt},
            ],
        }
    ]
    prompt = processor.apply_chat_template(messages, add_generation_prompt=True)
    inputs = processor(text=prompt, images=[image], return_tensors="pt").to("cuda")

    torch.cuda.reset_peak_memory_stats()
    torch.cuda.synchronize()
    started = time.perf_counter()
    with torch.inference_mode():
        generated_ids = model.generate(**inputs, max_new_tokens=args.max_new_tokens)
    torch.cuda.synchronize()
    elapsed_s = time.perf_counter() - started

    input_len = inputs["input_ids"].shape[1]
    generated_ids = generated_ids[:, input_len:]
    output_text = processor.batch_decode(
        generated_ids,
        skip_special_tokens=True,
        clean_up_tokenization_spaces=False,
    )[0]
    tokens = int(generated_ids.shape[1])
    peak_gb = torch.cuda.max_memory_allocated() / (1024**3)

    print(f"elapsed_s: {elapsed_s:.3f}")
    print(f"tokens: {tokens}")
    print(f"tps: {tokens / elapsed_s if elapsed_s > 0 else 0.0:.3f}")
    print(f"peak_cuda_allocated_gb: {peak_gb:.3f}")
    print(f"result: {compact(output_text)}")
    return 0


def load_or_create_image(path_value: str, image_size: int, Image, ImageDraw):
    path = Path(path_value)
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        image = Image.new("RGB", (image_size, image_size), (24, 28, 36))
        draw = ImageDraw.Draw(image)
        draw.rectangle((image_size // 3, image_size // 3, image_size // 2, image_size // 2), fill=(220, 40, 32))
        draw.line((20, image_size - 40, image_size - 20, 40), fill=(230, 230, 210), width=3)
        image.save(path)

    image = Image.open(path).convert("RGB")
    image.thumbnail((image_size, image_size))
    canvas = Image.new("RGB", (image_size, image_size), (0, 0, 0))
    x = (image_size - image.width) // 2
    y = (image_size - image.height) // 2
    canvas.paste(image, (x, y))
    return canvas


def compact(text: str) -> str:
    return " ".join(text.split())[:512]


if __name__ == "__main__":
    raise SystemExit(main())
