#!/usr/bin/env python3
"""Describe annotated DTU wind-turbine defects with a real SmolVLM backend.

This script is intentionally outside the Rust build: it validates semantic VLM
output on Jetson without making TALOS depend on PyTorch/Transformers at compile
time. It consumes COCO-style DTU annotation JSON, crops/overlays annotated
defects, runs SmolVLM once per annotated image, and writes JSONL/Markdown logs.
"""

from __future__ import annotations

import argparse
import json
import time
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any


DEFAULT_MODEL_ID = "HuggingFaceTB/SmolVLM-256M-Instruct"
DEFAULT_PROMPT = (
    "You are assisting wind-turbine blade inspection. The crop is centered on "
    "one annotated defect and contains no visual annotation overlay. Describe "
    "only the visible blade damage in one concise sentence. Mention uncertainty "
    "if the crop is unclear."
)


@dataclass(frozen=True)
class ImageRecord:
    image_id: int
    file_name: str
    width: int
    height: int


@dataclass(frozen=True)
class AnnotationRecord:
    annotation_id: int
    image_id: int
    bbox: tuple[float, float, float, float]
    category_id: int


@dataclass(frozen=True)
class DefectExample:
    example_id: str
    image: ImageRecord
    annotations: list[AnnotationRecord]
    raw_path: Path
    full_bboxes: list[tuple[float, float, float, float]]
    category_names: list[str]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Describe DTU defects with SmolVLM.")
    parser.add_argument("--annotations", default="data/test-HR.json")
    parser.add_argument("--image-root", default="data/dtu_wind_turbine")
    parser.add_argument("--prefer-folder", default="Nordtank 2018")
    parser.add_argument("--model-id", default=DEFAULT_MODEL_ID)
    parser.add_argument("--output", default="logs/dtu-smolvlm-defects.jsonl")
    parser.add_argument("--answers-md", default="logs/dtu-smolvlm-defects.md")
    parser.add_argument("--crop-dir", default="tmp/dtu_smolvlm_defects")
    parser.add_argument("--prompt", default=DEFAULT_PROMPT)
    parser.add_argument("--max-images", type=int, default=0, help="0 means all annotated images.")
    parser.add_argument("--max-new-tokens", type=int, default=32)
    parser.add_argument("--image-size", type=int, default=448)
    parser.add_argument("--context-margin", type=float, default=1.75)
    parser.add_argument("--min-crop-size", type=int, default=384)
    parser.add_argument(
        "--group-mode",
        choices=("annotation", "image"),
        default="annotation",
        help="annotation creates one VLM prompt per defect; image groups all bboxes in a frame.",
    )
    parser.add_argument("--draw-box", action="store_true")
    parser.add_argument("--draw-labels", action="store_true")
    parser.add_argument("--progress-every", type=int, default=1)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--allow-cpu", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    annotations_path = Path(args.annotations)
    image_root = Path(args.image_root)
    output_path = Path(args.output)
    answers_path = Path(args.answers_md)
    crop_dir = Path(args.crop_dir)

    examples = load_examples(
        annotations_path=annotations_path,
        image_root=image_root,
        prefer_folder=args.prefer_folder,
        group_mode=args.group_mode,
    )
    if args.max_images > 0:
        examples = examples[: args.max_images]

    output_path.parent.mkdir(parents=True, exist_ok=True)
    answers_path.parent.mkdir(parents=True, exist_ok=True)
    crop_dir.mkdir(parents=True, exist_ok=True)

    require_pillow()

    if args.dry_run:
        runner = None
    else:
        runner = SmolVlmRunner(
            model_id=args.model_id,
            image_size=args.image_size,
            max_new_tokens=args.max_new_tokens,
            allow_cpu=args.allow_cpu,
        )

    started = time.perf_counter()
    processed = 0
    failures = 0

    with output_path.open("w", encoding="utf-8") as out, answers_path.open(
        "w", encoding="utf-8"
    ) as md:
        md.write("# TALOS DTU SmolVLM Defect Descriptions\n\n")
        md.write(f"- annotations: `{annotations_path}`\n")
        md.write(f"- model: `{args.model_id}`\n")
        md.write(f"- max_new_tokens: `{args.max_new_tokens}`\n")
        md.write(f"- examples: `{len(examples)}`\n\n")

        for index, example in enumerate(examples, start=1):
            record_started = time.perf_counter()
            try:
                crop_path = crop_defect_context(
                    example=example,
                    crop_dir=crop_dir,
                    context_margin=args.context_margin,
                    min_crop_size=args.min_crop_size,
                    draw_box=args.draw_box,
                    draw_labels=args.draw_labels,
                )
                if runner is None:
                    answer = "dry_run: VLM was not executed"
                    tokens = 0
                    peak_cuda_mb = None
                else:
                    prompt = prompt_for_example(args.prompt, example)
                    answer, tokens, peak_cuda_mb = runner.describe(crop_path, prompt)

                latency_ms = int((time.perf_counter() - record_started) * 1000)
                row = {
                    "ok": True,
                    "index": index,
                    "example_id": example.example_id,
                    "image_id": example.image.image_id,
                    "file_name": example.image.file_name,
                    "raw_image_path": str(example.raw_path),
                    "crop_path": str(crop_path),
                    "bbox_count": len(example.annotations),
                    "categories": example.category_names,
                    "bboxes_xywh": [list(box) for box in example.full_bboxes],
                    "model": args.model_id,
                    "max_new_tokens": args.max_new_tokens,
                    "latency_ms": latency_ms,
                    "tokens": tokens,
                    "peak_cuda_allocated_mb": peak_cuda_mb,
                    "answer": answer,
                }
                processed += 1
                md.write(f"## {index}. {example.example_id}\n\n")
                md.write(f"- image: `{example.image.file_name}`\n")
                md.write(f"- raw: `{example.raw_path}`\n")
                md.write(f"- crop: `{crop_path}`\n")
                md.write(f"- categories: `{', '.join(example.category_names)}`\n")
                md.write(f"- boxes: `{len(example.annotations)}`\n")
                md.write(f"- latency_ms: `{latency_ms}`\n\n")
                md.write(f"{answer}\n\n")
            except Exception as exc:  # noqa: BLE001
                failures += 1
                row = {
                    "ok": False,
                    "index": index,
                    "example_id": example.example_id,
                    "image_id": example.image.image_id,
                    "file_name": example.image.file_name,
                    "raw_image_path": str(example.raw_path),
                    "bbox_count": len(example.annotations),
                    "categories": example.category_names,
                    "model": args.model_id,
                    "error": f"{type(exc).__name__}: {exc}",
                }

            out.write(json.dumps(row, ensure_ascii=False) + "\n")
            out.flush()
            if args.progress_every > 0 and index % args.progress_every == 0:
                print(
                    "progress "
                    f"completed={index} total={len(examples)} "
                    f"processed={processed} failures={failures}"
                )

    elapsed_s = time.perf_counter() - started
    print("mode=dtu-smolvlm-defects")
    print(f"annotations={annotations_path}")
    print(f"image_root={image_root}")
    print(f"model={args.model_id}")
    print(f"examples={len(examples)} processed={processed} failures={failures}")
    print(f"output={output_path}")
    print(f"answers_md={answers_path}")
    print(f"elapsed_s={elapsed_s:.3f}")
    return 0 if failures == 0 else 1


class SmolVlmRunner:
    def __init__(
        self,
        model_id: str,
        image_size: int,
        max_new_tokens: int,
        allow_cpu: bool,
    ) -> None:
        try:
            import torch
            import PIL.Image
            import PIL.ImagePalette
            from transformers import AutoModelForVision2Seq, AutoProcessor
        except Exception as exc:  # noqa: BLE001
            raise RuntimeError(
                "missing SmolVLM dependencies; install torch, transformers, pillow"
            ) from exc

        self.torch = torch
        self.max_new_tokens = max_new_tokens
        self.device = "cuda" if torch.cuda.is_available() else "cpu"
        if self.device != "cuda" and not allow_cpu:
            raise RuntimeError("CUDA is not available; pass --allow-cpu only for local smoke tests")

        self.processor = AutoProcessor.from_pretrained(
            model_id,
            size={"longest_edge": image_size},
        )
        dtype = torch.float16 if self.device == "cuda" else torch.float32
        self.model = AutoModelForVision2Seq.from_pretrained(
            model_id,
            torch_dtype=dtype,
            _attn_implementation="eager",
            low_cpu_mem_usage=True,
        ).to(self.device)
        self.model.eval()

    def describe(self, image_path: Path, prompt_text: str) -> tuple[str, int, float | None]:
        from PIL import Image

        image = Image.open(image_path).convert("RGB")
        messages = [
            {
                "role": "user",
                "content": [
                    {"type": "image"},
                    {"type": "text", "text": prompt_text},
                ],
            }
        ]
        prompt = self.processor.apply_chat_template(messages, add_generation_prompt=True)
        inputs = self.processor(text=prompt, images=[image], return_tensors="pt").to(self.device)

        if self.device == "cuda":
            self.torch.cuda.reset_peak_memory_stats()
            self.torch.cuda.synchronize()
        with self.torch.inference_mode():
            generated_ids = self.model.generate(
                **inputs,
                max_new_tokens=self.max_new_tokens,
                do_sample=False,
            )
        if self.device == "cuda":
            self.torch.cuda.synchronize()

        input_len = inputs["input_ids"].shape[1]
        generated_ids = generated_ids[:, input_len:]
        output_text = self.processor.batch_decode(
            generated_ids,
            skip_special_tokens=True,
            clean_up_tokenization_spaces=False,
        )[0]
        tokens = int(generated_ids.shape[1])
        peak_cuda_mb = None
        if self.device == "cuda":
            peak_cuda_mb = self.torch.cuda.max_memory_allocated() / (1024**2)
        return compact(output_text), tokens, peak_cuda_mb


def require_pillow() -> None:
    try:
        import PIL.Image  # noqa: F401
        import PIL.ImageDraw  # noqa: F401
        import PIL.ImagePalette  # noqa: F401
    except Exception as exc:  # noqa: BLE001
        raise RuntimeError("missing Pillow dependency; install python3-pil or pillow") from exc


def prompt_for_example(base_prompt: str, example: DefectExample) -> str:
    categories = ", ".join(
        f"{category} ({expand_category(category)})" for category in example.category_names
    )
    return (
        f"{base_prompt} Annotated defect category codes for this crop: {categories}. "
        "Use the category as weak context, but describe what is visually visible. "
        "The image crop has no visible annotation overlay."
    )


def expand_category(category: str) -> str:
    return {
        "LE;ER": "leading-edge erosion",
        "LE;CR": "leading-edge crack",
        "LR;DA": "lightning receptor damage",
        "SF;PO": "surface coating or paint damage",
        "VG;MT": "vortex generator or mounting issue",
    }.get(category, "unknown defect category")


def load_examples(
    annotations_path: Path,
    image_root: Path,
    prefer_folder: str,
    group_mode: str,
) -> list[DefectExample]:
    data = json.loads(annotations_path.read_text(encoding="utf-8"))
    images = {
        int(image["id"]): ImageRecord(
            image_id=int(image["id"]),
            file_name=str(image["file_name"]),
            width=int(image["width"]),
            height=int(image["height"]),
        )
        for image in data["images"]
    }
    categories = {int(item["id"]): str(item["name"]) for item in data["categories"]}
    grouped: dict[int, list[AnnotationRecord]] = defaultdict(list)
    for annotation in data["annotations"]:
        bbox = annotation["bbox"]
        grouped[int(annotation["image_id"])].append(
            AnnotationRecord(
                annotation_id=int(annotation["id"]),
                image_id=int(annotation["image_id"]),
                bbox=(float(bbox[0]), float(bbox[1]), float(bbox[2]), float(bbox[3])),
                category_id=int(annotation["category_id"]),
            )
        )

    raw_index = index_raw_images(image_root)
    examples = []
    for image_id in sorted(grouped):
        image = images[image_id]
        raw_name, offset_x, offset_y = raw_name_and_tile_offset(image.file_name, image.width)
        raw_path = choose_raw_path(raw_index.get(raw_name, []), prefer_folder)
        if raw_path is None:
            continue
        image_annotations = grouped[image_id]
        if group_mode == "image":
            annotations = image_annotations
            full_bboxes = [
                (bbox[0] + offset_x, bbox[1] + offset_y, bbox[2], bbox[3])
                for bbox in (annotation.bbox for annotation in annotations)
            ]
            category_names = [
                categories.get(annotation.category_id, f"category_{annotation.category_id}")
                for annotation in annotations
            ]
            examples.append(
                DefectExample(
                    example_id=Path(image.file_name).stem,
                    image=image,
                    annotations=annotations,
                    raw_path=raw_path,
                    full_bboxes=full_bboxes,
                    category_names=category_names,
                )
            )
        else:
            for annotation in image_annotations:
                bbox = annotation.bbox
                category_name = categories.get(
                    annotation.category_id, f"category_{annotation.category_id}"
                )
                examples.append(
                    DefectExample(
                        example_id=f"{Path(image.file_name).stem}_ann{annotation.annotation_id}",
                        image=image,
                        annotations=[annotation],
                        raw_path=raw_path,
                        full_bboxes=[(bbox[0] + offset_x, bbox[1] + offset_y, bbox[2], bbox[3])],
                        category_names=[category_name],
                    )
                )

    if not examples:
        raise RuntimeError(
            f"no annotated examples could be mapped from {annotations_path} to {image_root}"
        )
    return examples


def index_raw_images(root: Path) -> dict[str, list[Path]]:
    index: dict[str, list[Path]] = defaultdict(list)
    for path in root.rglob("*"):
        if path.suffix.lower() in {".jpg", ".jpeg", ".png"}:
            index[path.name].append(path)
    return index


def choose_raw_path(paths: list[Path], prefer_folder: str) -> Path | None:
    if not paths:
        return None
    for path in paths:
        if prefer_folder and prefer_folder in path.parts:
            return path
    return sorted(paths)[0]


def raw_name_and_tile_offset(file_name: str, width: int) -> tuple[str, int, int]:
    path = Path(file_name)
    parts = path.stem.split("_")
    if len(parts) >= 4 and width == 1024:
        raw_name = "_".join(parts[:2]) + path.suffix
        tile_row = int(parts[2])
        tile_col = int(parts[3])
        return raw_name, tile_col * 1024, tile_row * 1024
    return path.name, 0, 0


def crop_defect_context(
    example: DefectExample,
    crop_dir: Path,
    context_margin: float,
    min_crop_size: int,
    draw_box: bool,
    draw_labels: bool,
) -> Path:
    from PIL import Image, ImageDraw

    image = Image.open(example.raw_path).convert("RGB")
    width, height = image.size
    x1, y1, x2, y2 = union_xyxy(example.full_bboxes)
    box_w = max(1.0, x2 - x1)
    box_h = max(1.0, y2 - y1)
    target_side = max(min_crop_size, int(max(box_w, box_h) * (1.0 + 2.0 * context_margin)))
    center_x = (x1 + x2) / 2.0
    center_y = (y1 + y2) / 2.0
    crop_x1 = max(0, int(center_x - target_side / 2))
    crop_y1 = max(0, int(center_y - target_side / 2))
    crop_x2 = min(width, crop_x1 + target_side)
    crop_y2 = min(height, crop_y1 + target_side)
    crop_x1 = max(0, crop_x2 - target_side)
    crop_y1 = max(0, crop_y2 - target_side)
    crop = image.crop((crop_x1, crop_y1, crop_x2, crop_y2))

    if draw_box or draw_labels:
        draw = ImageDraw.Draw(crop)
        for bbox, category in zip(example.full_bboxes, example.category_names):
            bx, by, bw, bh = bbox
            left = bx - crop_x1
            top = by - crop_y1
            right = left + bw
            bottom = top + bh
            if draw_box:
                draw.rectangle((left, top, right, bottom), outline=(255, 0, 0), width=3)
            if draw_labels:
                draw.text((left + 4, max(0, top - 14)), category, fill=(255, 0, 0))

    output = crop_dir / f"{example.example_id}_defect.jpg"
    crop.save(output, quality=92)
    return output


def union_xyxy(bboxes: list[tuple[float, float, float, float]]) -> tuple[float, float, float, float]:
    x1 = min(box[0] for box in bboxes)
    y1 = min(box[1] for box in bboxes)
    x2 = max(box[0] + box[2] for box in bboxes)
    y2 = max(box[1] + box[3] for box in bboxes)
    return x1, y1, x2, y2


def compact(text: str) -> str:
    return " ".join(text.split())[:1024]


if __name__ == "__main__":
    raise SystemExit(main())
