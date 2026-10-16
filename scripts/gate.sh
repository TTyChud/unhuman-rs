#!/usr/bin/env bash
# The gate. One script, so a green run locally means a green run in CI.
#
# Usage:
#   scripts/gate.sh              # full gate: build, tests, validation, clippy
#   scripts/gate.sh --no-vulkan  # skip the hardware tests (no driver present)
#   scripts/gate.sh --serial     # one test at a time, for a driver that is flaky
#
# `docs/WORK.md` documents the rules this encodes. Two of them exist because
# getting them wrong produces a green run that means nothing:
#
#   UHE_VALIDATION=1 only enables validation through DeviceOptions::from_env.
#   DeviceOptions::new defaults it OFF, so a suite calling `new` reports zero
#   validation errors having checked nothing at all.
#
#   --test-threads=1. Several suites share one Vulkan device and one scratch
#   directory. In parallel they collide.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

no_vulkan=0
serial=0
for argument in "$@"; do
  case "$argument" in
    --no-vulkan) no_vulkan=1 ;;
    --serial) serial=1 ;;
    *) echo "unknown option: $argument" >&2; exit 2 ;;
  esac
done

THREADS=()
if [[ "$serial" == 1 ]]; then
  THREADS=(--test-threads=1)
fi

step() { printf '\n\033[1m== %s\033[0m\n' "$1"; }

step "build"
cargo build --workspace --all-targets

step "clippy"
# Warnings are errors. `-D warnings` rather than counting them, so a new one
# fails the build instead of being one more line in the log to notice.
cargo clippy --workspace --all-targets -- -D warnings

if [[ "$no_vulkan" == 1 ]]; then
  step "tests (no Vulkan: pure logic only)"
  # Still runs the suites that do not need a device: the job system, handles,
  # the render graph, the shader cache. Those are most of the workspace.
  cargo test -p grind -p chassis -p adam -p forge -- "${THREADS[@]}"
  step "skipped: stormbreaker (needs a Vulkan driver)"
else
  step "tests with validation"
  export UHE_VALIDATION=1
  cargo test --workspace -- "${THREADS[@]}"

  step "validation errors must be zero"
  # Re-run capturing output: a pipeline's exit status does not carry this, and a
  # validation error is printed by the layer rather than returned by cargo.
  count=$(cargo test --workspace -- "${THREADS[@]}" 2>&1 | grep -c "Validation Error" || true)
  if [[ "$count" != "0" ]]; then
    echo "FAIL: $count validation error(s)." >&2
    echo "These are almost always real. Run locally for the full message:" >&2
    echo "  UHE_VALIDATION=1 cargo test --workspace -- --test-threads=1 --nocapture" >&2
    exit 1
  fi
  echo "0 validation errors"
fi

step "gate passed"