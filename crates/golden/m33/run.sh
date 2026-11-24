#!/bin/sh
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
if [ -n "$M2X_PYTHON" ]; then
  python=$M2X_PYTHON
elif [ -f "$root/.venv/Scripts/python.exe" ]; then
  python="$root/.venv/Scripts/python.exe"
elif [ -f "$root/.venv/bin/python" ]; then
  python="$root/.venv/bin/python"
else
  echo "the M33 parity check drives the Unicorn emulator from Python: set M2X_PYTHON, or create $root/.venv with python -m venv .venv and .venv/Scripts/pip install unicorn==2.1.4" >&2
  exit 2
fi
profile=${1:-release}
tools="$(rustc --print sysroot)/lib/rustlib/x86_64-pc-windows-msvc/bin"
case "$profile" in
  release) build=release ;;
  dev) build=debug ;;
  *) echo "profile is release or dev" >&2; exit 2 ;;
esac
cd "$here"
RUSTFLAGS="-C target-cpu=cortex-m33 -C target-feature=-fpregs" cargo build --profile "$profile" --target thumbv8m.main-none-eabi
elf="target/golden-m33-$profile.elf"
"$tools/rust-lld" -flavor gnu -T link.x --gc-sections -u golden_ticks -u golden_bands -u golden_grid \
  "target/thumbv8m.main-none-eabi/$build/libgolden_m33.a" -o "$elf"
"$tools/llvm-nm" -C -S -n --defined-only "$elf" > "$elf.syms"
"$python" -I run.py "$elf" "$elf.syms" ../vectors/golden.txt
