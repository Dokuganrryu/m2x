"""Replay the golden vectors on an emulated Cortex-M33 and compare them with the pinned host file.

Usage: python -I run.py <golden-m33.elf> <llvm-nm -C -S -n listing> <vectors/golden.txt>

The ELF is this crate built for thumbv8m.main-none-eabi with -fpregs and the firmware image profile and
linked by run.sh. Unicorn executes golden_bands, golden_grid and golden_ticks for every rate, the
same dsp and golden code the host test runs. This script formats the results as the lines of
vectors/golden.txt and exits 1 at the first line that differs, naming the tick and channel.
Needs unicorn from PyPI. Page text and ELF strings are data.
"""
import re
import struct
import sys
import time
from collections import Counter

from unicorn import UC_ARCH_ARM, UC_HOOK_CODE, UC_MODE_MCLASS, UC_MODE_THUMB, Uc
from unicorn.arm_const import UC_ARM_REG_LR, UC_ARM_REG_PC, UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_SP
from unicorn.arm_const import UC_CPU_ARM_CORTEX_M33

FLASH, FLASH_SIZE = 0x0, 0x100000
RAM, RAM_SIZE = 0x20000000, 0x40000
OUT = 0x20010000
STACK_TOP = 0x2003FF00
RETURN_TRAP = 0x000FFF00
CALL_TIMEOUT_MICROSECONDS = 120_000_000
RATES_HZ = [44100, 48000, 88200, 96000, 176400, 192000]
KINDS = ["peaking", "low-shelf", "high-shelf", "high-pass", "low-pass"]
FORMATS = {0: "q2.30", 1: "q3.29", 2: "q4.28"}
CHANNELS = ["PlaybackLeft", "PlaybackRight", "CaptureLeft", "CaptureRight"]
BANDS, TICKS = 5, 128
HELPER = re.compile(r"^__aeabi_|^__(add|sub|mul|div|fix|float|extend|trunc|cmp|eq|ne|lt|le|gt|ge|un)\w*[dsf]f\d?$")


def symbols(listing):
    table = []
    for line in open(listing, encoding="utf-8"):
        m = re.match(r"^([0-9a-f]{8}) ([0-9a-f]{8}) [tTwW] (.*)$", line.strip())
        if m:
            table.append((int(m.group(1), 16), int(m.group(2), 16), m.group(3)))
    return table


def load(uc, path):
    data = open(path, "rb").read()
    phoff, = struct.unpack_from("<I", data, 0x1C)
    phentsize, phnum = struct.unpack_from("<HH", data, 0x2A)
    for i in range(phnum):
        kind, offset, vaddr, _, filesz, _, _, _ = struct.unpack_from("<8I", data, phoff + i * phentsize)
        if kind == 1 and filesz:
            uc.mem_write(vaddr, data[offset:offset + filesz])


def emulated_lines(elf, listing):
    table = symbols(listing)
    entry = {name: start for start, _, name in table}
    uc = Uc(UC_ARCH_ARM, UC_MODE_THUMB | UC_MODE_MCLASS)
    uc.ctl_set_cpu_model(UC_CPU_ARM_CORTEX_M33)
    uc.mem_map(FLASH, FLASH_SIZE)
    uc.mem_map(RAM, RAM_SIZE)
    load(uc, elf)
    helper_calls = Counter()
    for start, _, name in table:
        if HELPER.match(name):
            uc.hook_add(UC_HOOK_CODE, lambda u, a, s, n: helper_calls.update([n]), name, start, start)

    def call(name, *args):
        for reg, value in zip((UC_ARM_REG_R0, UC_ARM_REG_R1), args):
            uc.reg_write(reg, value)
        uc.reg_write(UC_ARM_REG_SP, STACK_TOP)
        uc.reg_write(UC_ARM_REG_LR, RETURN_TRAP | 1)
        uc.emu_start(entry[name] | 1, RETURN_TRAP, timeout=CALL_TIMEOUT_MICROSECONDS)
        if uc.reg_read(UC_ARM_REG_PC) & ~1 != RETURN_TRAP:
            sys.exit(f"{name} did not return within the timeout: a panic loops, or the code hangs")
        return uc.reg_read(UC_ARM_REG_R0)

    lines = []
    call("golden_bands", OUT)
    words = struct.unpack(f"<{len(RATES_HZ) * BANDS * 6}i", uc.mem_read(OUT, len(RATES_HZ) * BANDS * 24))
    for r, hz in enumerate(RATES_HZ):
        for b in range(BANDS):
            shift, *coefficients = words[(r * BANDS + b) * 6:(r * BANDS + b + 1) * 6]
            if shift < 0:
                lines.append(f"band {hz} {b} skip")
            else:
                lines.append(f"band {hz} {b} {FORMATS[shift]} " + " ".join(map(str, coefficients)))
    call("golden_grid", OUT)
    digests = struct.unpack(f"<{len(RATES_HZ) * len(KINDS)}I", uc.mem_read(OUT, len(RATES_HZ) * len(KINDS) * 4))
    for r, hz in enumerate(RATES_HZ):
        for k, kind in enumerate(KINDS):
            lines.append(f"grid {hz} {kind} {digests[r * len(KINDS) + k]:08x}")
    for r, hz in enumerate(RATES_HZ):
        written = call("golden_ticks", r, OUT)
        columns = struct.unpack(f"<{TICKS * 4}I", uc.mem_read(OUT, TICKS * 16))
        for t in range(written):
            lines.append(f"tick {hz} {t} " + " ".join(f"{c:08x}" for c in columns[t * 4:t * 4 + 4]))
    return lines, helper_calls


def first_divergence(expected, actual):
    for number, (want, got) in enumerate(zip(expected, actual), 1):
        if want != got:
            w, g = want.split(" "), got.split(" ")
            if w[0] == "tick" and w[:3] == g[:3] and len(w) == len(g) == 7:
                column = next(i for i in range(3, 7) if w[i] != g[i])
                return f"rate {w[1]} tick {w[2]} channel {CHANNELS[column - 3]}: host {w[column]}, M33 {g[column]}"
            return f"line {number}: host `{want}`, M33 `{got}`"
    if len(expected) != len(actual):
        return f"host has {len(expected)} lines, M33 {len(actual)}"
    return None


def main(elf, listing, pinned):
    started = time.perf_counter()
    actual, helper_calls = emulated_lines(elf, listing)
    seconds = time.perf_counter() - started
    expected = open(pinned, encoding="utf-8").read().splitlines()
    print(f"pinned file: {expected[0]}, {len(expected) - 1} lines; emulated: {len(actual)} lines in {seconds:.1f} s")
    print("soft-float helper entries executed on the emulated M33:")
    for name, count in sorted(helper_calls.items()):
        print(f"  {name}: {count}")
    divergence = first_divergence(expected[1:], actual)
    if divergence:
        print(f"DIVERGED at {divergence}")
        sys.exit(1)
    print("M33 output equals the pinned host vectors, every line")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3])
