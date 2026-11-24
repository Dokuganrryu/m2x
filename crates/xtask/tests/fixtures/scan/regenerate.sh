#!/bin/sh
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
source_file="$here/image.rs"
out="$here"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
bin="$(rustc --print sysroot)/lib/rustlib/x86_64-pc-windows-msvc/bin"
mkdir -p "$out" "$work"
build() {
  name="$1"; level="$2"; shift 2
  rustc --edition 2024 --crate-name image --target thumbv8m.main-none-eabihf -C target-cpu=cortex-m33 \
    -C opt-level="$level" -C panic=abort --crate-type bin \
    -C link-arg=--entry=Reset -C link-arg=--undefined=CTIMER0 -C link-arg=--undefined=USB1 \
    -C link-arg=-Ttext=0x20000000 -C link-arg=--gc-sections "$@" -o "$work/$name.elf" "$source_file"
  (cd "$work" && "$bin/llvm-objdump" -d --demangle --mcpu=cortex-m33 $name.elf) > "$out/$name.objdump.txt"
  "$bin/llvm-nm" --demangle "$work/$name.elf" > "$out/$name.nm.txt"
}
build clean-speed 3
build clean-size z
build flash-controller-interleaved 3 --cfg 'plant="flash_controller_interleaved"'
(cd "$work" && "$bin/llvm-objdump" -d --mcpu=cortex-m33 clean-speed.elf) > "$out/clean-speed-mangled.objdump.txt"
"$bin/llvm-nm" "$work/clean-speed.elf" > "$out/clean-speed-mangled.nm.txt"
build vfp-without-cpu 3 --cfg 'plant="vfp_in_tick"'
(cd "$work" && "$bin/llvm-objdump" -d --demangle vfp-without-cpu.elf) > "$out/vfp-without-cpu.objdump.txt"
build flash-controller-literal-pool z --cfg 'plant="flash_controller"'
build flash-controller-movw-movt 3 --cfg 'plant="flash_controller"'
build flash-controller-secure-alias 3 --cfg 'plant="flash_controller_secure_alias"'
build rom-flash-api-entry z --cfg 'plant="rom_flash_api_entry"'
build rom-api-tree-literal-pool z --cfg 'plant="rom_api_tree"'
build rom-api-tree-movw-movt 3 --cfg 'plant="rom_api_tree"'
build rom-api-tree-non-secure-alias 3 --cfg 'plant="rom_api_tree_non_secure_alias"'
build rom-flash-erase-entry-version-0 z --cfg 'plant="rom_flash_erase_entry_version_0"'
build rom-flash-erase-entry-non-secure-alias z --cfg 'plant="rom_flash_erase_entry_non_secure_alias"'
build factory-record-outside-accessor 3 --cfg 'plant="factory_record_outside_accessor"'
build factory-magic-outside-accessor z --cfg 'plant="factory_magic_outside_accessor"'
build factory-write-in-accessor 3 --cfg 'plant="factory_write_in_accessor"'
build factory-write-in-accessor-literal-pool z --cfg 'plant="factory_write_in_accessor"'
build vfp-outside-interrupts 3 --cfg 'plant="vfp_outside_interrupts"'
build udiv-in-tick 3 --cfg 'plant="udiv_in_tick"'
build aeabi-div-in-tick 3 --cfg 'plant="aeabi_div_in_tick"'
build vfp-in-tick 3 --cfg 'plant="vfp_in_tick"'
build panic-in-tick z --cfg 'plant="panic_in_tick"'
build fmt-in-tick z --cfg 'plant="fmt_in_tick"'
build masking-in-tick 3 --cfg 'plant="masking_in_tick"'
build critical-section-in-tick 3 --cfg 'plant="critical_section_in_tick"'
build libm-in-tick 3 --cfg 'plant="libm_in_tick"'
build indirect-call-in-tick 3 --cfg 'plant="indirect_call_in_tick"'
build flash-driver-symbol 3 --cfg 'plant="flash_driver_symbol"'
build dead-stripped-symbol-kept 3 --cfg 'plant="dead_stripped_symbol_kept"'
build dac-register-zero-writer 3 --cfg 'plant="dac_register_zero_writer"'
