#!/usr/bin/env bash
set -euo pipefail

JETSON_HOST="${JETSON_HOST:-ste@192.168.55.1}"
JETSON_REPO="${JETSON_REPO:-~/TALOS}"
TALOS_SSH_COMMAND="${TALOS_SSH_COMMAND:-ssh}"

FILES=(
  .gitignore
  Cargo.lock
  Cargo.toml
  Makefile
  README.md
  build.rs
  configs
  core
  deployment
  docs
  edge_node
  evaluation
  hitl
  ipc
  real_model
  reports
  runtime
  scripts
  tools
)

for file in "${FILES[@]}"; do
  if [[ ! -e "${file}" ]]; then
    echo "missing local path: ${file}" >&2
    exit 1
  fi
done

${TALOS_SSH_COMMAND} "${JETSON_HOST}" "mkdir -p ${JETSON_REPO}/logs"

rsync -avhR \
  -e "${TALOS_SSH_COMMAND}" \
  --exclude '.DS_Store' \
  --exclude 'target/' \
  --exclude 'logs/' \
  --exclude '*.tmp' \
  --exclude '*.log' \
  "${FILES[@]}" \
  "${JETSON_HOST}:${JETSON_REPO}/"

echo "Synced TALOS to ${JETSON_HOST}:${JETSON_REPO}"
