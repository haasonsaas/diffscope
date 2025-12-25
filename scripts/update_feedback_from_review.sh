#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 --action accept|reject --input review.json [--feedback .diffscope.feedback.json]" >&2
  exit 1
}

ACTION=""
INPUT=""
FEEDBACK_PATH=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --action)
      ACTION="$2"
      shift 2
      ;;
    --input)
      INPUT="$2"
      shift 2
      ;;
    --feedback)
      FEEDBACK_PATH="$2"
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done

if [[ -z "${ACTION}" || -z "${INPUT}" ]]; then
  usage
fi

if [[ "${ACTION}" != "accept" && "${ACTION}" != "reject" ]]; then
  echo "Invalid action: ${ACTION}" >&2
  usage
fi

if [[ ! -f "${INPUT}" ]]; then
  echo "Input file not found: ${INPUT}" >&2
  exit 1
fi

CMD=()
if command -v diffscope >/dev/null 2>&1; then
  CMD=(diffscope feedback)
else
  CMD=(cargo run --quiet -- feedback)
fi

if [[ "${ACTION}" == "accept" ]]; then
  CMD+=(--accept "${INPUT}")
else
  CMD+=(--reject "${INPUT}")
fi

if [[ -n "${FEEDBACK_PATH}" ]]; then
  CMD+=(--feedback-path "${FEEDBACK_PATH}")
fi

echo "Running: ${CMD[*]}" >&2
exec "${CMD[@]}"
