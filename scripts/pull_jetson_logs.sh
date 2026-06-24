#!/usr/bin/env bash
set -euo pipefail

JETSON_HOST="${JETSON_HOST:-ste@192.168.55.1}"
JETSON_REPO="${JETSON_REPO:-~/TALOS}"
LOCAL_LOG_DIR="${LOCAL_LOG_DIR:-logs/jetson}"
TALOS_SSH_COMMAND="${TALOS_SSH_COMMAND:-ssh}"

mkdir -p "${LOCAL_LOG_DIR}"

rsync -avh \
  -e "${TALOS_SSH_COMMAND}" \
  "${JETSON_HOST}:${JETSON_REPO}/logs/" \
  "${LOCAL_LOG_DIR}/"

echo "Pulled Jetson logs into ${LOCAL_LOG_DIR}/"
