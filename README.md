# TALOS (Tensor Allocation Layer for Onboard Systems)
**Safety-aware resource arbitration and execution isolation for edge robotics. Built for environments where GPU starvation is not an option.**

[![Rust](https://img.shields.io/badge/Rust-Control_Plane-orange.svg)]()
[![C++](https://img.shields.io/badge/C++20-Data_Plane-blue.svg)]()
[![CUDA](https://img.shields.io/badge/TensorRT-Zero_Copy-green.svg)]()
[![Target](https://img.shields.io/badge/Target-Orin_Nano_8GB-lightgrey.svg)]()

Aegis-RT is not another AI demo. It is a strict **GPU execution arbiter** designed to run perception pipelines (DINOv2), temporal anomaly detection, and Vision-Language Models (Qwen2.5-VL) on an 8GB hardware constraint without causing Out-Of-Memory (OOM) kills or thermal throttling.

[System Architecture] | [Experiments & Sweep Data] | [IPC Zero-Copy Docs]