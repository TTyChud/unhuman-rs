#!/usr/bin/env bash
# Regenerate the test SPIR-V from Slang source.
#
# The .spv files are checked in so that `cargo test` is hermetic: no toolchain is
# needed to run the suite, and a shader compiler bug cannot turn into a backend
# bug. This script is how they are rebuilt.
#
# slangc is the one already built and vendored in the C++ tree, so there is no
# second compiler in the project and nothing new to keep patched.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
slangc="${SLANGC:-../unhuman/UHE/vendor/bin/linux/slang/bin/slangc}"

if [[ ! -x "$slangc" ]]; then
  echo "slangc not found at $slangc" >&2
  echo "set SLANGC=/path/to/slangc" >&2
  exit 1
fi

# One SPIR-V module per entry point. Vulkan has no linked shader stage, so a
# vertex/fragment pair is two modules, and the backend loads them as a pair.
#
# sm_6_0 is the floor the engine targets; nothing in the test needs more, and
# compiling higher would hide a missing feature rather than report it.
compile() {
  local entry="$1" stage="$2" out="$3"
  "$slangc" \
    -target spirv \
    -profile sm_6_0 \
    -emit-spirv-directly \
    -entry "$entry" \
    -stage "$stage" \
    "$here/Triangle.slang" \
    -o "$here/$out"
  echo "  $out"
}

echo "compiling with $slangc"
compile vs_main    vertex    Triangle.vert.spv
compile fs_main    fragment  Triangle.frag.spv

# A zero-byte or truncated module fails at pipeline creation with a message that
# points at the backend rather than the shader, so check the header here.
for spv in "$here"/*.spv; do
  magic=$(od -A n -t x4 -N 4 "$spv" | tr -d ' ')
  if [[ "$magic" != "07230203" ]]; then
    echo "$(basename "$spv"): bad SPIR-V magic 0x$magic, expected 0x07230203" >&2
    exit 1
  fi
done
echo "ok"