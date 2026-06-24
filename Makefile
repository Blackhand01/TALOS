SHELL := /bin/bash
.DEFAULT_GOAL := help

JETSON_USER ?= ste
JETSON_ADDR ?= 192.168.55.1
JETSON_HOST ?= $(JETSON_USER)@$(JETSON_ADDR)
JETSON_DIR ?= /home/$(JETSON_USER)/TALOS
JETSON_OLD_REPO ?= /home/$(JETSON_USER)/Edge-VLA-Micro
JETSON_TARGET_DIR ?= /tmp/talos-target
LOCAL_TARGET_DIR ?= /tmp/talos-target

SSH ?= ssh
RSYNC ?= rsync
CARGO ?= cargo
PYTHON ?= python3
CUDA_HOME ?= /usr/local/cuda
TRTEXEC_CANDIDATES ?= trtexec /usr/src/tensorrt/bin/trtexec /usr/local/TensorRT/bin/trtexec

SSH_CONTROL_PATH ?= /tmp/talos-ssh-%r@%h:%p
SSH_OPTS ?= -o ServerAliveInterval=30 -o ServerAliveCountMax=3 -o ControlMaster=auto -o ControlPersist=10m -o ControlPath=$(SSH_CONTROL_PATH)
RSYNC_FLAGS ?= -az --progress --stats
SSH_TTY_OPTS ?= -tt -o ServerAliveInterval=30 -o ServerAliveCountMax=3 -o ControlMaster=auto -o ControlPersist=10m -o ControlPath=$(SSH_CONTROL_PATH)
TALOS_SSH_COMMAND ?= $(SSH) $(SSH_OPTS)

RSYNC_EXCLUDES := \
	--exclude='.git/' \
	--exclude='.DS_Store' \
	--exclude='.vscode/' \
	--exclude='target/' \
	--exclude='logs/' \
	--exclude='data/dtu_wind_turbine/' \
	--exclude='*.tmp' \
	--exclude='*.log'

EDGE_ARGS ?= --demo-dtu data/dtu_wind_turbine --max-tasks 10 --telemetry tegrastats --workload alternating --log-jsonl logs/jetson-edge.jsonl
PHASE6_ARGS ?= --mode phase6-contention --tasks 60 --log-jsonl logs/sitl-phase6-contention.jsonl --no-csv
PHASE8_ARGS ?= --mode phase8-optimization --tasks 120 --log-jsonl logs/sitl-phase8-optimization.jsonl --no-csv
HITL_ARGS ?= --tasks 60 --telemetry sysfs --log-jsonl logs/hitl-orinnano-baseline.jsonl --no-csv
HITL_HEAVY_ARGS ?= --workload heavy --tasks 10000 --duration-secs 60 --progress-every 5 --telemetry sysfs --sample-ms 20 --inter-task-ms 0 --payload-bytes 16777216 --log-jsonl logs/hitl-orinnano-heavy-60s.jsonl --no-csv
THERMAL_SOAK_SECONDS ?= 180
THERMAL_SOAK_WORKERS ?= auto
THERMAL_SOAK_ARGS ?= --workload thermal --tasks 100000 --duration-secs $(THERMAL_SOAK_SECONDS) --progress-every 1 --cpu-burn-threads $(THERMAL_SOAK_WORKERS) --target-temp-c 70 --stop-temp-c 78 --telemetry sysfs --sample-ms 50 --inter-task-ms 0 --payload-bytes 1048576 --log-jsonl logs/hitl-thermal-soak.jsonl --no-csv
RESOURCE_MAX_SECONDS ?= 120
RESOURCE_MAX_MEMORY_MB ?= 4608
RESOURCE_MAX_ARGS ?= --workload thermal --tasks 100000 --duration-secs $(RESOURCE_MAX_SECONDS) --progress-every 1 --cpu-burn-threads auto --target-temp-c 70 --stop-temp-c 78 --memory-pressure-mb $(RESOURCE_MAX_MEMORY_MB) --telemetry sysfs --sample-ms 50 --inter-task-ms 0 --payload-bytes 1048576 --log-jsonl logs/hitl-resource-max.jsonl --no-csv
CUDA_BURN_SECONDS ?= 180
CUDA_BURN_BLOCKS ?= 512
CUDA_BURN_THREADS ?= 256
CUDA_BURN_ITERATIONS ?= 40000
GPU_RESOURCE_ARGS ?= --workload thermal --tasks 100000 --duration-secs $(CUDA_BURN_SECONDS) --progress-every 1 --cpu-burn-threads auto --target-temp-c 70 --stop-temp-c 82 --memory-pressure-mb 4096 --vlm-temperature-gate-c 70 --telemetry sysfs --sample-ms 50 --inter-task-ms 0 --payload-bytes 1048576 --log-jsonl logs/hitl-gpu-resource-max.jsonl --no-csv
JETSON_HARDEN_ARGS ?= --mode 0
REAL_MODEL_ARGS ?= --backend tensorrt-engine --model models/vision.engine --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-real-model.jsonl --no-csv
TRT_ONNX_ARGS ?= --backend tensorrt-onnx --model models/vision.onnx --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-trt-onnx.jsonl --no-csv
SMOLVLM_ARGS ?= --backend smolvlm-cuda --model HuggingFaceTB/SmolVLM-256M-Instruct --tasks 1 --telemetry tegrastats --log-jsonl logs/hitl-smolvlm-cuda.jsonl --no-csv
TINY_VISION_ONNX ?= models/vision.onnx
TINY_VISION_ARGS ?= --backend tensorrt-onnx --model $(TINY_VISION_ONNX) --backend-arg --fp16 --tasks 3 --telemetry tegrastats --log-jsonl logs/hitl-tiny-vision-trt.jsonl --no-csv

.PHONY: help
help: ## Show available targets.
	@awk 'BEGIN {FS = ":.*##"; printf "TALOS automation\n\n"} /^[a-zA-Z0-9_.-]+:.*##/ {printf "  %-22s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

.PHONY: check
check: ## Run local cargo check with an external target dir.
	CARGO_TARGET_DIR=$(LOCAL_TARGET_DIR) $(CARGO) check --all-targets

.PHONY: test
test: ## Run local tests with an external target dir.
	CARGO_TARGET_DIR=$(LOCAL_TARGET_DIR) $(CARGO) test

.PHONY: bench-phase6
bench-phase6: ## Run local Phase 6 contention simulation.
	mkdir -p logs
	CARGO_TARGET_DIR=$(LOCAL_TARGET_DIR) $(CARGO) run --bin talos_bench -- $(PHASE6_ARGS)

.PHONY: bench-phase8
bench-phase8: ## Run local Phase 8 optimization benchmark.
	mkdir -p logs
	CARGO_TARGET_DIR=$(LOCAL_TARGET_DIR) $(CARGO) run --bin talos_bench -- $(PHASE8_ARGS)

.PHONY: hitl-baseline
hitl-baseline: ## Run a local HITL baseline if this system exposes compatible /sys telemetry.
	mkdir -p logs
	CARGO_TARGET_DIR=$(LOCAL_TARGET_DIR) $(CARGO) run --bin talos_hitl -- $(HITL_ARGS)

.PHONY: hitl-heavy
hitl-heavy: ## Run a local heavy HITL workload if this system exposes compatible /sys telemetry.
	mkdir -p logs
	CARGO_TARGET_DIR=$(LOCAL_TARGET_DIR) $(CARGO) run --bin talos_hitl -- $(HITL_HEAVY_ARGS)

.PHONY: real-model
real-model: ## Run local TALOS control plane around a real external model backend.
	mkdir -p logs
	CARGO_TARGET_DIR=$(LOCAL_TARGET_DIR) $(CARGO) run --bin talos_real_model -- $(REAL_MODEL_ARGS)

.PHONY: model-tiny-vision
model-tiny-vision: ## Generate a small static ONNX vision model locally. Requires python onnx + numpy.
	$(PYTHON) scripts/create_tiny_vision_onnx.py --output $(TINY_VISION_ONNX)

.PHONY: report
report: ## Generate README metrics, report, and SVG assets.
	python3 scripts/generate_readme_assets.py

.PHONY: jetson-ping
jetson-ping: ## Verify SSH connectivity to the Jetson.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'hostname && uname -m && cat /etc/nv_tegra_release 2>/dev/null || true'

.PHONY: jetson-ssh-start
jetson-ssh-start: ## Start one persistent SSH master connection for subsequent Jetson targets.
	$(SSH) $(SSH_OPTS) -O check $(JETSON_HOST) >/dev/null 2>&1 || $(SSH) $(SSH_OPTS) -MNf $(JETSON_HOST)

.PHONY: jetson-ssh-stop
jetson-ssh-stop: ## Stop the persistent SSH master connection.
	$(SSH) $(SSH_OPTS) -O exit $(JETSON_HOST) || true

.PHONY: jetson-deps-check
jetson-deps-check: ## Check required tools on the Jetson without installing anything.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set +e; \
		missing=0; \
		check_tool() { \
			if command -v "$$1" >/dev/null 2>&1; then \
				printf "%-12s %s\n" "$$1" "$$(command -v "$$1")"; \
				shift; \
				if [ "$$#" -gt 0 ]; then "$$@" || true; fi; \
			else \
				echo "MISSING      $$1"; \
				missing=1; \
			fi; \
		}; \
		echo "host=$$(hostname)"; \
		echo "arch=$$(uname -m)"; \
		check_tool rustc rustc --version; \
		check_tool cargo cargo --version; \
		check_tool g++ sh -c "g++ --version | head -n 1"; \
		check_tool curl curl --version; \
		check_tool tegrastats; \
		check_tool python3 python3 --version; \
		exit $$missing'

.PHONY: jetson-network-check
jetson-network-check: ## Check Jetson internet access needed by rustup.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		echo "addresses=$$(hostname -I 2>/dev/null || true)"; \
		ip route 2>/dev/null || true; \
		command -v curl >/dev/null 2>&1 || { echo "MISSING curl"; exit 1; }; \
		curl -I --max-time 15 https://sh.rustup.rs >/dev/null; \
		echo "rustup endpoint reachable"'

.PHONY: jetson-install-deps
jetson-install-deps: ## Install Jetson build dependencies and Rust toolchain.
	$(SSH) $(SSH_TTY_OPTS) $(JETSON_HOST) 'set -e; \
		sudo apt-get update; \
		sudo apt-get install -y build-essential curl ca-certificates pkg-config python3; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		if ! command -v cargo >/dev/null 2>&1; then \
			curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; \
		fi; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		rustup default stable; \
		rustc --version; \
		cargo --version; \
		g++ --version | head -n 1'

.PHONY: jetson-install-rust
jetson-install-rust: ## Install only Rust/Cargo on the Jetson using rustup. No sudo.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		command -v curl >/dev/null 2>&1 || { echo "curl is missing; run make jetson-install-deps instead"; exit 1; }; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		if ! command -v cargo >/dev/null 2>&1; then \
			curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; \
		fi; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		rustup default stable; \
		rustc --version; \
		cargo --version'

.PHONY: jetson-prepare
jetson-prepare: ## Create the remote project and log directories.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'mkdir -p $(JETSON_DIR) $(JETSON_DIR)/logs'

.PHONY: jetson-status
jetson-status: ## Show remote repo/build/telemetry status.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		echo "dir=$(JETSON_DIR)"; \
		if [ -d "$(JETSON_DIR)/.git" ]; then git -C "$(JETSON_DIR)" status --short; else echo "no git repo in $(JETSON_DIR)"; fi; \
		df -h "$(JETSON_DIR)" 2>/dev/null || df -h "$$HOME"; \
		if command -v tegrastats >/dev/null 2>&1; then \
			tegrastats --interval 1000 & pid=$$!; \
			sleep 2; \
			kill $$pid 2>/dev/null || true; \
			wait $$pid 2>/dev/null || true; \
		else \
			echo "tegrastats not found"; \
		fi'

.PHONY: jetson-sync
jetson-sync: ## Safely copy local files to the Jetson. Does not delete remote-only files.
	JETSON_HOST=$(JETSON_HOST) JETSON_REPO=$(JETSON_DIR) TALOS_SSH_COMMAND='$(TALOS_SSH_COMMAND)' ./scripts/sync_jetson_talos.sh

.PHONY: jetson-sync-data
jetson-sync-data: jetson-prepare ## Copy the local DTU dataset to the Jetson. Optional and heavier.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'mkdir -p $(JETSON_DIR)/data'
	$(RSYNC) $(RSYNC_FLAGS) --exclude='.DS_Store' data/dataset.md $(JETSON_HOST):$(JETSON_DIR)/data/
	$(RSYNC) $(RSYNC_FLAGS) --exclude='.DS_Store' data/dtu_wind_turbine/ $(JETSON_HOST):$(JETSON_DIR)/data/dtu_wind_turbine/

.PHONY: jetson-backup
jetson-backup: ## Archive the current remote project before destructive cleanup.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		if [ -d "$(JETSON_DIR)" ]; then \
			mkdir -p "$$HOME/talos-backups"; \
			tar -czf "$$HOME/talos-backups/talos-$$(date +%Y%m%d-%H%M%S).tgz" -C "$$(dirname "$(JETSON_DIR)")" "$$(basename "$(JETSON_DIR)")"; \
			ls -lh "$$HOME/talos-backups" | tail -n 5; \
		else \
			echo "nothing to back up: $(JETSON_DIR) does not exist"; \
		fi'

.PHONY: jetson-clean
jetson-clean: ## Delete remote project contents. Requires CONFIRM=1.
	@test "$(CONFIRM)" = "1" || (echo 'Refusing to clean remote project. Re-run with: make jetson-clean CONFIRM=1'; exit 2)
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -euo pipefail; \
		test -n "$(JETSON_DIR)"; \
		test "$(JETSON_DIR)" != "/"; \
		mkdir -p "$(JETSON_DIR)"; \
		find "$(JETSON_DIR)" -mindepth 1 -maxdepth 1 -exec rm -rf {} +; \
		echo "cleaned $(JETSON_DIR)"'

.PHONY: jetson-sync-clean
jetson-sync-clean: jetson-clean jetson-sync ## Clean remote project, then sync. Requires CONFIRM=1.

.PHONY: jetson-backup-old-repo
jetson-backup-old-repo: ## Archive the existing Edge-VLA-Micro repo on the Jetson.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		if [ -d "$(JETSON_OLD_REPO)" ]; then \
			mkdir -p "$$HOME/talos-backups"; \
			tar -czf "$$HOME/talos-backups/Edge-VLA-Micro-$$(date +%Y%m%d-%H%M%S).tgz" -C "$$(dirname "$(JETSON_OLD_REPO)")" "$$(basename "$(JETSON_OLD_REPO)")"; \
			ls -lh "$$HOME/talos-backups" | tail -n 5; \
		else \
			echo "old repo not found: $(JETSON_OLD_REPO)"; \
		fi'

.PHONY: jetson-clean-old-repo
jetson-clean-old-repo: ## Delete the existing Edge-VLA-Micro repo. Requires CONFIRM=1.
	@test "$(CONFIRM)" = "1" || (echo 'Refusing to delete old repo. Re-run with: make jetson-clean-old-repo CONFIRM=1'; exit 2)
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -euo pipefail; \
		test -n "$(JETSON_OLD_REPO)"; \
		test "$(JETSON_OLD_REPO)" != "/"; \
		test "$(JETSON_OLD_REPO)" != "$$HOME"; \
		if [ -d "$(JETSON_OLD_REPO)" ]; then rm -rf "$(JETSON_OLD_REPO)"; fi; \
		echo "removed $(JETSON_OLD_REPO)"'

.PHONY: jetson-replace-old-repo
jetson-replace-old-repo: jetson-backup-old-repo jetson-clean-old-repo jetson-sync ## Archive old Edge-VLA-Micro, remove it, then sync TALOS. Requires CONFIRM=1.

.PHONY: jetson-build
jetson-build: jetson-sync ## Build TALOS on the Jetson.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo build --bins'

.PHONY: jetson-bootstrap
jetson-bootstrap: jetson-ssh-start jetson-sync ## Sync once, install Rust if needed, then check dependencies.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		command -v curl >/dev/null 2>&1 || { echo "curl missing: run make jetson-install-deps"; exit 1; }; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		if ! command -v cargo >/dev/null 2>&1; then \
			curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; \
		fi; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		rustup default stable; \
		rustc --version; \
		cargo --version'

.PHONY: jetson-update
jetson-update: jetson-ssh-start jetson-sync ## One-shot Jetson sync, Rust bootstrap, tests, hardening status, Phase 6, and Phase 8.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		command -v curl >/dev/null 2>&1 || { echo "curl missing: run make jetson-install-deps"; exit 1; }; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		if ! command -v cargo >/dev/null 2>&1; then \
			curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; \
		fi; \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		command -v cargo >/dev/null 2>&1 || { echo "cargo not found after rustup install"; exit 127; }; \
		cd $(JETSON_DIR); \
		mkdir -p logs; \
		rustc --version; \
		cargo --version; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo test; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin jetson_harden -- --status; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_bench -- $(PHASE6_ARGS); \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_bench -- $(PHASE8_ARGS)'

.PHONY: jetson-validate
jetson-validate: jetson-ssh-start jetson-sync ## Run Jetson test, hardening status, Phase 6, and Phase 8 after one sync.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		cd $(JETSON_DIR); \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		command -v cargo >/dev/null 2>&1 || { echo "cargo not found. Run: make jetson-bootstrap"; exit 127; }; \
		mkdir -p logs; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo test; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin jetson_harden -- --status; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_bench -- $(PHASE6_ARGS); \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_bench -- $(PHASE8_ARGS)'

.PHONY: jetson-harden-plan
jetson-harden-plan: jetson-sync ## Print Jetson power/clocks hardening plan. Does not apply changes.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin jetson_harden -- $(JETSON_HARDEN_ARGS)'

.PHONY: jetson-harden-status
jetson-harden-status: jetson-sync ## Run non-mutating Jetson power/clocks/thermal status probes.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin jetson_harden -- --status'

.PHONY: jetson-harden-apply
jetson-harden-apply: jetson-sync ## Apply Jetson nvpmodel/clocks hardening. Requires sudo on Jetson.
	$(SSH) $(SSH_TTY_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin jetson_harden -- --apply $(JETSON_HARDEN_ARGS)'

.PHONY: jetson-harden-restore
jetson-harden-restore: jetson-sync ## Restore Jetson clocks after benchmarking. Requires sudo on Jetson.
	$(SSH) $(SSH_TTY_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin jetson_harden -- --restore-clocks --apply'

.PHONY: jetson-test
jetson-test: jetson-sync ## Run tests on the Jetson.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo test'

.PHONY: jetson-run-edge
jetson-run-edge: jetson-sync ## Run edge_node on the Jetson. Override with EDGE_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && mkdir -p logs && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin edge_node -- $(EDGE_ARGS)'

.PHONY: jetson-run-phase6
jetson-run-phase6: jetson-sync ## Run Phase 6 contention benchmark on the Jetson. Override with PHASE6_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && mkdir -p logs && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_bench -- $(PHASE6_ARGS)'

.PHONY: jetson-run-phase8
jetson-run-phase8: jetson-sync ## Run Phase 8 optimization benchmark on the Jetson. Override with PHASE8_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && mkdir -p logs && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_bench -- $(PHASE8_ARGS)'

.PHONY: jetson-run-hitl
jetson-run-hitl: jetson-sync ## Run HITL baseline with real Jetson telemetry. Override with HITL_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && mkdir -p logs && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_hitl -- $(HITL_ARGS)'

.PHONY: jetson-run-hitl-heavy
jetson-run-hitl-heavy: jetson-sync ## Run heavy HITL workload with real Jetson telemetry. Override with HITL_HEAVY_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && mkdir -p logs && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_hitl -- $(HITL_HEAVY_ARGS)'

.PHONY: jetson-run-thermal-soak
jetson-run-thermal-soak: jetson-sync ## Aggressive Jetson thermal soak: TALOS thermal workload plus internal CPU burners and live tegrastats.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		cd $(JETSON_DIR); \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		mkdir -p logs; \
		echo "thermal_soak_seconds=$(THERMAL_SOAK_SECONDS)"; \
		cleanup() { \
			set +e; \
			if [ -n "$${tegrastats_pid:-}" ]; then kill "$$tegrastats_pid" 2>/dev/null || true; wait "$$tegrastats_pid" 2>/dev/null || true; fi; \
		}; \
		trap cleanup EXIT INT TERM; \
		(tegrastats --interval 1000 2>/dev/null | tee logs/hitl-thermal-soak-tegrastats.log) & tegrastats_pid=$$!; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_hitl -- $(THERMAL_SOAK_ARGS)'

.PHONY: jetson-run-thermal-max
jetson-run-thermal-max: jetson-harden-apply jetson-run-thermal-soak ## Apply max clocks/power, then run thermal soak. Requires sudo on Jetson.

.PHONY: jetson-run-resource-max
jetson-run-resource-max: jetson-sync ## Aggressive HITL pressure: CPU burners plus guarded RAM pressure to trigger real admission gating.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		cd $(JETSON_DIR); \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		mkdir -p logs; \
		cleanup() { \
			set +e; \
			if [ -n "$${tegrastats_pid:-}" ]; then kill "$$tegrastats_pid" 2>/dev/null || true; wait "$$tegrastats_pid" 2>/dev/null || true; fi; \
		}; \
		trap cleanup EXIT INT TERM; \
		(tegrastats --interval 1000 2>/dev/null | tee logs/hitl-resource-max-tegrastats.log) & tegrastats_pid=$$!; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_hitl -- $(RESOURCE_MAX_ARGS)'

.PHONY: jetson-build-cuda-burn
jetson-build-cuda-burn: jetson-sync ## Build the local CUDA burn helper on the Jetson. Requires nvcc.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		export PATH="$(CUDA_HOME)/bin:$$PATH"; \
		export LD_LIBRARY_PATH="$(CUDA_HOME)/lib64:$${LD_LIBRARY_PATH:-}"; \
		command -v nvcc >/dev/null 2>&1 || { echo "nvcc missing: expected $(CUDA_HOME)/bin/nvcc or nvcc in PATH"; exit 127; }; \
		cd $(JETSON_DIR); \
		mkdir -p /tmp/talos-tools; \
		nvcc -O3 -std=c++17 tools/cuda_burn.cu -o /tmp/talos-tools/talos_cuda_burn; \
		/tmp/talos-tools/talos_cuda_burn 1 64 128 1000'

.PHONY: jetson-run-cuda-burn
jetson-run-cuda-burn: jetson-build-cuda-burn ## Run standalone CUDA burn with live tegrastats. Override CUDA_BURN_* variables.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		export LD_LIBRARY_PATH="$(CUDA_HOME)/lib64:$${LD_LIBRARY_PATH:-}"; \
		mkdir -p $(JETSON_DIR)/logs; \
		(tegrastats --interval 1000 2>/dev/null | tee $(JETSON_DIR)/logs/hitl-cuda-burn-tegrastats.log) & tegrastats_pid=$$!; \
		cleanup() { kill "$$tegrastats_pid" 2>/dev/null || true; wait "$$tegrastats_pid" 2>/dev/null || true; }; \
		trap cleanup EXIT INT TERM; \
		/tmp/talos-tools/talos_cuda_burn $(CUDA_BURN_SECONDS) $(CUDA_BURN_BLOCKS) $(CUDA_BURN_THREADS) $(CUDA_BURN_ITERATIONS)'

.PHONY: jetson-run-gpu-resource-max
jetson-run-gpu-resource-max: jetson-build-cuda-burn ## Run CUDA burn concurrently with TALOS resource pressure and real telemetry.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		export LD_LIBRARY_PATH="$(CUDA_HOME)/lib64:$${LD_LIBRARY_PATH:-}"; \
		cd $(JETSON_DIR); \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		mkdir -p logs; \
		cleanup() { \
			set +e; \
			if [ -n "$${tegrastats_pid:-}" ]; then kill "$$tegrastats_pid" 2>/dev/null || true; wait "$$tegrastats_pid" 2>/dev/null || true; fi; \
			if [ -n "$${cuda_burn_pid:-}" ]; then kill "$$cuda_burn_pid" 2>/dev/null || true; wait "$$cuda_burn_pid" 2>/dev/null || true; fi; \
		}; \
		trap cleanup EXIT INT TERM; \
		(tegrastats --interval 1000 2>/dev/null | tee logs/hitl-gpu-resource-max-tegrastats.log) & tegrastats_pid=$$!; \
		(/tmp/talos-tools/talos_cuda_burn $(CUDA_BURN_SECONDS) $(CUDA_BURN_BLOCKS) $(CUDA_BURN_THREADS) $(CUDA_BURN_ITERATIONS) | tee logs/hitl-cuda-burn.log) & cuda_burn_pid=$$!; \
		CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_hitl -- $(GPU_RESOURCE_ARGS)'

.PHONY: jetson-run-real-model
jetson-run-real-model: jetson-sync ## Run TALOS admission/lease/logging around a real external model backend. Override REAL_MODEL_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && mkdir -p logs tmp && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_real_model -- $(REAL_MODEL_ARGS)'

.PHONY: jetson-run-trt-onnx
jetson-run-trt-onnx: jetson-sync ## Run TALOS around TensorRT trtexec from an ONNX model. Override TRT_ONNX_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		cd $(JETSON_DIR); \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		mkdir -p logs tmp; \
		trtexec_path=""; \
		for candidate in $(TRTEXEC_CANDIDATES); do if command -v "$$candidate" >/dev/null 2>&1; then trtexec_path="$$(command -v "$$candidate")"; break; elif [ -x "$$candidate" ]; then trtexec_path="$$candidate"; break; fi; done; \
		test -n "$$trtexec_path" || { echo "trtexec missing: install TensorRT samples/tools or set TRTEXEC_CANDIDATES"; exit 127; }; \
		echo "trtexec=$$trtexec_path"; \
		TALOS_TRTEXEC="$$trtexec_path" CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_real_model -- $(TRT_ONNX_ARGS)'

.PHONY: jetson-run-smolvlm
jetson-run-smolvlm: jetson-sync ## Run TALOS around a real SmolVLM CUDA inference on Jetson. Override SMOLVLM_ARGS='...'.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'cd $(JETSON_DIR) && if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi && mkdir -p logs tmp && CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_real_model -- $(SMOLVLM_ARGS)'

.PHONY: jetson-setup-tiny-vision-onnx
jetson-setup-tiny-vision-onnx: jetson-ssh-start jetson-sync ## Generate models/vision.onnx on the Jetson. Installs user-level Python deps if needed.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		cd $(JETSON_DIR); \
		mkdir -p models; \
		if ! python3 -c "import onnx, numpy" >/dev/null 2>&1; then \
			python3 -m pip install --user onnx numpy; \
		fi; \
		python3 scripts/create_tiny_vision_onnx.py --output $(TINY_VISION_ONNX); \
		ls -lh $(TINY_VISION_ONNX)'

.PHONY: jetson-run-tiny-vision-trt
jetson-run-tiny-vision-trt: jetson-setup-tiny-vision-onnx ## Generate tiny ONNX on Jetson, then run it through TensorRT inside TALOS.
	$(SSH) $(SSH_OPTS) $(JETSON_HOST) 'set -e; \
		cd $(JETSON_DIR); \
		if [ -f "$$HOME/.cargo/env" ]; then . "$$HOME/.cargo/env"; fi; \
		mkdir -p logs tmp; \
		trtexec_path=""; \
		for candidate in $(TRTEXEC_CANDIDATES); do if command -v "$$candidate" >/dev/null 2>&1; then trtexec_path="$$(command -v "$$candidate")"; break; elif [ -x "$$candidate" ]; then trtexec_path="$$candidate"; break; fi; done; \
		test -n "$$trtexec_path" || { echo "trtexec missing: install TensorRT samples/tools or set TRTEXEC_CANDIDATES"; exit 127; }; \
		echo "trtexec=$$trtexec_path"; \
		TALOS_TRTEXEC="$$trtexec_path" CARGO_TARGET_DIR=$(JETSON_TARGET_DIR) cargo run --bin talos_real_model -- $(TINY_VISION_ARGS)'

.PHONY: jetson-logs
jetson-logs: ## Pull Jetson logs into logs/jetson/.
	JETSON_HOST=$(JETSON_HOST) JETSON_REPO=$(JETSON_DIR) TALOS_SSH_COMMAND='$(TALOS_SSH_COMMAND)' ./scripts/pull_jetson_logs.sh
