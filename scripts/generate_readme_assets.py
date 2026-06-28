#!/usr/bin/env python3
"""Generate TALOS metrics and SVG figures.

The script intentionally uses only the Python standard library so the report can
be regenerated on a clean Jetson or laptop without installing plotting tools.
"""

from __future__ import annotations

import html
import json
from collections import Counter
from pathlib import Path
from statistics import median


ROOT = Path(__file__).resolve().parents[1]
LOG_DIR = ROOT / "logs"
REPORT_DIR = ROOT / "reports"
HARDWARE_DIR = REPORT_DIR / "hardware_runs"
DOCS_DIR = ROOT / "docs"
ASSET_DIR = DOCS_DIR / "assets"


def read_jsonl(path: Path) -> list[dict]:
    rows: list[dict] = []
    if not path.exists():
        return rows
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return rows


def pct(values: list[float], percentile: int) -> float:
    if not values:
        return 0.0
    values = sorted(values)
    index = int((len(values) - 1) * percentile / 100)
    return values[index]


def summarize_jsonl(path: Path, label: str, profile: str) -> dict:
    rows = read_jsonl(path)
    stage_counter = Counter(row.get("stage") for row in rows)
    decisions = Counter(row.get("decision") for row in rows)
    task_types = Counter(row.get("task_type") for row in rows)
    states = Counter(row.get("scheduler_state") for row in rows)
    gate_reasons = Counter(row.get("vlm_gate_reason") for row in rows if row.get("vlm_gate_reason"))
    execution_rows = [row for row in rows if row.get("stage") == "execution"]
    execution_times = [float(row.get("execution_time_ms") or 0) for row in execution_rows]
    runtime_latencies = [float(row.get("latency_ms") or 0) for row in execution_rows]
    anomaly_scores = [float(row.get("feature_anomaly_score") or 0) for row in execution_rows]
    detection_counts = [float(row.get("feature_detection_count") or 0) for row in execution_rows]
    temperatures = [float(row.get("temperature_c") or 0) for row in rows]
    memory = [float(row.get("memory_usage_percent") or 0) for row in rows]
    gpu = [float(row.get("gpu_utilization") or 0) for row in rows]

    return {
        "run_id": path.stem,
        "label": label,
        "profile": profile,
        "path": str(path.relative_to(ROOT)),
        "observations": len(rows),
        "executed": stage_counter.get("execution", 0),
        "admitted": decisions.get("ADMIT", 0),
        "deferred": decisions.get("DEFER", 0),
        "rejected": decisions.get("REJECT", 0),
        "task_types": dict(task_types),
        "scheduler_states": dict(states),
        "vlm_gate_reasons": dict(gate_reasons),
        "execution_time_ms_p50": round(median(execution_times), 3) if execution_times else 0,
        "execution_time_ms_p95": round(pct(execution_times, 95), 3),
        "runtime_latency_ms_p50": round(median(runtime_latencies), 3) if runtime_latencies else 0,
        "runtime_latency_ms_p95": round(pct(runtime_latencies, 95), 3),
        "feature_anomaly_score_p95": round(pct(anomaly_scores, 95), 3),
        "feature_detection_count_max": round(max(detection_counts), 3) if detection_counts else 0,
        "peak_temperature_c": round(max(temperatures), 3) if temperatures else 0,
        "peak_memory_percent": round(max(memory), 3) if memory else 0,
        "peak_gpu_utilization_percent": round(max(gpu), 3) if gpu else 0,
    }


def load_hardware_runs() -> list[dict]:
    runs = []
    for path in sorted(HARDWARE_DIR.glob("*.json")):
        data = json.loads(path.read_text(encoding="utf-8"))
        data["path"] = str(path.relative_to(ROOT))
        runs.append(data)
    return runs


def esc(value: object) -> str:
    return html.escape(str(value), quote=True)


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def write_bytes(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)


def svg_header(width: int, height: int) -> list[str]:
    return [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img">',
        "<style>",
        "text{font-family:Inter,Arial,sans-serif;fill:#17202a} .small{font-size:13px}.label{font-size:15px;font-weight:600}.title{font-size:22px;font-weight:700}.muted{fill:#5d6d7e}.box{fill:#f8fafc;stroke:#ccd6e0;stroke-width:1.5}.green{fill:#1f9d55}.red{fill:#c0392b}.blue{fill:#2471a3}.amber{fill:#d68910}.purple{fill:#6c5ce7}.line{stroke:#2c3e50;stroke-width:2;fill:none}.grid{stroke:#edf2f7;stroke-width:1}",
        "</style>",
    ]


def simple_arrow(x1: int, y1: int, x2: int, y2: int) -> str:
    return (
        f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#34495e" stroke-width="2"/>'
        f'<polygon points="{x2},{y2} {x2-9},{y2-5} {x2-9},{y2+5}" fill="#34495e"/>'
    )


def generate_architecture_svg() -> str:
    parts = svg_header(980, 520)
    parts.append('<rect width="980" height="520" fill="#ffffff"/>')
    parts.append('<text x="40" y="48" class="title">TALOS execution architecture</text>')
    parts.append('<text x="40" y="75" class="small muted">Control-plane admission, deterministic lease, stateless runtime, read-only telemetry.</text>')
    boxes = [
        (45, 130, 180, 92, "L1 Ingestion", "Frame/task arrives", "#f8fafc"),
        (285, 130, 190, 92, "L3 Controller", "state + admission", "#eef7ff"),
        (535, 130, 180, 92, "GPU Lease", "single active owner", "#fff7e8"),
        (775, 130, 160, 92, "C++ Runtime", "execute only", "#f3fff5"),
    ]
    for x, y, w, h, title, subtitle, fill in boxes:
        parts.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="{fill}" stroke="#ccd6e0" stroke-width="1.5"/>')
        parts.append(f'<text x="{x+18}" y="{y+36}" class="label">{esc(title)}</text>')
        parts.append(f'<text x="{x+18}" y="{y+64}" class="small muted">{esc(subtitle)}</text>')
    parts.append(simple_arrow(225, 176, 285, 176))
    parts.append(simple_arrow(475, 176, 535, 176))
    parts.append(simple_arrow(715, 176, 775, 176))
    parts.append('<rect x="285" y="310" width="430" height="116" rx="8" fill="#fbfcfd" stroke="#ccd6e0" stroke-width="1.5"/>')
    parts.append('<text x="310" y="346" class="label">Read-only telemetry</text>')
    parts.append('<text x="310" y="375" class="small muted">tegrastats/sysfs/jtop update memory, temperature, GPU load.</text>')
    parts.append('<text x="310" y="404" class="small muted">Telemetry cannot mutate state; it only changes admission decisions.</text>')
    parts.append('<path d="M500 310 C500 270 380 250 380 222" class="line"/>')
    parts.append('<path d="M500 310 C500 270 620 250 620 222" class="line"/>')
    parts.append('<rect x="45" y="300" width="180" height="126" rx="8" fill="#fffafa" stroke="#f0b7b7" stroke-width="1.5"/>')
    parts.append('<text x="65" y="335" class="label">Hard invariant</text>')
    parts.append('<text x="65" y="365" class="small muted">One GPU-heavy task</text>')
    parts.append('<text x="65" y="390" class="small muted">at a time.</text>')
    parts.append('<text x="65" y="415" class="small red">No runtime policy leakage.</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def generate_policy_svg() -> str:
    parts = svg_header(980, 460)
    parts.append('<rect width="980" height="460" fill="#ffffff"/>')
    parts.append('<text x="40" y="48" class="title">Selective degradation policy</text>')
    parts.append('<text x="40" y="75" class="small muted">When resources are scarce, TALOS sacrifices non-critical VLM before CV/change workloads.</text>')
    rows = [
        ("Normal", "CV ADMIT", "Change ADMIT", "VLM ADMIT", "#eafaf1"),
        ("High load", "CV ADMIT", "Change ADMIT", "VLM DEFER", "#fff7e8"),
        ("Thermal VLM gate", "CV ADMIT", "Change ADMIT", "VLM DEFER", "#fff0f0"),
        ("Degraded", "High-priority CV only", "REJECT", "REJECT", "#f8d7da"),
    ]
    y = 120
    for state, cv, change, vlm, fill in rows:
        parts.append(f'<rect x="50" y="{y}" width="880" height="58" rx="8" fill="{fill}" stroke="#d8dee6"/>')
        parts.append(f'<text x="75" y="{y+36}" class="label">{esc(state)}</text>')
        parts.append(f'<text x="285" y="{y+36}" class="small">{esc(cv)}</text>')
        parts.append(f'<text x="500" y="{y+36}" class="small">{esc(change)}</text>')
        parts.append(f'<text x="735" y="{y+36}" class="small">{esc(vlm)}</text>')
        y += 72
    parts.append('<text x="285" y="108" class="small muted">CV_FEATURES</text>')
    parts.append('<text x="500" y="108" class="small muted">CHANGE_DETECTION</text>')
    parts.append('<text x="735" y="108" class="small muted">VLM_QUERY</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def generate_bar_svg(hardware_runs: list[dict]) -> str:
    gpu = next(run for run in hardware_runs if run["run_id"] == "hitl_gpu_resource_max_thermal55")
    memory = next(run for run in hardware_runs if run["run_id"] == "hitl_resource_max_memory")
    recovery = next(run for run in hardware_runs if run["run_id"] == "hitl_vlm_defer_recovery")
    metrics = [
        ("GPU load", gpu["summary"]["peak_gpu_utilization_percent"], 100, "#2471a3"),
        ("Thermal VLM deferrals", gpu["summary"]["vlm_thermal_pressure_deferrals"], 8, "#c0392b"),
        ("Memory pressure", memory["summary"]["peak_memory_percent"], 100, "#d68910"),
        ("Memory-gated VLM", memory["summary"]["vlm_memory_pressure_decisions"], 25, "#6c5ce7"),
        ("Replay success", recovery["summary"]["vlm_replayed"], recovery["summary"]["vlm_deferred"], "#1f9d55"),
    ]
    parts = svg_header(1080, 420)
    parts.append('<rect width="1080" height="420" fill="#ffffff"/>')
    parts.append('<text x="40" y="48" class="title">Hardware validation summary</text>')
    parts.append('<text x="40" y="75" class="small muted">Real Orin Nano telemetry from HITL runs pasted from terminal.</text>')
    x = 70
    for label, value, max_value, color in metrics:
        bar_h = 220 * min(float(value) / max_value, 1.0)
        parts.append(f'<rect x="{x}" y="{300-bar_h:.1f}" width="105" height="{bar_h:.1f}" rx="5" fill="{color}"/>')
        parts.append(f'<text x="{x+52}" y="{326}" text-anchor="middle" class="label">{value:g}</text>')
        parts.append(f'<text x="{x+52}" y="{355}" text-anchor="middle" class="small muted">{esc(label)}</text>')
        x += 190
    parts.append('<line x1="45" y1="300" x2="1030" y2="300" stroke="#d8dee6"/>')
    parts.append("</svg>")
    return "\n".join(parts)


def scale(value: float, min_value: float, max_value: float, out_min: float, out_max: float) -> float:
    if max_value == min_value:
        return (out_min + out_max) / 2
    return out_min + ((value - min_value) / (max_value - min_value)) * (out_max - out_min)


def wrap_words(text: str, max_chars: int) -> list[str]:
    lines: list[str] = []
    current: list[str] = []
    for word in text.split():
        candidate = " ".join(current + [word])
        if len(candidate) > max_chars and current:
            lines.append(" ".join(current))
            current = [word]
        else:
            current.append(word)
    if current:
        lines.append(" ".join(current))
    return lines


def generate_timeline_svg(gpu_run: dict) -> str:
    points = gpu_run["timeline"]
    xs = [point["elapsed_ms"] / 1000 for point in points]
    temps = [point["temperature_c"] for point in points]
    defs = [point["vlm_thermal_pressure_deferrals"] for point in points]
    parts = svg_header(980, 460)
    parts.append('<rect width="980" height="460" fill="#ffffff"/>')
    parts.append('<text x="40" y="48" class="title">Thermal gate from real hardware telemetry</text>')
    parts.append('<text x="40" y="75" class="small muted">At 55C, low-priority VLM begins to defer while CV/change tasks continue.</text>')
    chart_x, chart_y, chart_w, chart_h = 80, 120, 800, 240
    for i in range(5):
        y = chart_y + i * chart_h / 4
        parts.append(f'<line x1="{chart_x}" y1="{y:.1f}" x2="{chart_x+chart_w}" y2="{y:.1f}" class="grid"/>')
    temp_path = []
    def_path = []
    for x_s, temp, defer in zip(xs, temps, defs):
        x = scale(x_s, min(xs), max(xs), chart_x, chart_x + chart_w)
        y_temp = scale(temp, min(temps) - 0.2, max(temps) + 0.2, chart_y + chart_h, chart_y)
        y_def = scale(defer, 0, max(defs) or 1, chart_y + chart_h, chart_y)
        temp_path.append(f"{x:.1f},{y_temp:.1f}")
        def_path.append(f"{x:.1f},{y_def:.1f}")
    parts.append(f'<polyline points="{" ".join(temp_path)}" fill="none" stroke="#c0392b" stroke-width="3"/>')
    parts.append(f'<polyline points="{" ".join(def_path)}" fill="none" stroke="#2471a3" stroke-width="3"/>')
    for x_s, temp, defer in zip(xs, temps, defs):
        x = scale(x_s, min(xs), max(xs), chart_x, chart_x + chart_w)
        y_temp = scale(temp, min(temps) - 0.2, max(temps) + 0.2, chart_y + chart_h, chart_y)
        y_def = scale(defer, 0, max(defs) or 1, chart_y + chart_h, chart_y)
        parts.append(f'<circle cx="{x:.1f}" cy="{y_temp:.1f}" r="5" fill="#c0392b"/>')
        parts.append(f'<circle cx="{x:.1f}" cy="{y_def:.1f}" r="5" fill="#2471a3"/>')
    gate_x = scale(15.506, min(xs), max(xs), chart_x, chart_x + chart_w)
    parts.append(f'<line x1="{gate_x:.1f}" y1="{chart_y}" x2="{gate_x:.1f}" y2="{chart_y+chart_h}" stroke="#d68910" stroke-dasharray="6 5" stroke-width="2"/>')
    parts.append(f'<text x="{gate_x+8:.1f}" y="{chart_y+20}" class="small amber">target reached</text>')
    parts.append('<text x="80" y="395" class="small red">red: temperature C</text>')
    parts.append('<text x="260" y="395" class="small blue">blue: VLM thermal deferrals</text>')
    parts.append('<text x="680" y="395" class="small muted">x-axis: elapsed seconds</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def generate_recovery_timeline_svg(recovery_run: dict) -> str:
    points = recovery_run["timeline"]
    xs = [point["elapsed_ms"] / 1000 for point in points]
    temps = [point["temperature_c"] for point in points]
    deferred = [point["vlm_deferred"] for point in points]
    replayed = [point["vlm_replayed"] for point in points]
    gate = recovery_run["config"]["vlm_temperature_gate_c"]

    parts = svg_header(980, 500)
    parts.append('<rect width="980" height="500" fill="#ffffff"/>')
    parts.append('<text x="40" y="48" class="title">HITL VLM defer and recovery on Orin Nano</text>')
    parts.append('<text x="40" y="75" class="small muted">Real sysfs telemetry: TALOS defers low-priority VLM while hot, then replays the entire queue after cooldown.</text>')
    chart_x, chart_y, chart_w, chart_h = 82, 120, 800, 250
    for i in range(6):
        y = chart_y + i * chart_h / 5
        parts.append(f'<line x1="{chart_x}" y1="{y:.1f}" x2="{chart_x+chart_w}" y2="{y:.1f}" class="grid"/>')
    temp_path = []
    deferred_path = []
    replay_path = []
    for x_s, temp, defer, replay in zip(xs, temps, deferred, replayed):
        x = scale(x_s, min(xs), max(xs), chart_x, chart_x + chart_w)
        y_temp = scale(temp, min(temps) - 0.5, max(temps) + 0.5, chart_y + chart_h, chart_y)
        y_defer = scale(defer, 0, max(deferred) or 1, chart_y + chart_h, chart_y)
        y_replay = scale(replay, 0, max(replayed) or 1, chart_y + chart_h, chart_y)
        temp_path.append(f"{x:.1f},{y_temp:.1f}")
        deferred_path.append(f"{x:.1f},{y_defer:.1f}")
        replay_path.append(f"{x:.1f},{y_replay:.1f}")
    gate_y = scale(gate, min(temps) - 0.5, max(temps) + 0.5, chart_y + chart_h, chart_y)
    parts.append(f'<line x1="{chart_x}" y1="{gate_y:.1f}" x2="{chart_x+chart_w}" y2="{gate_y:.1f}" stroke="#c0392b" stroke-dasharray="6 5" stroke-width="2"/>')
    parts.append(f'<text x="{chart_x+chart_w-160}" y="{gate_y-8:.1f}" class="small red">VLM gate {gate:.0f}C</text>')
    parts.append(f'<polyline points="{" ".join(temp_path)}" fill="none" stroke="#c0392b" stroke-width="3"/>')
    parts.append(f'<polyline points="{" ".join(deferred_path)}" fill="none" stroke="#d68910" stroke-width="3"/>')
    parts.append(f'<polyline points="{" ".join(replay_path)}" fill="none" stroke="#1f9d55" stroke-width="3"/>')
    for point, x_s, temp, defer, replay in zip(points, xs, temps, deferred, replayed):
        x = scale(x_s, min(xs), max(xs), chart_x, chart_x + chart_w)
        y_temp = scale(temp, min(temps) - 0.5, max(temps) + 0.5, chart_y + chart_h, chart_y)
        parts.append(f'<circle cx="{x:.1f}" cy="{y_temp:.1f}" r="5" fill="#c0392b"/>')
        if point["phase"] in {"cooling_started", "recovery_complete"}:
            parts.append(f'<line x1="{x:.1f}" y1="{chart_y}" x2="{x:.1f}" y2="{chart_y+chart_h}" stroke="#5d6d7e" stroke-dasharray="5 5"/>')
            label = "cooldown starts" if point["phase"] == "cooling_started" else "queue empty"
            parts.append(f'<text x="{x+8:.1f}" y="{chart_y+22}" class="small muted">{label}</text>')
    parts.append('<text x="82" y="415" class="small red">red: temperature C</text>')
    parts.append('<text x="255" y="415" class="small amber">amber: cumulative VLM deferred</text>')
    parts.append('<text x="520" y="415" class="small green">green: deferred VLM replayed</text>')
    parts.append('<text x="720" y="445" class="small muted">x-axis: elapsed seconds</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def generate_recovery_storyboard_svg(recovery_run: dict) -> str:
    s = recovery_run["summary"]
    parts = svg_header(980, 520)
    parts.append('<rect width="980" height="520" fill="#ffffff"/>')
    parts.append('<text x="40" y="48" class="title">Mission story: protect CV, postpone VLM, recover later</text>')
    parts.append('<text x="40" y="75" class="small muted">Deferred VLM work is replayed after thermal recovery instead of running during constrained conditions.</text>')
    cards = [
        ("1. Nominal", "CV / change / VLM are admitted while telemetry is healthy.", "temp < gate", "#eef7ff"),
        ("2. Thermal pressure", "Low-priority VLM is deferred; critical CV keeps running.", f"vlm_deferred={s['vlm_deferred']}", "#fff7e8"),
        ("3. Cooldown", "TALOS stops synthetic burn and waits for real telemetry to recover.", "burners off", "#f8fafc"),
        ("4. Recovery", "Deferred VLM work is replayed. No VLM task is lost.", f"vlm_replayed={s['vlm_replayed']}/{s['vlm_deferred']}", "#eafaf1"),
    ]
    x = 45
    for title, body, metric, fill in cards:
        parts.append(f'<rect x="{x}" y="132" width="205" height="210" rx="8" fill="{fill}" stroke="#ccd6e0" stroke-width="1.5"/>')
        parts.append(f'<text x="{x+18}" y="170" class="label">{esc(title)}</text>')
        for i, line in enumerate(wrap_words(body, 28)[:4]):
            parts.append(f'<text x="{x+18}" y="{205 + i*24}" class="small muted">{esc(line)}</text>')
        parts.append(f'<rect x="{x+18}" y="280" width="160" height="36" rx="6" fill="#ffffff" stroke="#d8dee6"/>')
        parts.append(f'<text x="{x+98}" y="303" text-anchor="middle" class="small">{esc(metric)}</text>')
        x += 235
    parts.append('<text x="55" y="420" class="label">Observed HITL result</text>')
    parts.append(f'<text x="55" y="450" class="small muted">unique_tasks={s["unique_tasks"]}, executed={s["executed"]}, rejected={s["rejected"]}, peak_temp_c={s["peak_temperature_c"]:.3f}, high_load_samples={s["high_load_samples"]}</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def gif_lzw_data(pixels: bytes, clear_interval: int = 200) -> bytes:
    clear = 256
    end = 257
    codes: list[int] = [clear]
    emitted_since_clear = 0
    for pixel in pixels:
        if emitted_since_clear >= clear_interval:
            codes.append(clear)
            emitted_since_clear = 0
        codes.append(pixel)
        emitted_since_clear += 1
    codes.append(end)

    out = bytearray()
    bit_buffer = 0
    bit_count = 0
    code_size = 9
    next_code = 258
    for code in codes:
        bit_buffer |= code << bit_count
        bit_count += code_size
        while bit_count >= 8:
            out.append(bit_buffer & 0xFF)
            bit_buffer >>= 8
            bit_count -= 8
        if code == clear:
            code_size = 9
            next_code = 258
        elif code != end:
            next_code += 1
            if next_code >= (1 << code_size) and code_size < 12:
                code_size += 1
    if bit_count:
        out.append(bit_buffer & 0xFF)

    blocks = bytearray([8])
    for start in range(0, len(out), 255):
        block = out[start : start + 255]
        blocks.append(len(block))
        blocks.extend(block)
    blocks.append(0)
    return bytes(blocks)


def generate_recovery_gif(recovery_run: dict) -> bytes:
    width, height = 640, 360
    palette = [
        (255, 255, 255), (23, 32, 42), (31, 157, 85), (214, 137, 16),
        (192, 57, 43), (36, 113, 163), (216, 222, 230), (108, 92, 231),
    ] + [(255, 255, 255)] * 248

    def frame(temp_ratio: float, queued: int, replayed: int, burner_on: bool) -> bytes:
        px = bytearray([0] * (width * height))

        def rect(x: int, y: int, w: int, h: int, color: int) -> None:
            for yy in range(max(0, y), min(height, y + h)):
                start = yy * width + max(0, x)
                end = yy * width + min(width, x + w)
                px[start:end] = bytes([color]) * (end - start)

        rect(40, 40, 560, 20, 6)
        rect(40, 40, int(560 * temp_ratio), 20, 4)
        rect(80, 120, 150, 74, 2)
        rect(80, 215, 150, 74, 2)
        rect(285, 110, 100, 195, 6)
        for i in range(queued):
            rect(300, 120 + i * 13, 70, 9, 3)
        for i in range(replayed):
            rect(455, 120 + i * 13, 70, 9, 5)
        if burner_on:
            rect(545, 132, 52, 52, 4)
            rect(560, 195, 22, 88, 4)
        else:
            rect(545, 132, 52, 52, 6)
            rect(560, 195, 22, 88, 6)
        return bytes(px)

    frames = [
        frame(0.35, 0, 0, True),
        frame(0.76, 8, 0, True),
        frame(0.92, 18, 0, True),
        frame(0.66, 18, 0, False),
        frame(0.48, 10, 8, False),
        frame(0.42, 0, 18, False),
    ]

    data = bytearray(b"GIF89a")
    data.extend(width.to_bytes(2, "little"))
    data.extend(height.to_bytes(2, "little"))
    data.extend(bytes([0b11110111, 0, 0]))
    for r, g, b in palette:
        data.extend(bytes([r, g, b]))
    for pixels in frames:
        data.extend(b"\x21\xF9\x04")
        data.extend(bytes([0x04]))
        data.extend((80).to_bytes(2, "little"))
        data.extend(b"\x00\x00")
        data.extend(b"\x2C")
        data.extend((0).to_bytes(2, "little") * 2)
        data.extend(width.to_bytes(2, "little"))
        data.extend(height.to_bytes(2, "little"))
        data.extend(b"\x00")
        data.extend(gif_lzw_data(pixels))
    data.extend(b"\x3B")
    return bytes(data)


def generate_markdown(summary: dict) -> str:
    hw = {run["run_id"]: run for run in summary["hardware_runs"]}
    gpu = hw["hitl_gpu_resource_max_thermal55"]["summary"]
    mem = hw["hitl_resource_max_memory"]["summary"]
    recovery = hw["hitl_vlm_defer_recovery"]["summary"]
    output = [
        "# TALOS Metrics Report",
        "",
        "Generated by `scripts/generate_readme_assets.py`.",
        "",
        "## Hardware Results",
        "",
        "| Run | Evidence | Result |",
        "| --- | --- | --- |",
        f"| GPU thermal gate | `GR3D_FREQ={gpu['peak_gpu_utilization_percent']:.0f}%`, peak temp `{gpu['peak_temperature_c']:.3f}C` | `vlm_thermal_pressure_deferrals={gpu['vlm_thermal_pressure_deferrals']}` while `executed={gpu['executed']}` |",
        f"| RAM/queue pressure | peak memory `{mem['peak_memory_percent']:.3f}%`, max queue pressure `{mem['max_queue_pressure']}` | `vlm_memory_pressure_decisions={mem['vlm_memory_pressure_decisions']}` while `executed={mem['executed']}` |",
        f"| VLM recovery | peak temp `{recovery['peak_temperature_c']:.3f}C`, `vlm_deferred={recovery['vlm_deferred']}` | `vlm_replayed={recovery['vlm_replayed']}/{recovery['vlm_deferred']}`, `rejected={recovery['rejected']}` |",
        "",
        "## Local JSONL Runs",
        "",
        "| Run | Profile | Observations | Executed | Deferred | Peak Temp | Peak Memory | Anomaly P95 | Max Detections | VLM Gate Reasons |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |",
    ]
    for run in summary["jsonl_runs"]:
        reasons = ", ".join(f"{k}:{v}" for k, v in sorted(run["vlm_gate_reasons"].items())) or "-"
        output.append(
            f"| {run['label']} | {run['profile']} | {run['observations']} | {run['executed']} | {run['deferred']} | {run['peak_temperature_c']:.3f} | {run['peak_memory_percent']:.3f} | {run['feature_anomaly_score_p95']:.3f} | {run['feature_detection_count_max']:.0f} | {reasons} |"
        )
    output.append("")
    return "\n".join(output)


def build_summary() -> dict:
    jsonl_specs = [
        ("SITL mission runtime smoke", "SITL", LOG_DIR / "sitl-mission-runtime-smoke-v2.jsonl"),
        ("SITL Phase 8 optimization", "SITL", LOG_DIR / "sitl-phase8-optimization.jsonl"),
        ("SITL Phase 6 contention", "SITL", LOG_DIR / "phase6-contention.jsonl"),
        ("HITL local thermal smoke", "HITL smoke", LOG_DIR / "hitl-thermal-smoke.jsonl"),
        ("HITL local resource smoke", "HITL smoke", LOG_DIR / "hitl-resource-smoke.jsonl"),
    ]
    jsonl_runs = [
        summarize_jsonl(path, label, profile)
        for label, profile, path in jsonl_specs
        if path.exists()
    ]
    return {
        "generated_by": "scripts/generate_readme_assets.py",
        "jsonl_runs": jsonl_runs,
        "hardware_runs": load_hardware_runs(),
    }


def main() -> None:
    DOCS_DIR.mkdir(exist_ok=True)
    ASSET_DIR.mkdir(parents=True, exist_ok=True)
    summary = build_summary()
    hardware_runs = summary["hardware_runs"]
    gpu_run = next(run for run in hardware_runs if run["run_id"] == "hitl_gpu_resource_max_thermal55")
    recovery_run = next(run for run in hardware_runs if run["run_id"] == "hitl_vlm_defer_recovery")

    write(ASSET_DIR / "talos_architecture.svg", generate_architecture_svg())
    write(ASSET_DIR / "admission_policy.svg", generate_policy_svg())
    write(ASSET_DIR / "hardware_summary.svg", generate_bar_svg(hardware_runs))
    write(ASSET_DIR / "hitl_thermal_timeline.svg", generate_timeline_svg(gpu_run))
    write(ASSET_DIR / "hitl_defer_recovery_timeline.svg", generate_recovery_timeline_svg(recovery_run))
    write(ASSET_DIR / "talos_recovery_storyboard.svg", generate_recovery_storyboard_svg(recovery_run))
    write_bytes(ASSET_DIR / "talos_defer_recovery.gif", generate_recovery_gif(recovery_run))
    write(DOCS_DIR / "metrics_summary.json", json.dumps(summary, indent=2))
    write(DOCS_DIR / "metrics_report.md", generate_markdown(summary))
    print(f"generated {ASSET_DIR.relative_to(ROOT)} and {DOCS_DIR / 'metrics_report.md'}")


if __name__ == "__main__":
    main()
