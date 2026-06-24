#!/usr/bin/env bash
set -euo pipefail

JETSON_HOST="${JETSON_HOST:-ste@192.168.55.1}"
JETSON_REPO="${JETSON_REPO:-~/TALOS}"

FILES=(
  .gitignore
  Cargo.lock
  Cargo.toml
  Makefile
  README.md
  blueprint.md
  build.rs
  configs
  core
  edge_node
  evaluation
  ipc
  runtime
)

for file in "${FILES[@]}"; do
  if [[ ! -e "${file}" ]]; then
    echo "missing local path: ${file}" >&2
    exit 1
  fi
done

ssh "${JETSON_HOST}" "mkdir -p ${JETSON_REPO}/logs"

rsync -avhR \
  --exclude '.DS_Store' \
  --exclude 'target/' \
  --exclude 'logs/' \
  --exclude '*.tmp' \
  --exclude '*.log' \
  "${FILES[@]}" \
  "${JETSON_HOST}:${JETSON_REPO}/"

echo "Synced TALOS to ${JETSON_HOST}:${JETSON_REPO}"
